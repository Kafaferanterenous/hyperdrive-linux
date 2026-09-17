//! HyperDrive - embedded audio playback (mp3/wav/ogg/flac/m4a).
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

// ---------------- libopenmpt FFI (tracker modules: XM/S3M/MOD/IT/MPTM) ----
extern "C" {
    #[link_name = "openmpt_module_create_from_memory"]
    fn om_create(
        data: *const u8,
        size: usize,
        logfunc: *const core::ffi::c_void,
        user: *const core::ffi::c_void,
        ctls: *const core::ffi::c_void,
    ) -> *mut core::ffi::c_void;
    #[link_name = "openmpt_module_destroy"]
    fn om_destroy(mod_: *mut core::ffi::c_void);
    #[link_name = "openmpt_module_read_interleaved_stereo"]
    fn om_read_stereo16(
        mod_: *mut core::ffi::c_void,
        samplerate: i32,
        frames: usize,
        interleaved: *mut i16,
    ) -> usize;
}

/// Decode a tracker file fully to interleaved stereo i16 @48kHz.
fn render_tracker(path: &Path) -> Result<Vec<i16>, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    unsafe {
        let m = om_create(bytes.as_ptr(), bytes.len(), std::ptr::null(), std::ptr::null(), std::ptr::null());
        if m.is_null() {
            return Err(format!("{}: not a supported module", path.display()));
        }
        const RATE: i32 = 48_000;
        let chunk_frames = 48_000usize; // 1s per chunk
        let mut all: Vec<i16> = Vec::new();
        let mut buf = vec![0i16; chunk_frames * 2];
        // Hard cap ~10 minutes to avoid pathological files.
        let max_frames = RATE as usize * 60 * 10;
        loop {
            let got = om_read_stereo16(m, RATE, chunk_frames, buf.as_mut_ptr());
            if got == 0 || all.len() / 2 >= max_frames {
                break;
            }
            all.extend_from_slice(&buf[..got * 2]);
        }
        om_destroy(m);
        Ok(all)
    }
}

pub fn is_tracker(path: &Path) -> bool {
    matches!(
        path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .as_deref(),
        Some("xm") | Some("s3m") | Some("mod") | Some("it") | Some("mptm")
    )
}

pub struct AudioPlayer {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    sink: Option<Sink>,
    pub path: Option<PathBuf>,
    mod_rx: Option<std::sync::mpsc::Receiver<Result<Vec<i16>, String>>>,
    pub last_err: Option<String>,
    /// True while a tracker module renders in the background (no sink yet).
    pub rendering: bool,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlayer {
    pub fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Self {
                _stream: Some(stream),
                handle: Some(handle),
                sink: None,
                path: None,
                mod_rx: None,
                last_err: None,
                rendering: false,
            },
            Err(e) => {
                eprintln!("audio init failed: {e}");
                Self {
                    _stream: None,
                    handle: None,
                    sink: None,
                    path: None,
                    mod_rx: None,
                    last_err: None,
                    rendering: false,
                }
            }
        }
    }

    /// Poll finished background module renders; start sink when ready.
    pub fn poll(&mut self) {
        if self.mod_rx.is_none() {
            return;
        }
        let res = self.mod_rx.as_ref().unwrap().try_recv().ok();
        if let Some(res) = res {
            self.mod_rx = None;
            self.rendering = false;
            match res {
                Ok(data) => {
                    if let Some(handle) = self.handle.clone() {
                        match Sink::try_new(&handle) {
                            Ok(sink) => {
                                sink.append(rodio::buffer::SamplesBuffer::new(2, 48_000, data));
                                self.sink = Some(sink);
                            }
                            Err(e) => self.last_err = Some(format!("sink: {e}")),
                        }
                    }
                }
                Err(e) => {
                    self.last_err = Some(e);
                    self.path = None;
                }
            }
        }
    }

    /// Start (or replace) playback of one file. Trackers render async first.
    pub fn play(&mut self, path: &PathBuf) -> Result<(), String> {
        if is_tracker(path) {
            self.stop();
            self.path = Some(path.clone());
            self.last_err = None;
            self.rendering = true;
            let p = path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.mod_rx = Some(rx);
            std::thread::spawn(move || {
                let res = render_tracker(&p);
                let _ = tx.send(res);
            });
            return Ok(());
        }
        let handle = self
            .handle
            .clone()
            .ok_or_else(|| "no audio output device".to_string())?;

        // Stop and drop previous sink first (releases the file handle).
        self.stop();

        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("decode failed: {e}"))?;
        let sink = Sink::try_new(&handle).map_err(|e| format!("sink failed: {e}"))?;
        sink.append(source);
        self.sink = Some(sink);
        self.path = Some(path.clone());
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.path = None;
        self.rendering = false;
    }

    pub fn toggle_pause(&self) {
        if let Some(sink) = &self.sink {
            if sink.is_paused() {
                sink.play();
            } else {
                sink.pause();
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.sink.as_ref().map(|s| s.is_paused()).unwrap_or(false)
    }

    pub fn set_volume(&self, v: f32) {
        if let Some(sink) = &self.sink {
            sink.set_volume(v.clamp(0.0, 1.5));
        }
    }

    /// Finished playing on its own?
    pub fn finished(&self) -> bool {
        if self.rendering {
            return false; // module still decoding - keep the bar alive
        }
        match (&self.sink, &self.path) {
            (Some(s), Some(_)) => s.empty(),
            (None, Some(_)) => true,
            _ => false,
        }
    }

    pub fn playing_file(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }
}
