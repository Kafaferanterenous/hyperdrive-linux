//! HyperDrive - filesystem core: listing, sorting, history, formatting.
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Created,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Modified => "Modified",
            SortKey::Created => "Created",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64, // unix seconds
    pub created: u64,  // unix seconds (btime where filesystem provides it)
    pub tag_color: Option<String>,
}

pub fn secs_of(t: &std::io::Result<std::time::SystemTime>) -> u64 {
    t.as_ref().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn secs_of_created(md: &std::fs::Metadata) -> u64 {
    let c = secs_of(&md.created());
    let m = secs_of(&md.modified());
    c.max(m)
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> u64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Split a virtual archive path: returns (real_zip, inner_dir_opt).
fn split_zip_virtual(dir: &Path) -> Option<(PathBuf, String)> {
    let s = dir.to_string_lossy();
    let idx = s.find("!")?;
    let real = &s[..idx];
    if !real.to_lowercase().ends_with(".zip") {
        return None;
    }
    if !Path::new(real).is_file() {
        return None;
    }
    Some((PathBuf::from(real), s[idx + 1..].trim_matches('/').to_string()))
}

/// List a directory inside a .zip (read-only virtual filesystem).
fn list_zip_virtual(zip_path: &Path, inner: &str) -> std::io::Result<Vec<Entry>> {
    use std::collections::BTreeSet;
    let f = File::open(zip_path)?;
    let mut arch = zip::ZipArchive::new(f)
        .map_err(|e| std::io::Error::other(format!("zip: {e}")))?;
    let prefix = if inner.is_empty() {
        String::new()
    } else {
        format!("{}/", inner.trim_end_matches('/'))
    };
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<Entry> = Vec::new();
    for i in 0..arch.len() {
        let zf = arch.by_index(i).map_err(io_err)?;
        let name = zf.name().replace('\\', "/");
        if !name.starts_with(&prefix) || name == prefix {
            continue;
        }
        let rest = &name[prefix.len()..];
        if rest.is_empty() {
            continue;
        }
        match rest.find('/') {
            None => {
                // direct file
                out.push(Entry {
                    name: rest.to_string(),
                    path: PathBuf::from(format!("{}!{}/{}", zip_path.display(), inner, rest)),
                    is_dir: false,
                    size: zf.size(),
                    modified: zip_dt_to_secs(zf.last_modified()),
                    created: zip_dt_to_secs(zf.last_modified()),
                    tag_color: None,
                });
            }
            Some(slash) => {
                let d = rest[..slash].to_string();
                if dirs.insert(d.clone()) {
                    out.push(Entry {
                        name: d,
                        path: PathBuf::from(format!(
                            "{}!{}/{}",
                            zip_path.display(),
                            inner,
                            &rest[..slash]
                        )),
                        is_dir: true,
                        size: 0,
                        modified: zip_dt_to_secs(zf.last_modified()),
                        created: zip_dt_to_secs(zf.last_modified()),
                        tag_color: None,
                    });
                }
            }
        }
    }
    Ok(out)
}


/// zip::DateTime (DOS time) -> unix seconds.
fn zip_dt_to_secs(dt: zip::DateTime) -> u64 {
    use std::time::{Duration, UNIX_EPOCH};
    // days from civil for the DOS date parts
    let y = dt.year() as i64;
    let m = dt.month() as i64;
    let d = dt.day() as i64;
    let z = if m <= 2 { y - 1 } else { y };
    let era = if z >= 0 { z } else { z - 399 } / 400;
    let yoe = z - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400
        + dt.hour() as i64 * 3600
        + dt.minute() as i64 * 60
        + dt.second() as i64;
    (UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64))
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

pub fn extract_zip_entry(virtual_path: &Path) -> std::io::Result<PathBuf> {
    let s = virtual_path.to_string_lossy();
    let bang = s.find("!").ok_or_else(|| io_err("not an archive path"))?;
    let real = PathBuf::from(&s[..bang]);
    let inner = s[bang + 1..].to_string();
    let f = File::open(&real)?;
    let mut arch = zip::ZipArchive::new(f).map_err(io_err)?;
    for i in 0..arch.len() {
        let mut zf = arch.by_index(i).map_err(io_err)?;
        if zf.name().replace('\\', "/") == inner {
            let base = std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache/hyperdrive/zip"))
                .unwrap_or_else(|_| PathBuf::from("/tmp/hyperdrive_zip"));
            std::fs::create_dir_all(&base)?;
            let out_name = inner.rsplit('/').next().unwrap_or("file").to_string();
            let out = base.join(out_name);
            let mut w = File::create(&out)?;
            std::io::copy(&mut zf, &mut w)?;
            return Ok(out);
        }
    }
    Err(io_err("entry not found in archive"))
}

pub fn list_dir(dir: &Path) -> std::io::Result<Vec<Entry>> {
    if let Some((zip_path, inner)) = split_zip_virtual(dir) {
        return list_zip_virtual(&zip_path, &inner);
    }
    let mut out = Vec::new();
    for rd in fs::read_dir(dir)? {
        let Ok(de) = rd else { continue };
        let Ok(md) = de.metadata() else { continue };
        let name = de.file_name().to_string_lossy().into_owned();
        let is_dir = md.is_dir();
        out.push(Entry {
            name,
            path: de.path(),
            is_dir,
            size: if is_dir { 0 } else { md.len() },
            modified: secs(md.modified()),
            created: secs(md.created()).max(secs(md.modified())),
            tag_color: None,
        });
    }
    Ok(out)
}

pub fn sort_entries(v: &mut [Entry], key: SortKey, ascending: bool) {
    v.sort_by(|a, b| {
        // Directories first, always.
        match (a.is_dir, b.is_dir) {
            (true, false) => return std::cmp::Ordering::Less,
            (false, true) => return std::cmp::Ordering::Greater,
            _ => {}
        }
        let ord = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Created => a.created.cmp(&b.created),
        };
        if ascending {
            ord.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        } else {
            ord.reverse().then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }
    });
}

pub struct History {
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    pub current: PathBuf,
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// Tree sidebar roots: Home, filesystem root, removable/mounted drives.
///
/// On Linux, mounts are enumerated from `/proc/self/mountinfo` rather than a
/// hardcoded root, so any mounted device or network share (wherever the admin
/// placed it) shows up consistently across systems: `/media/<user>`,
/// `/run/media/<user>`, `/mnt`, Docker bind mounts, etc.
pub fn drive_roots() -> Vec<(String, PathBuf)> {
    let mut v = vec![("\u{1F3E0} Home".to_string(), home_dir())];
    v.push(("\u{1F4BE} Filesystem".to_string(), PathBuf::from("/")));
    #[cfg(target_os = "linux")]
    {
        let mut seen = std::collections::HashSet::new();
        if let Ok(mi) = fs::read_to_string("/proc/self/mountinfo") {
            for line in mi.lines() {
                let f: Vec<&str> = line.split(' ').collect();
                if f.len() > 4 {
                    // mountinfo: 0 1 2 3 mountpoint...
                    let mnt = f[4];
                    if is_registered_root(mnt) && std::path::Path::new(mnt).is_dir() {
                        let root_name = mnt.rsplit('/').next().unwrap_or(mnt);
                        let label = format!(
                            "\u{1F4BD} {}",
                            decode_mount_label(mnt, root_name)
                        );
                        if seen.insert(label.clone()) {
                            v.push((label, PathBuf::from(mnt)));
                        }
                    }
                }
            }
        }
    }
    v
}

#[cfg(target_os = "linux")]
/// True for mount points under the conventional placeable roots.
fn is_registered_root(p: &str) -> bool {
    p.starts_with("/media/")
        || p.starts_with("/run/media/")
        || p.starts_with("/mnt/")
        || p == "/mnt"
}

#[cfg(target_os = "linux")]
/// Prefer showing `/media/<user>/<share>` style labels (short) while
/// keeping the path useful. Returns the last two path components joined
/// by " / " when the mount is nested more than two levels deep.
fn decode_mount_label(mnt: &str, root_name: &str) -> String {
    let parts: Vec<&str> = mnt.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 3 && (mnt.starts_with("/media/") || mnt.starts_with("/run/media/")) {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        root_name.to_string()
    }
}

impl History {
    pub fn with_start(start: PathBuf) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            current: start,
        }
    }

    /// Record a NEW navigation (not a back/forward traversal).
    pub fn go(&mut self, path: PathBuf) {
        if path != self.current {
            self.back.push(self.current.clone());
            self.forward.clear();
            self.current = path;
        }
    }

    /// Traverse back; returns the directory now current.
    pub fn back(&mut self) -> Option<PathBuf> {
        let prev = self.back.pop()?;
        let cur = std::mem::replace(&mut self.current, prev);
        self.forward.push(cur);
        Some(self.current.clone())
    }

    /// Traverse forward; returns the directory now current.
    pub fn forward(&mut self) -> Option<PathBuf> {
        let next = self.forward.pop()?;
        let cur = std::mem::replace(&mut self.current, next);
        self.back.push(cur);
        Some(self.current.clone())
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.1} GB", b / GB)
    }
}

