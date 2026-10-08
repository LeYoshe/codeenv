//! Uploads and downloads of files and folders.
//!
//! Uploads come in chunks (Cloudflare caps a request body at 100 MB): each
//! chunk is appended at its offset to a hidden temporary file next to the
//! destination, which is published with the last chunk. Folders are
//! uploaded file by file with their relative path.
//!
//! Downloads stream the file as is, or a folder as a zip built on the fly.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::files::{check_name, http_err, resolve};

/// Largest chunk accepted (the browser sends 32 MB).
pub const MAX_CHUNK: usize = 64 << 20;

#[derive(Deserialize)]
pub struct UploadParams {
    /// Destination folder.
    pub dir: String,
    /// Path of the file relative to `dir` ("a/b/c.txt" for folder uploads).
    pub path: String,
    /// Identifies this upload across its chunks.
    pub id: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Serialize)]
pub struct UploadResult {
    /// Bytes received so far.
    pub size: u64,
    /// Final path, once `done`.
    pub path: Option<String>,
}

/// Creates `parts` under `dir` (like `mkdir -p`), one level at a time,
/// checking that each existing level stays inside the root: create_dir_all
/// would follow a symlink out of the root before we could check it.
pub async fn ensure_dirs(root: &Path, dir: PathBuf, parts: &[&str]) -> Result<PathBuf> {
    let mut parent = dir;
    for part in parts {
        check_name(part)?;
        let next = parent.join(part);
        match tokio::fs::symlink_metadata(&next).await {
            Ok(_) => parent = resolve(root, &next.to_string_lossy())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match tokio::fs::create_dir(&next).await {
                    Ok(()) => {}
                    // Created concurrently by a parallel upload of the same folder.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e).with_context(|| format!("{}", next.display())),
                }
                parent = resolve(root, &next.to_string_lossy())?;
            }
            Err(e) => return Err(e).with_context(|| format!("{}", next.display())),
        }
        if !parent.is_dir() {
            return Err(http_err(
                StatusCode::CONFLICT,
                format!("{} is not a folder", parent.display()),
            ));
        }
    }
    Ok(parent)
}

pub async fn upload_chunk(root: &Path, params: UploadParams, body: Body) -> Result<UploadResult> {
    if params.id.is_empty()
        || params.id.len() > 40
        || !params
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(http_err(StatusCode::BAD_REQUEST, "invalid upload id"));
    }
    let parts: Vec<&str> = params.path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((name, subdirs)) = parts.split_last() else {
        return Err(http_err(StatusCode::BAD_REQUEST, "empty path"));
    };
    for part in &parts {
        check_name(part)?;
    }
    let dir = resolve(root, &params.dir)?;
    if !dir.is_dir() {
        return Err(http_err(
            StatusCode::BAD_REQUEST,
            format!("{} is not a folder", dir.display()),
        ));
    }
    let parent = ensure_dirs(root, dir, subdirs).await?;
    let target = parent.join(name);
    let temp_path = parent.join(format!(".{name}.ce-upload-{}", params.id));

    let mut options = tokio::fs::OpenOptions::new();
    if params.offset == 0 {
        // Never reuse or follow a pre-existing path at the temp name.
        options.write(true).create_new(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let mut file = options
        .open(&temp_path)
        .await
        .with_context(|| format!("{}", temp_path.display()))?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(http_err(StatusCode::BAD_REQUEST, "not a regular file"));
    }
    let previous_size = metadata.len();
    if previous_size != params.offset {
        return Err(http_err(
            StatusCode::CONFLICT,
            format!("upload is at byte {previous_size}, not {}", params.offset),
        ));
    }

    let mut stream = body.into_data_stream();
    let mut size = previous_size;
    let received = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("receiving upload")?;
            if chunk.len() as u64 > MAX_CHUNK as u64 - (size - previous_size) {
                return Err(http_err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "chunk too large (64 MiB max)",
                ));
            }
            file.write_all(&chunk).await?;
            size += chunk.len() as u64;
        }
        file.flush().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = received {
        // A retry starts at the last completed chunk, not at a partial write.
        file.set_len(previous_size)
            .await
            .context("rolling back failed upload chunk")?;
        return Err(error);
    }

    if !params.done {
        return Ok(UploadResult { size, path: None });
    }
    file.sync_all().await?;
    drop(file);
    match tokio::fs::symlink_metadata(&target).await {
        Ok(m) if m.is_dir() => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(http_err(
                StatusCode::CONFLICT,
                format!("{} is a folder", target.display()),
            ));
        }
        Ok(_) if !params.overwrite => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(http_err(
                StatusCode::CONFLICT,
                format!("{} already exists", target.display()),
            ));
        }
        _ => {}
    }
    if params.overwrite {
        tokio::fs::rename(&temp_path, &target)
            .await
            .with_context(|| format!("{}", target.display()))?;
    } else {
        // Publish without replacing a file created after the metadata check.
        // Both paths are in the same directory, so this stays on one filesystem.
        if let Err(error) = tokio::fs::hard_link(&temp_path, &target).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(http_err(StatusCode::CONFLICT, "destination already exists"));
            }
            return Err(error).with_context(|| format!("{}", target.display()));
        }
        tokio::fs::remove_file(&temp_path).await?;
    }
    Ok(UploadResult {
        size,
        path: Some(target.to_string_lossy().into_owned()),
    })
}

