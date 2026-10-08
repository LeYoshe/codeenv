//! File operations for the explorer and the editor, confined to the
//! configured root.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use axum::http::StatusCode;
use serde::Serialize;

/// An error carrying the HTTP status the API should answer with.
#[derive(Debug)]
pub struct HttpError(pub StatusCode, pub String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for HttpError {}

pub fn http_err(status: StatusCode, msg: impl Into<String>) -> anyhow::Error {
    HttpError(status, msg.into()).into()
}

/// Files larger than this are not opened in the editor.
pub const MAX_EDIT_SIZE: u64 = 5 << 20;

const MAX_ENTRIES: usize = 5000;

/// Opens a path for reading, refusing anything that is not a regular file.
/// Refuses final symlinks and uses `O_NONBLOCK` to avoid hanging on FIFOs.
/// Checks the opened file before reading it.
pub fn open_regular(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("{}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(http_err(StatusCode::BAD_REQUEST, "not a regular file"));
    }
    Ok(file)
}

#[derive(Serialize)]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub link: bool,
    pub size: u64,
    pub mtime: u64,
}

/// Resolves `path` (absolute, or relative to `root`) and checks that it stays
/// inside `root` once symlinks are resolved. `root` must be canonical.
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf> {
    let path = if path.is_empty() {
        root.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved_path = path
        .canonicalize()
        .with_context(|| format!("{}", path.display()))?;
    if !resolved_path.starts_with(root) {
        bail!(
            "{} is outside of {}",
            resolved_path.display(),
            root.display()
        );
    }
    Ok(resolved_path)
}

/// Resolves a path that may not exist yet: its parent must exist inside
/// `root`, and its last component must be a plain name.
pub fn resolve_new(root: &Path, path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| http_err(StatusCode::BAD_REQUEST, format!("invalid name in {path:?}")))?;
    check_name(name)?;
    let parent = path
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = resolve(root, &parent)?.join(name);
    // An existing symlink must not lead out of the root either.
    if target.symlink_metadata().is_ok() {
        return resolve(root, &target.to_string_lossy());
    }
    Ok(target)
}

/// A single path component chosen by the user.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(http_err(
            StatusCode::BAD_REQUEST,
            format!("invalid name {name:?}"),
        ));
    }
    Ok(())
}

/// Identifies the content of a file, to detect changes made since the editor
/// read it. A content hash: mtime + size misses same-size rewrites within the
/// filesystem's timestamp granularity (a few ms on many Linux setups).
pub fn version(content: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(content)[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Serialize)]
pub struct TextFile {
    pub path: String,
    pub content: String,
    pub version: String,
    pub readonly: bool,
}

pub fn read_text(root: &Path, path: &str) -> Result<TextFile> {
    use std::io::Read;
    let path = resolve(root, path)?;
    let file = open_regular(&path)?;
    if file.metadata()?.len() > MAX_EDIT_SIZE {
        return Err(http_err(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file too large for the editor ({} MiB max)",
                MAX_EDIT_SIZE >> 20
            ),
        ));
    }
    // Bound the read in case the file grows between the stat and the read.
    let mut bytes = Vec::new();
    file.take(MAX_EDIT_SIZE + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("{}", path.display()))?;
    if bytes.len() as u64 > MAX_EDIT_SIZE {
        return Err(http_err(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file too large for the editor ({} MiB max)",
                MAX_EDIT_SIZE >> 20
            ),
        ));
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(http_err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "binary file"));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| http_err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "file is not UTF-8"))?;
    let readonly = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .is_err();
    let version = version(content.as_bytes());
    Ok(TextFile {
        path: path.to_string_lossy().into_owned(),
        content,
        version,
        readonly,
    })
}

