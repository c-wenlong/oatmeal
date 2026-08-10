//! The sidecar log, on disk.
//!
//! The sidecar's stderr is the only account of what audio capture, the ASR
//! model and the calendar watcher actually did. It used to go to the Rust
//! process's stderr, which in a bundled `.app` means nowhere: launched from
//! Finder there is no terminal attached, so every failure in there was
//! invisible unless the app happened to be started from a shell.
//!
//! A file rather than an in-memory ring, because the questions worth asking are
//! about what happened *before* you thought to look — "did the calendar watcher
//! ever start" is not answerable by a buffer that began when the panel opened.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Where the log lives, beside the database.
pub fn log_path(app_data: &Path) -> PathBuf {
    app_data.join("sidecar.log")
}

/// How large the log may grow before the oldest half is dropped.
///
/// Two megabytes is a few days of ordinary use and a few minutes of a crash
/// loop. Unbounded, a loop that logs on every restart fills the disk; rotating
/// files would be more machinery than one diagnostic file deserves.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Everything after the halfway mark, so trimming keeps the *recent* half.
///
/// Split on a line boundary: a log whose first line is the tail of another is
/// the kind of thing that makes someone doubt the rest of the file.
pub fn trim(contents: &str, max_bytes: usize) -> String {
    if contents.len() <= max_bytes {
        return contents.to_string();
    }
    let cut = contents.len() - max_bytes / 2;
    match contents[cut..].find('\n') {
        Some(offset) => contents[cut + offset + 1..].to_string(),
        // No newline in the back half: one enormous line, and none of it is
        // worth more than the space it takes.
        None => String::new(),
    }
}

/// Appends one line, trimming first if the file has grown too large.
pub fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_BYTES {
            let contents = std::fs::read_to_string(path).unwrap_or_default();
            std::fs::write(path, trim(&contents, MAX_BYTES as usize))?;
        }
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

/// The last `limit` lines, oldest first.
///
/// Reads from the end rather than loading the file: the whole point of the tail
/// is to answer a question without paying for two megabytes of history.
pub fn tail(path: &Path, limit: usize) -> std::io::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.seek(SeekFrom::End(0))?;
    // Generous: 400 bytes a line is far longer than these lines run, so one
    // read almost always suffices.
    let window = (limit as u64 * 400).min(len);
    file.seek(SeekFrom::Start(len - window))?;

    let mut lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    // The first line is probably a fragment, unless we read the whole file.
    if window < len && !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() > limit {
        lines.drain(..lines.len() - limit);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_log_is_left_alone() {
        assert_eq!(trim("a\nb\n", 100), "a\nb\n");
    }

    #[test]
    fn trimming_keeps_the_recent_half_and_starts_on_a_line() {
        // A log whose first line is the tail of another makes a reader doubt
        // everything below it.
        let contents = (0..100).map(|i| format!("line{i}\n")).collect::<String>();
        let trimmed = trim(&contents, 200);
        assert!(trimmed.len() <= 200);
        assert!(trimmed.starts_with("line"));
        assert!(trimmed.ends_with("line99\n"));
        // And it is the *end* that survives, not the beginning.
        assert!(!trimmed.contains("line0\n"));
    }

    #[test]
    fn one_enormous_line_is_dropped_rather_than_half_kept() {
        let contents = "x".repeat(1000);
        assert_eq!(trim(&contents, 100), "");
    }

    #[test]
    fn the_tail_is_the_end_of_the_file() {
        let dir = std::env::temp_dir().join(format!("oatmeal-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.log");
        let _ = std::fs::remove_file(&path);
        for i in 0..50 {
            append(&path, &format!("line{i}")).unwrap();
        }

        let last = tail(&path, 5).unwrap();
        assert_eq!(last.len(), 5);
        assert_eq!(last.last().unwrap(), "line49");
        // Oldest first, so it reads in the order it happened.
        assert_eq!(last.first().unwrap(), "line45");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_file_tails_to_itself() {
        let dir = std::env::temp_dir().join(format!("oatmeal-diag-s-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.log");
        let _ = std::fs::remove_file(&path);
        append(&path, "only").unwrap();
        // The fragment-dropping rule must not eat the one line there is.
        assert_eq!(tail(&path, 10).unwrap(), vec!["only".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
