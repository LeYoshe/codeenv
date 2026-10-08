//! Search in file contents (ripgrep's libraries) and by file name.
//!
//! Both walk with the `ignore` crate: .gitignore / .ignore rules apply (even
//! outside a git repository), `.git` is skipped, symlinks are not followed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, SearcherBuilder, Sink, SinkMatch};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Matches reported per file, and overall.
const MAX_PER_FILE: usize = 100;
const MAX_TOTAL: usize = 2000;
/// Files larger than this are not searched (minified bundles, data dumps).
const MAX_FILE_SIZE: u64 = 2 << 20;
/// Long lines (minified code) are cut around the first match.
const MAX_LINE_CHARS: usize = 300;
/// A single content search gives up after this long.
const SEARCH_BUDGET: Duration = Duration::from_secs(20);

fn walker(dir: &Path) -> ignore::WalkBuilder {
    // Disable ignore-file loading if the pre-scan finds a special or oversized
    // file. This check covers files under dir, not parent or global rules.
    let safe = ignore_files_safe(dir);
    let mut builder = ignore::WalkBuilder::new(dir);
    builder
        .hidden(false) // dotfiles are searched (.env, .github…), .git is not:
        .filter_entry(|e| e.file_name() != ".git")
        .git_ignore(safe)
        .git_exclude(safe)
        .git_global(safe)
        .ignore(safe)
        .parents(safe)
        .require_git(false)
        .max_filesize(Some(MAX_FILE_SIZE))
        .follow_links(false);
    builder
}

/// Size over which we treat an ignore file as hostile rather than read it.
const MAX_IGNORE_SIZE: u64 = 1 << 20;

/// Checks ignore-file types and sizes before the search library reads them.
fn ignore_files_safe(dir: &Path) -> bool {
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let name = entry.file_name();
        if name != ".gitignore" && name != ".ignore" {
            continue;
        }
        // file_type() here comes from the directory entry (no extra open).
        let ok = entry.file_type().is_file()
            && entry
                .metadata()
                .map(|m| m.len() <= MAX_IGNORE_SIZE)
                .unwrap_or(false);
        if !ok {
            tracing::warn!("ignoring unsafe ignore file {}", entry.path().display());
            return false;
        }
    }
    true
}

#[derive(Deserialize)]
pub struct TextQuery {
    pub dir: String,
    #[serde(rename = "q")]
    pub text: String,
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive (default: insensitive).
    #[serde(default)]
    pub case: bool,
}

#[derive(Serialize)]
struct FileMatches {
    file: String,
    matches: Vec<TextMatch>,
    /// More matches in this file than reported.
    more: bool,
}

#[derive(Serialize)]
struct TextMatch {
    line: u64,
    text: String,
    /// Match spans in `text`, in UTF-16 code units (JavaScript string indices).
    ranges: Vec<[usize; 2]>,
}

pub fn build_matcher(query: &TextQuery) -> Result<RegexMatcher> {
    if query.text.is_empty() {
        return Err(crate::files::http_err(
            axum::http::StatusCode::BAD_REQUEST,
            "empty search",
        ));
    }
    RegexMatcherBuilder::new()
        .case_insensitive(!query.case)
        .fixed_strings(!query.regex)
        .build(&query.text)
        .map_err(|e| {
            // The full message quotes the pattern as rewritten internally;
            // its last line ("error: unclosed group") is the useful part.
            let msg = e.to_string();
            let cause = msg
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or(&msg)
                .trim();
            let cause = cause.strip_prefix("error: ").unwrap_or(cause);
            crate::files::http_err(
                axum::http::StatusCode::BAD_REQUEST,
                format!("invalid expression: {cause}"),
            )
        })
}

/// Streams NDJSON lines: one `FileMatches` per file with matches, then
/// `{"done":true,…}`. Stops early when `tx` is closed (client went away).
pub fn search_text(dir: PathBuf, matcher: RegexMatcher, tx: mpsc::Sender<Vec<u8>>) {
    let total = Arc::new(AtomicUsize::new(0));
    let files = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    walker(&dir).build_parallel().run(|| {
        let tx = tx.clone();
        let matcher = matcher.clone();
        let total = total.clone();
        let files = files.clone();
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(0))
            .build();
        Box::new(move |entry| {
            use ignore::WalkState;
            // Stop on: client gone, enough matches, or the time budget spent.
            if tx.is_closed()
                || total.load(Ordering::Relaxed) >= MAX_TOTAL
                || started.elapsed() > SEARCH_BUDGET
            {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let mut sink = MatchCollector {
                matcher: &matcher,
                hits: Vec::new(),
                more: false,
            };
            if searcher
                .search_path(&matcher, entry.path(), &mut sink)
                .is_err()
                || sink.hits.is_empty()
            {
                return WalkState::Continue;
            }
            total.fetch_add(sink.hits.len(), Ordering::Relaxed);
            files.fetch_add(1, Ordering::Relaxed);
            let line = FileMatches {
                file: entry.path().to_string_lossy().into_owned(),
                matches: sink.hits,
                more: sink.more,
            };
            let mut json = serde_json::to_vec(&line).unwrap_or_default();
            json.push(b'\n');
            if tx.blocking_send(json).is_err() {
                return WalkState::Quit;
            }
            WalkState::Continue
        })
    });
    let match_count = total.load(Ordering::Relaxed);
    let done = serde_json::json!({
        "done": true,
        "files": files.load(Ordering::Relaxed),
        "matches": match_count,
        "truncated": match_count >= MAX_TOTAL || started.elapsed() > SEARCH_BUDGET,
        "ms": started.elapsed().as_millis() as u64,
    });
    let _ = tx.blocking_send(format!("{done}\n").into_bytes());
}