/// Removes a partial upload (cancelled from the browser).
pub async fn upload_abort(root: &Path, params: &UploadParams) -> Result<()> {
    let parts: Vec<&str> = params.path.split('/').filter(|s| !s.is_empty()).collect();
    let Some((name, subdirs)) = parts.split_last() else {
        return Ok(());
    };
    for part in &parts {
        check_name(part)?;
    }
    if !params
        .id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Ok(());
    }
    let parent = resolve(
        root,
        &resolve(root, &params.dir)?
            .join(subdirs.join("/"))
            .to_string_lossy(),
    )?;
    let _ = tokio::fs::remove_file(parent.join(format!(".{name}.ce-upload-{}", params.id))).await;
    Ok(())
}

/// Types shown inline by the viewer. Anything else (HTML in particular) is
/// never served inline from the UI's origin.
const INLINE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("ico", "image/x-icon"),
    ("svg", "image/svg+xml"),
    ("pdf", "application/pdf"),
];

/// A file for the image / PDF viewer, displayed inline.
pub async fn raw(root: &Path, path: &str) -> Result<Response> {
    let path = resolve(root, path)?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some((_, mime)) = INLINE_TYPES.iter().find(|(e, _)| *e == extension) else {
        return Err(http_err(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported preview type",
        ));
    };
    let (file, len) = open_file(&path).await?;
    let mut response =
        Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file, CHUNK)).into_response();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    set_disposition(&mut response, "inline", &name, mime);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if extension == "svg" {
        // An SVG opened directly is a document that can run scripts: give it
        // an opaque origin and no script, so it cannot reach the API.
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'",
            ),
        );
    }
    Ok(response)
}

pub async fn download(root: &Path, path: &str) -> Result<Response> {
    let path = resolve(root, path)?;
    let metadata = tokio::fs::metadata(&path).await?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    if metadata.is_dir() {
        let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>(8);
        let dir = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut writer = ChannelWriter {
                tx,
                buf: Vec::with_capacity(CHUNK),
            };
            if let Err(e) = zip_dir(&dir, &mut writer) {
                tracing::warn!("zip {}: {e:#}", dir.display());
                let _ = writer
                    .tx
                    .blocking_send(Err(std::io::Error::other(e.to_string())));
            }
        });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        let mut response = Body::from_stream(stream).into_response();
        set_disposition(
            &mut response,
            "attachment",
            &format!("{name}.zip"),
            "application/zip",
        );
        return Ok(response);
    }
    let (file, len) = open_file(&path).await?;
    let mut response =
        Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file, CHUNK)).into_response();
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    set_disposition(&mut response, "attachment", &name, mime.as_ref());
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    Ok(response)
}

async fn open_file(path: &Path) -> Result<(tokio::fs::File, u64)> {
    let path = path.to_path_buf();
    let (file, len) = tokio::task::spawn_blocking(move || -> Result<_> {
        let file = crate::files::open_regular(&path)?;
        let len = file.metadata()?.len();
        Ok((file, len))
    })
    .await??;
    Ok((tokio::fs::File::from_std(file), len))
}

