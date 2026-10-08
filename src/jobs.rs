//! HyperDrive - background file-operation job manager.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobKind {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone)]
pub struct Job {
    pub label: String,
    pub total: u64,
    pub done: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    pub state: JobState,
    pub error: Option<String>,
}

struct JobCore {
    jobs: Mutex<Vec<Arc<Mutex<Job>>>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

pub struct JobManager {
    core: std::sync::Arc<JobCore>,
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            core: std::sync::Arc::new(JobCore {
                jobs: Mutex::new(Vec::new()),
                handles: Mutex::new(Vec::new()),
            }),
        }
    }

    fn register(&self, mut job: Job) -> Arc<Mutex<Job>> {
        let arc = Arc::new(Mutex::new(job.clone()));
        if let Ok(mut j)=self.core.jobs.lock(){ j.push(arc.clone()); }
        let _ = &mut job;
        arc
    }

    fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let h = std::thread::spawn(f);
        if let Ok(mut hs)=self.core.handles.lock(){ hs.push(h); }
        // Reap finished handles opportunistically.
        if let Ok(mut hs)=self.core.handles.lock(){ hs.retain(|h| !h.is_finished()); }
    }

    /// Enqueue a copy/move of many sources into dest_dir.
    pub fn enqueue_transfer(&self, kind: JobKind, sources: Vec<PathBuf>, dest_dir: PathBuf) {
        let verb = match kind { JobKind::Move => "Move", JobKind::Copy => "Copy" };
        let label = format!("{verb} {} item(s) \u{2192} {}", sources.len(), dest_dir.display());
        let total: u64 = sources.iter().filter_map(|p| dir_size_quick(p)).sum();
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { label, total, done: done.clone(), cancel: cancel.clone(), state: JobState::Running, error: None };
        let arc = self.register(job);
        let _core = std::sync::Arc::clone(&self.core);

        self.spawn(move || {
            for src in &sources {
                if cancel.load(Ordering::Relaxed) { break; }
                let fname = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                let mut dst = dest_dir.join(&fname);
                if dst.exists() {
                    let stem = src.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    let mut n = 2;
                    while { dst = dest_dir.join(format!("{stem} ({n}){ext}")); dst.exists() } { n += 1; }
                }
                let r = transfer_one(src, &dst, kind == JobKind::Move, &done, &cancel);
                if let Err(e) = r {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}: {e}", src.display()));
                    }
                    return;
                }
            }
            if let Ok(mut j) = arc.lock() {
                j.state = if cancel.load(Ordering::Relaxed) { JobState::Cancelled } else { JobState::Done };
            }
        });
    }

    /// Permanent deletion with item-count progress.
    pub fn enqueue_delete(&self, paths: Vec<PathBuf>) {
        let label = format!("Delete {} item(s) permanently", paths.len());
        let total = paths.len() as u64;
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { label, total, done: done.clone(), cancel: cancel.clone(), state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            for p in &paths {
                if cancel.load(Ordering::Relaxed) { break; }
                let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                if let Err(e) = r {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}: {e}", p.display()));
                    }
                    return;
                }
                done.fetch_add(1, Ordering::Relaxed);
            }
            if let Ok(mut j) = arc.lock() {
                j.state = if cancel.load(Ordering::Relaxed) { JobState::Cancelled } else { JobState::Done };
            }
        });
    }

    /// Archive extraction via external tool (no fine-grained progress).
    pub fn enqueue_extract(&self, archive: PathBuf, mut extractor: std::process::Command, dest_note: String) {
        let label = format!("Extract {} \u{2192} {}", archive.display(), dest_note);
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { label, total: 100, done, cancel, state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            match extractor.status() {
                Ok(s) if s.success() => {
                    if let Ok(mut j) = arc.lock() { j.state = JobState::Done; }
                }
                Ok(s) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("extractor exited with {s}"));
                    }
                }
                Err(e) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{e}"));
                    }
                }
            }
        });
    }

    /// Extract a .zip natively via the bundled zip crate (no external tools).
    pub fn enqueue_extract_zip(&self, archive: PathBuf, dest_dir: PathBuf) {
        let label = format!("Extract {} \u{2192} {}", archive.display(), dest_dir.display());
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(true));
        let job = Job { label, total: 100, done: done.clone(), cancel, state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            match extract_zip_all(&archive, &dest_dir, Some(&done)) {
                Ok(_) => {
                    if let Ok(mut j) = arc.lock() {
                        j.done.store(100, Ordering::Relaxed);
                        j.state = JobState::Done;
                    }
                }
                Err(e) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}: {e}", archive.display()));
                    }
                }
            }
        });
    }

    /// Integrity-check a .zip / .7z without extracting.
    pub fn enqueue_check(&self, archive: PathBuf) {
        let label = format!("Check {}", archive.display());
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(true));
        let job = Job { label: label.clone(), total: 100, done: done.clone(), cancel, state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            match archive_check(&archive) {
                Ok(msg) => {
                    if let Ok(mut j) = arc.lock() {
                        j.done.store(100, Ordering::Relaxed);
                        j.state = JobState::Done;
                        j.label = format!("{label} \u{2713} ({msg})");
                    }
                }
                Err(e) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}", e));
                    }
                }
            }
        });
    }

    /// Extract a .7z natively via the sevenz-rust decoder (no external tools).
    pub fn enqueue_extract_7z(&self, archive: PathBuf, dest_dir: PathBuf) {
        let label = format!("Extract {} \u{2192} {}", archive.display(), dest_dir.display());
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(true));
        let job = Job { label, total: 100, done: done.clone(), cancel, state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            match extract_7z(&archive, &dest_dir) {
                Ok(_) => {
                    if let Ok(mut j) = arc.lock() {
                        j.done.store(100, Ordering::Relaxed);
                        j.state = JobState::Done;
                    }
                }
                Err(e) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}: {e}", archive.display()));
                    }
                }
            }
        });
    }

    /// Create a .zip from the given sources natively via the bundled zip crate.
    pub fn enqueue_compress(&self, sources: Vec<PathBuf>, dest_zip: PathBuf) {
        let label = format!("Compress {} item(s) \u{2192} {}", sources.len(), dest_zip.display());
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(true));
        let job = Job { label, total: 100, done: done.clone(), cancel, state: JobState::Running, error: None };
        let arc = self.register(job);
        self.spawn(move || {
            match create_zip(&sources, &dest_zip) {
                Ok(_) => {
                    if let Ok(mut j) = arc.lock() {
                        j.done.store(100, Ordering::Relaxed);
                        j.state = JobState::Done;
                    }
                }
                Err(e) => {
                    if let Ok(mut j) = arc.lock() {
                        j.state = JobState::Failed;
                        j.error = Some(format!("{}: {e}", dest_zip.display()));
                    }
                }
            }
        });
    }

    pub fn snapshot(&self) -> Vec<Arc<Mutex<Job>>> {
        self.core.jobs.lock().map(|j| j.clone()).unwrap_or_default()
    }


    pub fn prune_finished(&self) {
        self.core.jobs.lock().map(|mut j| {
            j.retain(|job| {
                matches!(job.lock().map(|x| x.state), Ok(JobState::Running))
                    .then_some(true).unwrap_or(matches!(job.lock().map(|x| x.state==JobState::Done), Ok(true)))
            });
        }).ok();
    }
}

