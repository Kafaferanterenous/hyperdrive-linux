//! HyperDrive - background file-operation job manager.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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
}