/// `disposition` is "attachment" (download) or "inline" (shown, but saved
/// under this name, e.g. from the browser's PDF viewer).
fn set_disposition(response: &mut Response, disposition: &str, filename: &str, mime: &str) {
    // RFC 6266 / 5987: ASCII fallback plus the UTF-8 name.
    let ascii: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

const CHUNK: usize = 256 * 1024;

/// Bridges the blocking zip writer to the response body.
struct ChannelWriter {
    tx: mpsc::Sender<std::io::Result<Vec<u8>>>,
    buf: Vec<u8>,
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.flush()?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK));
            // The browser cancelled the download: stop zipping.
            self.tx
                .blocking_send(Ok(chunk))
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        }
        Ok(())
    }
}

/// Already-compressed formats are stored as is, to save CPU.
const STORED: &[&str] = &[
    "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "jpg", "jpeg", "png", "gif", "webp",
    "avif", "mp4", "mkv", "mov", "webm", "mp3", "ogg", "opus", "flac", "woff", "woff2", "pdf",
    "jar", "deb", "rpm",
];

fn zip_dir(dir: &Path, writer: &mut ChannelWriter) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let base = dir
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("root"));
    let mut zip = zip::ZipWriter::new_stream(&mut *writer);
    let mut skipped = Vec::new();
    // Symlinks are not followed: they could point outside the root.
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                skipped.push(format!("{e}"));
                continue;
            }
        };
        // Partial uploads and saves in progress are not part of the folder.
        let filename = entry.file_name().to_string_lossy();
        if filename.starts_with('.')
            && (filename.contains(".ce-upload-") || filename.contains(".ce-save-"))
        {
            continue;
        }
        let suffix = entry.path().strip_prefix(dir).unwrap_or(entry.path());
        // A filename may legitimately contain '\' on Unix. Do NOT turn it into
        // '/': that would forge extra path segments in the archive ("zip
        // slip"). Skip any component that is "..", empty, or holds a separator.
        if suffix.to_string_lossy().contains('\\')
            || suffix
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            skipped.push(format!(
                "{}: unsafe filename, skipped",
                suffix.to_string_lossy()
            ));
            continue;
        }
        let archive_path = base
            .join(suffix)
            .to_string_lossy()
            .trim_end_matches('/')
            .to_string();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                skipped.push(format!("{archive_path}: {e}"));
                continue;
            }
        };
        let mut options = SimpleFileOptions::default().last_modified_time(zip_time(&metadata));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            options = options.unix_permissions(metadata.permissions().mode() & 0o7777);
        }
        if entry.file_type().is_dir() {
            zip.add_directory(format!("{archive_path}/"), options)?;
        } else if entry.file_type().is_file() {
            let extension = archive_path
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let method = if STORED.contains(&extension.as_str()) {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            };
            let mut file = match crate::files::open_regular(entry.path()) {
                Ok(file) => file,
                Err(e) => {
                    skipped.push(format!("{archive_path}: {e}"));
                    continue;
                }
            };
            zip.start_file(
                archive_path.as_str(),
                options
                    .compression_method(method)
                    .large_file(metadata.len() >= u32::MAX as u64),
            )?;
            std::io::copy(&mut file, &mut zip)?;
        } else {
            skipped.push(format!("{archive_path}: symlink or special file, skipped"));
        }
    }
    if !skipped.is_empty() {
        zip.start_file(
            format!("{}/CODEENV-SKIPPED-FILES.txt", base.display()),
            SimpleFileOptions::default(),
        )?;
        zip.write_all(skipped.join("\n").as_bytes())?;
    }
    zip.finish()?;
    writer.flush()?;
    Ok(())
}