struct MatchCollector<'a> {
    matcher: &'a RegexMatcher,
    hits: Vec<TextMatch>,
    more: bool,
}

impl Sink for MatchCollector<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _: &grep_searcher::Searcher,
        line_match: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        if self.hits.len() >= MAX_PER_FILE {
            self.more = true;
            return Ok(false);
        }
        let bytes = line_match.bytes();
        let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let mut spans = Vec::new();
        let _ = self.matcher.find_iter(bytes, |span| {
            spans.push((span.start(), span.end()));
            spans.len() < 50
        });
        let (text, ranges) = excerpt(bytes, &spans);
        self.hits.push(TextMatch {
            line: line_match.line_number().unwrap_or(0),
            text,
            ranges,
        });
        Ok(true)
    }
}

/// The line (cut around the first match if too long) and the match spans
/// converted from byte offsets to UTF-16 offsets within the returned text.
fn excerpt(bytes: &[u8], spans: &[(usize, usize)]) -> (String, Vec<[usize; 2]>) {
    let line = String::from_utf8_lossy(bytes);
    // Invalid UTF-8 changes byte offsets, so omit highlighting in that case.
    let spans = if matches!(line, std::borrow::Cow::Borrowed(_)) {
        spans
    } else {
        &[]
    };
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let char_at = |byte: usize| chars.partition_point(|(b, _)| *b < byte);
    let (mut from, mut to) = (0, chars.len());
    if chars.len() > MAX_LINE_CHARS {
        let first = spans.first().map(|start| char_at(start.0)).unwrap_or(0);
        from = first.saturating_sub(MAX_LINE_CHARS / 3);
        to = (from + MAX_LINE_CHARS).min(chars.len());
    }
    let prefix = if from > 0 { "…" } else { "" };
    let mut text: String = prefix.to_string();
    text.extend(chars[from..to].iter().map(|(_, c)| *c));
    if to < chars.len() {
        text.push('…');
    }
    let utf16 = |char_index: usize| -> usize {
        prefix.encode_utf16().count()
            + chars[from..char_index.clamp(from, to)]
                .iter()
                .map(|(_, c)| c.len_utf16())
                .sum::<usize>()
    };
    let ranges = spans
        .iter()
        .map(|&(start, end)| (char_at(start), char_at(end)))
        .filter(|&(start, end)| end > from && start < to)
        .map(|(start, end)| [utf16(start), utf16(end)])
        .collect();
    (text, ranges)
}

#[derive(Serialize)]
pub struct NameHit {
    pub path: String,
    pub rel: String,
    /// Indices (UTF-16) of matched characters in `rel`, for highlighting.
    pub marks: Vec<usize>,
}

/// Files under `dir` whose relative path fuzzily matches `query`, best first.
pub fn search_names(dir: &Path, query: &str, limit: usize) -> Vec<NameHit> {
    const MAX_ENTRIES: usize = 300_000;
    const BUDGET: Duration = Duration::from_millis(1500);
    let query: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if query.is_empty() {
        return Vec::new();
    }
    let started = Instant::now();
    let mut scored: Vec<(i64, NameHit)> = Vec::new();
    for (i, entry) in walker(dir).build().enumerate() {
        if i >= MAX_ENTRIES || (i % 1024 == 0 && started.elapsed() > BUDGET) {
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(dir) else {
            continue;
        };
        let rel = rel.to_string_lossy();
        if let Some((score, marks)) = fuzzy(&query, &rel) {
            scored.push((
                score,
                NameHit {
                    path: entry.path().to_string_lossy().into_owned(),
                    rel: rel.into_owned(),
                    marks,
                },
            ));
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.rel.len().cmp(&b.1.rel.len()))
    });
    scored.into_iter().take(limit).map(|(_, h)| h).collect()
}