fn zip_safe_options() -> zip::write::FileOptions {
    zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
}

/// Recursively add `path` (named `top`) under the archive at virtual dir `vdir`.
fn add_to_zip(
    zw: &mut zip::ZipWriter<std::fs::File>,
    vdir: &mut Vec<String>,
    path: &Path,
    top: &str,
) -> std::io::Result<()> {
    vdir.push(top.to_string());
    let rel = vdir.join("/");
    if path.is_dir() {
        zw.add_directory(format!("{rel}/"), zip_safe_options())?;
        for e in std::fs::read_dir(path)?.flatten() {
            add_to_zip(zw, vdir, &e.path(), &e.file_name().to_string_lossy())?;
        }
    } else {
        zw.start_file(rel, zip_safe_options())?;
        let mut fh = std::fs::File::open(path)?;
        std::io::copy(&mut fh, zw)?;
    }
    vdir.pop();
    Ok(())
}

/// Natively check a .zip / .7z for integrity (reads every entry).
pub fn archive_check(archive: &Path) -> std::io::Result<String> {
    let lower = archive.to_string_lossy().to_lowercase();
    if lower.ends_with(".zip") {
        let f = std::fs::File::open(archive)?;
        let mut z = zip::ZipArchive::new(f)
            .map_err(|e| std::io::Error::other(format!("zip: {e}")))?;
        let total = z.len();
        let mut n = 0u64;
        for i in 0..total {
            let mut e = z.by_index(i).map_err(|e| std::io::Error::other(format!("zip: {e}")))?;
            let mut buf = [0u8; 65536];
            loop {
                match e.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => return Err(std::io::Error::other(format!("zip: {e}"))),
                }
            }
            n += 1;
        }
        Ok(format!("{n} entries OK"))
    } else if lower.ends_with(".7z") {
        let mut rd = sevenz_rust::SevenZReader::open(archive, sevenz_rust::Password::empty())
            .map_err(|e| std::io::Error::other(format!("7z: {e}")))?;
        let mut n = 0u64;
        rd.for_each_entries(|_entry, mut rdr| {
            let mut buf = [0u8; 65536];
            loop {
                match rdr.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => return Err(sevenz_rust::Error::other(format!("read: {e}"))),
                }
            }
            n += 1;
            Ok(true)
        })
        .map_err(|e| std::io::Error::other(format!("7z: {e}")))?;
        Ok(format!("{n} entries OK"))
    } else {
        Ok("not a zip/7z (skipped)".into())
    }
}