/// File mtime as a zip timestamp (UTC; zip has no time zone).
fn zip_time(metadata: &std::fs::Metadata) -> zip::DateTime {
    let seconds = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, seconds_of_day) = (seconds.div_euclid(86400), seconds.rem_euclid(86400));
    // Days since 1970-01-01 to civil date (H. Hinnant's algorithm).
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * march_month + 2) / 5 + 1) as u8;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    } as u8;
    let year = (year_of_era + era * 400 + i64::from(month <= 2)) as u16;
    zip::DateTime::from_date_and_time(
        year,
        month,
        day,
        (seconds_of_day / 3600) as u8,
        (seconds_of_day % 3600 / 60) as u8,
        (seconds_of_day % 60) as u8,
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_limit_keeps_previous_chunks() {
        let tmp = std::env::temp_dir().join(format!("codeenv-limit-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let root = tmp.canonicalize().unwrap();
        let params = |offset, done| UploadParams {
            dir: String::new(),
            path: "file.txt".into(),
            id: "limit".into(),
            offset,
            done,
            overwrite: false,
        };
        upload_chunk(&root, params(0, false), Body::from("first"))
            .await
            .unwrap();
        let chunk = axum::body::Bytes::from(vec![b'x'; 1 << 20]);
        let stream = futures_util::stream::iter(
            (0..65).map(move |_| Ok::<_, std::io::Error>(chunk.clone())),
        );
        let error = upload_chunk(&root, params(5, true), Body::from_stream(stream))
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.downcast_ref::<crate::files::HttpError>().unwrap().0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert!(!root.join("file.txt").exists());
        assert_eq!(
            std::fs::read(root.join(".file.txt.ce-upload-limit")).unwrap(),
            b"first"
        );
        upload_chunk(&root, params(5, true), Body::from(" last"))
            .await
            .unwrap();
        assert_eq!(std::fs::read(root.join("file.txt")).unwrap(), b"first last");
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn zip_time_civil() {
        let tmp = std::env::temp_dir().join(format!("codeenv-zt-{}", std::process::id()));
        std::fs::write(&tmp, b"x").unwrap();
        // 2024-02-29 13:37:42 UTC
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_709_213_862);
        std::fs::File::options()
            .write(true)
            .open(&tmp)
            .unwrap()
            .set_modified(t)
            .unwrap();
        let d = zip_time(&std::fs::metadata(&tmp).unwrap());
        assert_eq!(
            (d.year(), d.month(), d.day(), d.hour(), d.minute()),
            (2024, 2, 29, 13, 37)
        );
        std::fs::remove_file(&tmp).unwrap();
    }

    #[tokio::test]
    async fn upload_and_zip() {
        let tmp = std::env::temp_dir().join(format!("codeenv-up-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let root = tmp.canonicalize().unwrap();
        let upload = |path: &str, offset: u64, done: bool, overwrite: bool, data: &'static [u8]| {
            let root = root.clone();
            let params = UploadParams {
                dir: String::new(),
                path: path.into(),
                id: "u1".into(),
                offset,
                done,
                overwrite,
            };
            async move { upload_chunk(&root, params, Body::from(data)).await }
        };
        // Two chunks into a new subfolder.
        assert_eq!(
            upload("d/sub/f.txt", 0, false, false, b"hello ")
                .await
                .unwrap()
                .size,
            6
        );
        let result = upload("d/sub/f.txt", 6, true, false, b"world")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(root.join("d/sub/f.txt")).unwrap(),
            b"hello world"
        );
        assert!(result.path.unwrap().ends_with("d/sub/f.txt"));
        // Wrong offset is refused; existing file needs overwrite.
        upload("d/sub/g.txt", 0, false, false, b"abc")
            .await
            .unwrap();
        assert!(upload("d/sub/g.txt", 1, true, false, b"x").await.is_err());
        assert!(upload("d/sub/f.txt", 0, true, false, b"new").await.is_err());
        upload("d/sub/f.txt", 0, true, true, b"new").await.unwrap();
        assert_eq!(std::fs::read(root.join("d/sub/f.txt")).unwrap(), b"new");
        // Escapes are refused, including through a symlinked folder, and
        // nothing is created outside the root.
        assert!(upload("../evil", 0, true, false, b"x").await.is_err());
        assert!(upload("d/../../evil", 0, true, false, b"x").await.is_err());
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("codeenv-out-{}", std::process::id()));
            std::fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
            assert!(
                upload("link/newdir/x.txt", 0, true, false, b"x")
                    .await
                    .is_err()
            );
            assert!(!outside.join("newdir").exists());
            std::fs::remove_file(root.join("link")).unwrap();
            std::fs::remove_dir_all(&outside).unwrap();
        }

        // Zip of the folder contains the files under the folder name.
        let (tx, mut rx) = mpsc::channel(64);
        let dir = root.join("d");
        let zip_thread = std::thread::spawn(move || {
            zip_dir(
                &dir,
                &mut ChannelWriter {
                    tx,
                    buf: Vec::new(),
                },
            )
            .unwrap()
        });
        let mut bytes = Vec::new();
        while let Some(chunk) = rx.recv().await {
            bytes.extend(chunk.unwrap());
        }
        zip_thread.join().unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut content = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("d/sub/f.txt").unwrap(), &mut content)
            .unwrap();
        assert_eq!(content, "new");
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