/// Saves `content` atomically (temporary file + rename, keeping the mode).
/// If `expected` is given and the file changed since, nothing is written and
/// a 409 is returned.
pub fn write_text(
    root: &Path,
    path: &str,
    content: &str,
    expected: Option<&str>,
) -> Result<String> {
    use std::io::Read;
    // Keep the version check and replacement together across editor requests.
    // External programs do not take this lock; their writes remain best-effort.
    static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _save = SAVE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("editor save lock poisoned"))?;
    if content.len() as u64 > MAX_EDIT_SIZE {
        return Err(http_err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file too large for the editor (5 MiB max)",
        ));
    }
    let path = resolve(root, path)?;
    let metadata = open_regular(&path)?.metadata()?;
    if std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .is_err()
    {
        return Err(http_err(StatusCode::FORBIDDEN, "read-only file"));
    }
    if let Some(expected) = expected {
        let mut current = Vec::new();
        open_regular(&path)?
            .take(MAX_EDIT_SIZE + 1)
            .read_to_end(&mut current)?;
        if version(&current) != expected {
            return Err(http_err(StatusCode::CONFLICT, "file has changed on disk"));
        }
    }
    let dir = path.parent().context("file without parent")?;
    let name = path
        .file_name()
        .context("file without name")?
        .to_string_lossy();
    let temp_path = dir.join(format!(".{name}.ce-save-{:08x}", rand::random::<u32>()));
    let result = (|| -> Result<()> {
        // create_new never follows a symlink and never truncates an existing
        // file. The temp file is born with the destination's permissions
        // (umask may only narrow them), so a 0600 file is never world-readable.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(metadata.permissions().mode() & 0o777);
        }
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("{}", temp_path.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        // Restore exactly the original mode (in case umask narrowed it).
        std::fs::set_permissions(&temp_path, metadata.permissions())?;
        std::fs::rename(&temp_path, &path).with_context(|| format!("{}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result?;
    Ok(version(content.as_bytes()))
}

pub fn create(root: &Path, path: &str, dir: bool) -> Result<String> {
    let path = resolve_new(root, path)?;
    if path.symlink_metadata().is_ok() {
        return Err(http_err(
            StatusCode::CONFLICT,
            format!("{} already exists", path.display()),
        ));
    }
    if dir {
        std::fs::create_dir(&path)
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map(drop)
    }
    .with_context(|| format!("{}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Resolves an existing entry *without* following it if it is a symlink:
/// renaming or deleting a link acts on the link. Its parent must be inside
/// `root`, and the root itself is never returned.
pub fn resolve_entry(root: &Path, path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| http_err(StatusCode::BAD_REQUEST, format!("invalid path {path:?}")))?;
    check_name(name)?;
    let parent = path
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let entry = resolve(root, &parent)?.join(name);
    if entry == root {
        return Err(http_err(
            StatusCode::BAD_REQUEST,
            "the root cannot be modified",
        ));
    }
    entry.symlink_metadata().map_err(|_| {
        http_err(
            StatusCode::NOT_FOUND,
            format!("{} does not exist", entry.display()),
        )
    })?;
    Ok(entry)
}

/// Renames or moves an entry. Never overwrites.
pub fn rename(root: &Path, from: &str, to: &str) -> Result<String> {
    let source = resolve_entry(root, from)?;
    let destination_path = Path::new(to);
    let name = destination_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| http_err(StatusCode::BAD_REQUEST, "invalid name"))?;
    check_name(name)?;
    let destination_parent = resolve(
        root,
        &destination_path
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )?;
    let destination = destination_parent.join(name);
    if destination == source {
        return Ok(destination.to_string_lossy().into_owned());
    }
    if destination.symlink_metadata().is_ok() {
        return Err(http_err(
            StatusCode::CONFLICT,
            format!("{} already exists", destination.display()),
        ));
    }
    let source_is_dir = source.symlink_metadata()?.is_dir();
    if source_is_dir && destination_parent.starts_with(&source) {
        return Err(http_err(
            StatusCode::BAD_REQUEST,
            "cannot move a folder into itself",
        ));
    }
    std::fs::rename(&source, &destination).map_err(|e| {
        if e.raw_os_error() == Some(libc::EXDEV) {
            http_err(
                StatusCode::BAD_REQUEST,
                "moving between filesystems is not supported",
            )
        } else {
            anyhow::Error::from(e).context(format!(
                "{} → {}",
                source.display(),
                destination.display()
            ))
        }
    })?;
    Ok(destination.to_string_lossy().into_owned())
}

/// Deletes a file, a symlink (not its target) or a folder with its content.
pub fn delete(root: &Path, path: &str) -> Result<()> {
    let path = resolve_entry(root, path)?;
    let metadata = path.symlink_metadata()?;
    // remove_dir_all does not follow symlinks found inside the folder.
    if metadata.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    }
    .with_context(|| format!("{}", path.display()))
}

pub fn list(root: &Path, path: &str) -> Result<Listing> {
    let dir = resolve(root, path)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(&dir).with_context(|| format!("{}", dir.display()))? {
        let Ok(entry) = entry else { continue };
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let link = file_type.is_symlink();
        // Follow symlinks to know whether they point to a directory; broken
        // links are shown as files.
        let metadata = if link {
            std::fs::metadata(entry.path()).ok()
        } else {
            entry.metadata().ok()
        };
        let (dir, size, mtime) = match &metadata {
            Some(metadata) => (
                metadata.is_dir(),
                if metadata.is_dir() { 0 } else { metadata.len() },
                metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            ),
            None => (false, 0, 0),
        };
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir,
            link,
            size,
            mtime,
        });
    }
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing {
        path: dir.to_string_lossy().into_owned(),
        entries,
        truncated,
    })
}

