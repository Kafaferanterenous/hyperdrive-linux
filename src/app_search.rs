//! HyperDrive - recursive background file search.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct SearchState {
    pub query: String,
    pub input: String,
    pub ext_filter: String,
    pub min_size_kb: u64,
    pub content_grep: bool,
    pub dir: PathBuf,
    pub results: Arc<Mutex<Vec<PathBuf>>>,
    pub cancelled: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    pub searched_for: Arc<Mutex<String>>, // query the current results belong to
}

pub const MAX_RESULTS: usize = 2000;

impl SearchState {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            query: String::new(),
            input: String::new(),
            ext_filter: String::new(),
            min_size_kb: 0,
            content_grep: false,
            dir,
            results: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(true)),
            searched_for: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Restart the search thread if the query/filters changed.
    pub fn maybe_start(&mut self) {
        let q = format!(
            "{}|{}|{}|{}",
            self.input.trim().to_lowercase(),
            self.ext_filter.trim().to_lowercase(),
            self.min_size_kb,
            if self.content_grep { "1" } else { "0" }
        );
        if q.len() < 4 || q == self.query {
            return;
        }
        let parts: Vec<&str> = q.split('|').collect();
        let needle = parts[0].to_string();
        let extf = parts[1].to_string();
        let minkb: u64 = parts[2].parse().unwrap_or(0);
        let do_content = parts.get(3) == Some(&"1");
        self.cancelled.store(true, Ordering::Relaxed);
        let cancel_new = Arc::new(AtomicBool::new(false));
        let done_new = Arc::new(AtomicBool::new(false));
        let results_new: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

        self.cancelled = cancel_new.clone();
        self.done = done_new.clone();
        self.results = results_new.clone();
        self.searched_for = Arc::new(Mutex::new(needle.clone()));
        self.query = q.clone();

        let root = self.dir.clone();
        std::thread::spawn(move || {
            let mut stack = vec![root];
            let mut count = 0usize;
            while let Some(d) = stack.pop() {
                if cancel_new.load(Ordering::Relaxed) || count >= MAX_RESULTS {
                    break;
                }
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    if count.is_multiple_of(64) && cancel_new.load(Ordering::Relaxed) {
                        return;
                    }
                    let name = e.file_name().to_string_lossy().into_owned();
                    let path = e.path();
                    if file_matches(&name, &path, &needle, &extf, minkb, do_content).unwrap_or(false) {
                            if results_new.lock().map(|mut r| r.push(path)).is_err() {
                                return;
                            }
                            count += 1;
                            if count >= MAX_RESULTS {
                                break;
                            }
                        }
                    if let Ok(ft) = e.file_type() {
                        if ft.is_dir() {
                            stack.push(e.path());
                        }
                    }
                }
            }
            done_new.store(true, Ordering::Relaxed);
        });
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Vec<PathBuf> {
        self.results.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

/// Deterministic match decision for one file. Pure + testable.
///
/// Returns Ok(true) when the file matches the search, Ok(false) when it does
/// not, Err when the file cannot be read (treated as non-match by callers).
/// Rules mirror the UI search contract:
/// - dotfiles are never matched
/// - filename match is case-insensitive substring
/// - `extf` when non-empty restricts to files whose extension equals it
/// - `minkb` when >0 skips files smaller than that many KB
/// - content grep reads text-like files (<1MB, no NUL byte in head)
pub fn file_matches(
    name: &str,
    path: &std::path::Path,
    needle: &str,
    extf: &str,
    minkb: u64,
    do_content: bool,
) -> std::io::Result<bool> {
    if name.starts_with('.') {
        return Ok(false);
    }
    let low = name.to_lowercase();
    let needle_low = needle.to_lowercase();
    if !extf.is_empty() {
        let ext = std::path::Path::new(name)
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext != extf.to_lowercase() {
            return Ok(false);
        }
    }
    let meta = std::fs::metadata(path)?;
    let sz = meta.len();
    if minkb > 0 && sz < minkb * 1024 {
        return Ok(false);
    }
    if low.contains(&needle_low) {
        return Ok(true);
    }
    if do_content && sz > 0 && sz < 1_000_000 {
        let bytes = std::fs::read(path)?;
        let head = bytes.len().min(512);
        if !bytes[..head].contains(&0) {
            let text = String::from_utf8_lossy(&bytes);
            if text.to_lowercase().contains(&needle_low) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("hd_search_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn filename_match_is_case_insensitive() {
        let path = tmp().join("Readme.TXT");
        std::fs::write(&path, "hello").unwrap();
        assert!(file_matches("Readme.TXT", &path, "readme", "", 0, false).unwrap());
        assert!(!file_matches("Readme.TXT", &path, "zzz", "", 0, false).unwrap());
    }

    #[test]
    fn dotfiles_are_never_matched() {
        let path = tmp().join(".hidden");
        std::fs::write(&path, "x").unwrap();
        assert!(!file_matches(".hidden", &path, "hidden", "", 0, false).unwrap());
    }

    #[test]
    fn extension_filter_restricts_matches() {
        let path = tmp().join("notes.md");
        std::fs::write(&path, "x").unwrap();
        assert!(file_matches("notes.md", &path, "notes", "md", 0, false).unwrap());
        assert!(!file_matches("notes.md", &path, "notes", "txt", 0, false).unwrap());
        // no extension given => not matched by the ".md" filter
        assert!(!file_matches("notes", &tmp().join("solo"), "notes", "md", 0, false).unwrap());
    }

    #[test]
    fn min_size_kb_skips_small_files() {
        let path = tmp().join("tiny.txt");
        std::fs::write(&path, "b").unwrap();
        assert!(file_matches("tiny.txt", &path, "tiny", "", 0, false).unwrap());
        assert!(!file_matches("tiny.txt", &path, "tiny", "", 1, false).unwrap());
    }

    #[test]
    fn content_grep_reads_text_only() {
        let near = tmp().join("near.txt");
        std::fs::write(&near, "the needle is here").unwrap();
        assert!(file_matches("near.txt", &near, "needle", "", 0, true).unwrap());
        assert!(!file_matches("near.txt", &near, "needle", "", 0, false).unwrap());

        let bin = tmp().join("blob.dat");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert!(!file_matches("blob.dat", &bin, "abc", "", 0, true).unwrap());
    }

    #[test]
    fn content_grep_skips_huge_files() {
        let big = tmp().join("hugefile.data");
        std::fs::write(&big, "y".repeat(1_000_002)).unwrap();
        assert!(!file_matches("hugefile.data", &big, "needle-that-is-not-in-name", "", 0, true).unwrap());
        let _ = std::fs::remove_file(&big);
    }
}