/// Create a .zip containing `sources` (files and/or whole folder trees).
pub fn create_zip(sources: &[std::path::PathBuf], dest_zip: &Path) -> std::io::Result<()> {
    let f = std::fs::File::create(dest_zip)?;
    let mut zw = zip::ZipWriter::new(f);
    for src in sources {
        if src == dest_zip || (dest_zip.starts_with(src) && src.is_dir()) && src.is_dir() {
            continue; // never include the archive being written
        }
        let top = src.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into());
        add_to_zip(&mut zw, &mut Vec::new(), src, &top)?;
    }
    zw.finish()?;
    Ok(())
}

/// Natively extract every entry of a .zip into dest_dir (overwrites, zip-slip safe).
pub fn extract_zip_all(
    archive: &Path,
    dest_dir: &Path,
    progress: Option<&AtomicU64>,
) -> std::io::Result<u64> {
    let f = std::fs::File::open(archive)?;
    let mut z = zip::ZipArchive::new(f)
        .map_err(|e| std::io::Error::other(format!("zip: {e}")))?;
    let total = z.len().max(1);
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| std::io::Error::other(format!("zip: {e}")))?;
        let name = e.name().replace('\\', "/");
        let name = name.trim_start_matches('/');
        if name.split('/').any(|s| s == ".." || s.is_empty() && !name.contains('/')) {
            continue; // zip-slip guard
        }
        let out = dest_dir.join(name);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(par) = out.parent() { std::fs::create_dir_all(par)?; }
            let mut w = std::fs::File::create(&out)?;
            std::io::copy(&mut e, &mut w)?;
        }
        if let Some(p) = progress {
            p.store((i * 100 / total) as u64, Ordering::Relaxed);
        }
    }
    if let Some(p) = progress {
        p.store(100, Ordering::Relaxed);
    }
    Ok(z.len() as u64)
}

/// Natively extract a .7z via sevenz-rust (LZMA/LZMA2/BCJ etc.).
pub fn extract_7z(archive: &Path, dest_dir: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(dest_dir)?;
    sevenz_rust::decompress_file(archive, dest_dir)
        .map_err(|e| std::io::Error::other(format!("7z: {e}")))?;
    Ok(1)
}

fn transfer_one(
    src: &Path,
    dst: &Path,
    move_it: bool,
    done: &AtomicU64,
    cancel: &AtomicBool,
) -> std::io::Result<()> {
    use std::io::Read;
    use std::io::Write;
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            if cancel.load(Ordering::Relaxed) { break; }
            transfer_one(&e.path(), &dst.join(e.file_name()), move_it, done, cancel)?;
        }
        if move_it && !cancel.load(Ordering::Relaxed) {
            std::fs::remove_dir(src).ok();
        }
        return Ok(());
    }
    let mut inp = File::open(src)?;
    let mut out = File::create(dst)?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) { break; }
        let n = inp.read(&mut buf)?;
        if n == 0 { break; }
        out.write_all(&buf[..n])?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(out);
    if move_it && !cancel.load(Ordering::Relaxed) {
        std::fs::remove_file(src).ok();
    }
    Ok(())
}

fn dir_size_quick(p: &Path) -> Option<u64> {
    if p.is_file() { Some(std::fs::metadata(p).ok()?.len()) } else { None }
}