/// Working directory for a new terminal: absolute, `~/…`, relative to the
/// root, or the root itself when empty. Must be an existing directory.
pub fn terminal_cwd(root: &Path, home: &Path, cwd: &str) -> Result<PathBuf> {
    let cwd = cwd.trim();
    let path = if cwd.is_empty() {
        root.to_path_buf()
    } else if cwd == "~" {
        home.to_path_buf()
    } else if let Some(rest) = cwd.strip_prefix("~/") {
        home.join(rest)
    } else {
        root.join(cwd)
    };
    let metadata = std::fs::metadata(&path).with_context(|| format!("{}", path.display()))?;
    if !metadata.is_dir() {
        bail!("{} is not a directory", path.display());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confinement() {
        let tmp = std::env::temp_dir().join(format!("codeenv-test-{}", std::process::id()));
        let root = tmp.join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(tmp.join("outside")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.join("outside"), root.join("escape")).unwrap();
        let root = root.canonicalize().unwrap();

        assert_eq!(resolve(&root, "").unwrap(), root);
        assert_eq!(resolve(&root, "sub").unwrap(), root.join("sub"));
        assert_eq!(
            resolve(&root, root.join("sub").to_str().unwrap()).unwrap(),
            root.join("sub")
        );
        assert!(resolve(&root, "..").is_err());
        assert!(resolve(&root, "/").is_err());
        #[cfg(unix)]
        assert!(resolve(&root, "escape").is_err());

        // Editor round trip: conflict detection, mode kept, symlinks written through.
        let file_path = root.join("sub/a.sh");
        std::fs::write(&file_path, "echo 1\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            &file_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o750),
        )
        .unwrap();
        let original = read_text(&root, "sub/a.sh").unwrap();
        let saved_version =
            write_text(&root, "sub/a.sh", "echo 2\n", Some(&original.version)).unwrap();
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "echo 2\n");
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(&file_path).unwrap().permissions()
            ) & 0o777,
            0o750
        );
        assert!(write_text(&root, "sub/a.sh", "stale\n", Some(&original.version)).is_err());
        assert!(write_text(&root, "sub/a.sh", "echo 3\n", Some(&saved_version)).is_ok());
        assert!(
            write_text(
                &root,
                "sub/a.sh",
                &"x".repeat(MAX_EDIT_SIZE as usize + 1),
                None
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "echo 3\n");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&file_path, root.join("sub/link.sh")).unwrap();
            write_text(&root, "sub/link.sh", "via link\n", None).unwrap();
            assert!(
                root.join("sub/link.sh")
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "via link\n");
            assert!(write_text(&root, "escape", "x", None).is_err());
        }
        std::fs::write(root.join("sub/bin"), b"a\0b").unwrap();
        assert!(read_text(&root, "sub/bin").is_err());
        assert!(create(&root, "sub/new", true).is_ok());
        assert!(create(&root, "sub/new", true).is_err());
        assert!(create(&root, "../x", false).is_err());
        for name in ["a.sh", "bin", "link.sh"] {
            let _ = std::fs::remove_file(root.join("sub").join(name));
        }
        std::fs::remove_dir(root.join("sub/new")).unwrap();

        // Rename / move / delete act on links, not targets, and stay inside.
        std::fs::write(root.join("sub/r.txt"), "r").unwrap();
        let moved = rename(&root, "sub/r.txt", "r2.txt").unwrap();
        assert_eq!(moved, root.join("r2.txt").to_string_lossy());
        std::fs::write(root.join("sub/r.txt"), "again").unwrap();
        assert!(
            rename(&root, "r2.txt", "sub/r.txt").is_err(),
            "must not overwrite"
        );
        assert!(
            rename(&root, "sub", "sub/inner").is_err(),
            "folder into itself"
        );
        assert!(rename(&root, "r2.txt", "../r2.txt").is_err());
        assert!(rename(&root, "", "x").is_err(), "root");
        delete(&root, "r2.txt").unwrap();
        delete(&root, "sub/r.txt").unwrap();
        #[cfg(unix)]
        {
            delete(&root, "escape").unwrap();
            assert!(
                tmp.join("outside").exists(),
                "deleting a link keeps its target"
            );
            std::os::unix::fs::symlink(tmp.join("outside"), root.join("escape")).unwrap();
        }
        assert!(delete(&root, "..").is_err());

        let listing = list(&root, "").unwrap();
        assert_eq!(listing.entries[0].name, "escape");
        assert!(listing.entries.iter().all(|e| e.dir));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