/// Subsequence match of `query` (lowercase) in `text`. Rewards consecutive
/// characters, matches at word starts, and matches inside the file name
/// (after the last '/'); matching right-to-left keeps them near the end.
fn fuzzy(query: &[char], text: &str) -> Option<(i64, Vec<usize>)> {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    if lower.len() != chars.len() {
        return None; // lowercase changed the length (rare scripts): skip
    }
    let name_start = text
        .rfind('/')
        .map(|i| text[..=i].chars().count())
        .unwrap_or(0);
    let mut positions = Vec::with_capacity(query.len());
    let mut before = lower.len();
    for &query_char in query.iter().rev() {
        let position = lower[..before].iter().rposition(|&c| c == query_char)?;
        positions.push(position);
        before = position;
    }
    positions.reverse();
    let mut score: i64 = 0;
    for (match_index, &position) in positions.iter().enumerate() {
        if match_index > 0 && positions[match_index - 1] + 1 == position {
            score += 8;
        }
        if position == 0 || matches!(chars[position - 1], '/' | '_' | '-' | '.' | ' ') {
            score += 6;
        }
        if position >= name_start {
            score += 4;
        }
    }
    score -= (chars.len() as i64) / 8;
    // UTF-16 indices for the browser.
    let mut utf16_offsets = Vec::with_capacity(chars.len());
    let mut offset = 0;
    for c in &chars {
        utf16_offsets.push(offset);
        offset += c.len_utf16();
    }
    Some((
        score,
        positions
            .iter()
            .map(|&position| utf16_offsets[position])
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_ranges_utf16() {
        let line = "é = foo(😀) + foo";
        let spans: Vec<(usize, usize)> = line
            .match_indices("foo")
            .map(|(i, s)| (i, i + s.len()))
            .collect();
        let (text, ranges) = excerpt(line.as_bytes(), &spans);
        let utf16: Vec<u16> = text.encode_utf16().collect();
        for [a, b] in ranges {
            assert_eq!(String::from_utf16(&utf16[a..b]).unwrap(), "foo");
        }
        let long = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let (text, ranges) = excerpt(long.as_bytes(), &[(1000, 1006)]);
        assert!(
            text.starts_with('…')
                && text.ends_with('…')
                && text.chars().count() <= MAX_LINE_CHARS + 2
        );
        let utf16: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&utf16[ranges[0][0]..ranges[0][1]]).unwrap(),
            "needle"
        );
    }

    #[test]
    fn fuzzy_prefers_file_names() {
        let query: Vec<char> = "main".chars().collect();
        let (a, _) = fuzzy(&query, "src/main.rs").unwrap();
        let (b, _) = fuzzy(&query, "m/a/i/n/other.rs").unwrap();
        assert!(a > b);
        assert!(fuzzy(&query, "src/lib.rs").is_none());
    }

    #[tokio::test]
    async fn text_search_respects_gitignore_and_binaries() {
        let tmp = std::env::temp_dir().join(format!("codeenv-search-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::write(tmp.join("a.txt"), "hello Needle\nnothing\nneedle again\n").unwrap();
        std::fs::write(tmp.join(".env"), "TOKEN=needle\n").unwrap();
        std::fs::write(tmp.join("node_modules/x/b.txt"), "needle\n").unwrap();
        std::fs::write(tmp.join(".git/config"), "needle\n").unwrap();
        std::fs::write(tmp.join("bin.dat"), b"needle\0\x01\x02").unwrap();
        let query = TextQuery {
            dir: String::new(),
            text: "needle".into(),
            regex: false,
            case: false,
        };
        let (tx, mut rx) = mpsc::channel(16);
        let dir = tmp.clone();
        let matcher = build_matcher(&query).unwrap();
        std::thread::spawn(move || search_text(dir, matcher, tx));
        let mut output = String::new();
        while let Some(chunk) = rx.recv().await {
            output.push_str(&String::from_utf8(chunk).unwrap());
        }
        assert!(
            output.contains("a.txt") && output.contains(".env"),
            "{output}"
        );
        assert!(
            !output.contains("node_modules")
                && !output.contains(".git/")
                && !output.contains("bin.dat"),
            "{output}"
        );
        assert!(output.contains("\"matches\":3"), "{output}");
        // Case-sensitive: only "needle" lowercase lines.
        let query = TextQuery {
            dir: String::new(),
            text: "Needle".into(),
            regex: false,
            case: true,
        };
        let (tx, mut rx) = mpsc::channel(16);
        let (dir, matcher) = (tmp.clone(), build_matcher(&query).unwrap());
        std::thread::spawn(move || search_text(dir, matcher, tx));
        let mut output = String::new();
        while let Some(chunk) = rx.recv().await {
            output.push_str(&String::from_utf8(chunk).unwrap());
        }
        assert!(output.contains("\"matches\":1"), "{output}");
        assert!(
            build_matcher(&TextQuery {
                dir: String::new(),
                text: "(".into(),
                regex: true,
                case: false
            })
            .is_err()
        );
        let names = search_names(&tmp, "atx", 10);
        assert_eq!(names.first().map(|n| n.rel.as_str()), Some("a.txt"));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