/// Civil date from unix seconds (Howard Hinnant algorithm, no chrono dep).
pub fn format_time(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return "-".to_string();
    }
    let days = (unix_secs / 86_400) as i64;
    let rem = (unix_secs % 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{min:02}")
}

pub fn open_with_system(path: &Path) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn();
    }
}

// ---------------------------------------------------------------- dir sizes

/// Background recursive directory size calculator with cache.
/// Sizes appear in the Size column once computed ("..." while working).
pub struct DirSizes {
    map: std::sync::Arc<std::sync::Mutex<HashMap<PathBuf, DirSizeState>>>,
}

#[derive(Clone, Copy, Debug)]
pub enum DirSizeState {
    Computing,
    Done(u64),
}

impl Default for DirSizes {
    fn default() -> Self {
        Self::new()
    }
}

impl DirSizes {
    pub fn new() -> Self {
        Self {
            map: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Known size, or None if computing/unknown. Spawns a worker on first request.
    pub fn get(&self, dir: &Path) -> Option<u64> {
        match self.map.lock().ok()?.get(dir) {
            Some(DirSizeState::Done(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn is_computing(&self, dir: &Path) -> bool {
        self.map
            .lock()
            .map(|m| matches!(m.get(dir), Some(DirSizeState::Computing)))
            .unwrap_or(false)
    }

    /// Ensure a computation is queued for this dir (idempotent).
    pub fn request(&self, dir: &Path, ctx: egui_ctx::Ctx) {
        let Ok(mut m) = self.map.lock() else { return };
        if m.contains_key(dir) {
            return;
        }
        m.insert(dir.to_path_buf(), DirSizeState::Computing);
        drop(m);

        let key = dir.to_path_buf();
        let map = std::sync::Arc::clone(&self.map);
        std::thread::spawn(move || {
            let total = compute_dir_size(&key);
            if let Ok(mut m) = map.lock() {
                m.insert(key.clone(), DirSizeState::Done(total));
            }
            ctx.request_repaint();
        });
    }
}

/// Narrow alias so browser.rs does not need a hard egui dependency in tests.
pub mod egui_ctx {
    #[derive(Clone)]
    pub struct Ctx(pub eframe::egui::Context);
    impl Ctx {
        pub fn request_repaint(&self) {
            self.0.request_repaint();
        }
    }
}

fn compute_dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            // Skip symlinks entirely (no loops, no double counting).
            if md.is_symlink() {
                continue;
            }
            if md.is_dir() {
                stack.push(e.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_format() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert!(format_size(3u64 * 1024 * 1024 * 1024).ends_with("GB"));
    }

    #[test]
    fn time_epoch_zero_is_dash() {
        assert_eq!(format_time(0), "-");
        let s = format_time(86_400); // 1970-01-02 00:00 UTC
        assert!(s.starts_with("1970-01"), "got {s}");
    }

    #[test]
    fn drive_roots_includes_conventional_mounts() {
        let roots = drive_roots();
        assert!(roots.iter().any(|(_, p)| *p == PathBuf::from("/")), "missing filesystem root");
        assert!(roots.iter().any(|(_, p)| *p == home_dir()), "missing home root");
        // Any mountinfo-reported mount under /media, /run/media or /mnt must
        // be present so shares show up regardless of deployment convention.
        if let Ok(mi) = fs::read_to_string("/proc/self/mountinfo") {
            for line in mi.lines() {
                let f: Vec<&str> = line.split(' ').collect();
                if f.len() > 4 {
                    let mnt = f[4];
                    if (mnt.starts_with("/media/") || mnt.starts_with("/run/media/") || mnt.starts_with("/mnt"))
                        && std::path::Path::new(mnt).is_dir()
                    {
                        assert!(
                            roots.iter().any(|(_, p)| *p == PathBuf::from(mnt)),
                            "mount {} missing from drive_roots",
                            mnt
                        );
                    }
                }
            }
        }
        // Sanity: no duplicate labels.
        let labels: Vec<&String> = roots.iter().map(|(l, _)| l).collect();
        let uniq: std::collections::HashSet<&String> = labels.iter().copied().collect();
        assert_eq!(labels.len(), uniq.len(), "duplicate drive labels");
    }

    #[test]
    fn dirs_sort_first_then_name() {
        let mk = |n: &str, d: bool| Entry {
            name: n.into(), path: PathBuf::from(n), is_dir: d,
            size: 1, modified: 0, created: 0, tag_color: None,
        };
        let mut v = vec![mk("zeta.txt", false), mk("alpha", true), mk("beta", false)];
        sort_entries(&mut v, SortKey::Name, true);
        assert!(v[0].is_dir && v[0].name == "alpha");
        assert!(!v[1].is_dir && v[1].name == "beta");
        assert!(!v[2].is_dir && v[2].name == "zeta.txt");
    }
}