use std::fs::File;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait_done(jm: &JobManager, timeout: Duration) -> Option<bool> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            let snap = jm.snapshot();
            for j in &snap {
                if let Ok(j) = j.lock() {
                    match j.state {
                        JobState::Done => return Some(true),
                        JobState::Failed => return Some(false),
                        _ => {}
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn permanent_delete_removes_file_and_dir() {
        let tmp = std::env::temp_dir().join(format!("hd_deltest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.txt"), "x").unwrap();
        std::fs::write(tmp.join("sub/b.txt"), "y").unwrap();

        let jm = JobManager::new();
        jm.enqueue_delete(vec![
            tmp.join("a.txt"),
            tmp.join("sub"),
        ]);

        let res = wait_done(&jm, Duration::from_secs(10));
        if res != Some(true) {
            let snap = jm.snapshot();
            let errs: Vec<String> = snap.iter().filter_map(|j| j.lock().ok())
                .filter_map(|j| j.error.clone()).collect();
            panic!("delete not ok: {:?}; errors={:?}", res, errs);
        }
        assert!(!tmp.join("a.txt").exists(), "file still exists");
        assert!(!tmp.join("sub").exists(), "dir still exists");
        assert!(std::fs::read_dir(&tmp).map(|mut d| d.next().is_none()).unwrap_or(false),
            "parent dir not emptied");
        let _ = std::fs::remove_dir(&tmp);
    }

    #[test]
    fn permanent_delete_reports_missing_path_as_failed() {
        let jm = JobManager::new();
        jm.enqueue_delete(vec![PathBuf::from("/nonexistent/__hd_no_such__")]);
        let res = wait_done(&jm, Duration::from_secs(10));
        assert_eq!(res, Some(false), "missing path should fail the job, not hang");
    }

    #[test]
    fn zip_compress_extract_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("hd_ziptest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp.join("tree/leaf")).unwrap();
        std::fs::write(tmp.join("top.txt"), "hello top").unwrap();
        std::fs::write(tmp.join("tree/leaf/inner.txt"), "hello inner").unwrap();

        let zip_path = tmp.join("out.zip");
        create_zip(&[tmp.join("top.txt"), tmp.join("tree")], &zip_path).unwrap();

        let dest = tmp.join("out");
        let n = extract_zip_all(&zip_path, &dest, None).unwrap();
        assert!(n >= 3, "expected at least 3 entries, got {n}");
        assert_eq!(std::fs::read_to_string(dest.join("top.txt")).unwrap(), "hello top");
        assert_eq!(
            std::fs::read_to_string(dest.join("tree/leaf/inner.txt")).unwrap(),
            "hello inner"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn zip_extract_rejects_path_traversal() {
        let tmp = std::env::temp_dir().join(format!("hd_ezztest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // hand-craft a zip whose entry points to ../escape.txt
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file("../escape.txt", zip_safe_options()).unwrap();
        std::io::Write::write_all(&mut writer, b"pwn").unwrap();
        let buf = writer.finish().unwrap().into_inner();
        let zpath = tmp.join("evil.zip");
        std::fs::write(&zpath, &buf).unwrap();
        let dest = tmp.join("d");
        std::fs::create_dir_all(&dest).unwrap();

        extract_zip_all(&zpath, &dest, None).unwrap();
        assert!(!dest.join("..").join("escape.txt").exists(), "zip-slip let file escape!");
        assert!(!dest.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sevenz_garbage_input_fails_cleanly() {
        let tmp = std::env::temp_dir().join(format!("hd_7ztest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let bad = tmp.join("bad.7z");
        std::fs::write(&bad, b"not a real 7z archive at all").unwrap();
        let r = extract_7z(&bad, &tmp.join("out"));
        assert!(r.is_err(), "garbage input must error, not hang");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn archive_check_reports_ok_and_catches_corruption() {
        let tmp = std::env::temp_dir().join(format!("hd_checktest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "hello a").unwrap();
        std::fs::write(tmp.join("b.txt"), "hello b").unwrap();
        let zip_path = tmp.join("ok.zip");
        create_zip(&[tmp.join("a.txt"), tmp.join("b.txt")], &zip_path).unwrap();
        let msg = archive_check(&zip_path).unwrap_or_else(|e| panic!("check failed: {e}"));
        assert!(msg.contains("2 entries OK"), "got: {msg}");
        // corrupt a byte in the stream and expect an error
        let mut bytes = std::fs::read(&zip_path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        let bad_path = tmp.join("bad.zip");
        std::fs::write(&bad_path, &bytes).unwrap();
        assert!(archive_check(&bad_path).is_err(), "corrupt zip must fail the check");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
