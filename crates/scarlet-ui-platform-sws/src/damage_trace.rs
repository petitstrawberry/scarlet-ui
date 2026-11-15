//! Opt-in, bounded native damage provenance. Never writes to the console.
//!
//! Set SCARLET_UI_DAMAGE_TRACE=1 (or a path prefix) before launching an app.
//! Each process appends to <prefix>-<pid>.tsv. This is diagnostic evidence,
//! not a frame-time measurement: records are buffered and flushed once per second.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_ROWS: usize = 512;
const MAX_BYTES: usize = 512 * 1024;
const MAX_STREAMS: usize = 32;
const MAX_ROW_BYTES: usize = 4096;

static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();
static STOPPED: AtomicBool = AtomicBool::new(false);
static FLUSHER: OnceLock<()> = OnceLock::new();

struct Trace {
    file: BufWriter<File>,
    epoch: Instant,
    sequences: BTreeMap<(u32, &'static str), u64>,
    rows: usize,
    bytes: usize,
    scratch: String,
    dirty: bool,
}

impl Trace {
    fn open() -> Option<Self> {
        let configured = std::env::var("SCARLET_UI_DAMAGE_TRACE").ok().or_else(|| {
            std::env::args()
                .any(|arg| arg == "--trace-ui-damage")
                .then(|| String::from("1"))
        })?;
        if configured.is_empty() || configured == "0" {
            return None;
        }
        let prefix = if configured == "1" {
            "/tmp/ui-damage"
        } else {
            &configured
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{prefix}-{}.tsv", std::process::id()))
            .ok()?;
        let bytes = usize::try_from(file.metadata().ok()?.len()).ok()?;
        if bytes >= MAX_BYTES {
            return None;
        }
        Some(Self::new(file, bytes))
    }

    fn new(file: File, bytes: usize) -> Self {
        Self {
            file: BufWriter::with_capacity(64 * 1024, file),
            epoch: Instant::now(),
            sequences: BTreeMap::new(),
            rows: 0,
            bytes,
            scratch: String::with_capacity(MAX_ROW_BYTES),
            dirty: false,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.dirty {
            self.file.flush()?;
            self.dirty = false;
        }
        Ok(())
    }

    // Return true on exhaustion or I/O failure. No diagnostic can fail a frame.
    fn record(
        &mut self,
        window: u32,
        stage: &'static str,
        serial: Option<u64>,
        force: bool,
        add: impl FnOnce(&mut String),
    ) -> bool {
        if self.rows >= MAX_ROWS || self.bytes >= MAX_BYTES {
            let _ = self.flush();
            return true;
        }
        let key = (window, stage);
        if self.sequences.len() >= MAX_STREAMS && !self.sequences.contains_key(&key) {
            return false;
        }
        let sequence = self.sequences.entry(key).or_default();
        *sequence = sequence.saturating_add(1);
        let sequence = *sequence;
        // Retain startup/reuse evidence and sample the rest of a normal drag.
        // Backend enqueue and worker publication sample the SAME commit serial,
        // even when the worker supersedes intervening completed frames.
        let sample = serial.unwrap_or(sequence);
        if !force && sample > 32 && sample % 17 != 0 {
            return false;
        }
        self.scratch.clear();
        let _ = write!(
            self.scratch,
            "v1\twindow={window}\tstage={stage}\tseq={sequence}\tns={}",
            self.epoch.elapsed().as_nanos()
        );
        add(&mut self.scratch);
        self.scratch.push('\n');
        if self.scratch.len() > MAX_ROW_BYTES
            || self.bytes.saturating_add(self.scratch.len()) > MAX_BYTES
        {
            let _ = self.flush();
            return true;
        }
        if self.file.write_all(self.scratch.as_bytes()).is_err() {
            return true;
        }
        self.dirty = true;
        self.bytes += self.scratch.len();
        self.rows += 1;
        if self.rows >= MAX_ROWS {
            let _ = self.flush();
            return true;
        }
        false
    }
}

pub(super) fn enabled() -> bool {
    let enabled = TRACE
        .get_or_init(|| Trace::open().map(Mutex::new))
        .is_some();
    if enabled {
        FLUSHER.get_or_init(|| {
            if std::thread::Builder::new()
                .name("ui-damage-flush".into())
                .spawn(|| {
                    let started = Instant::now();
                    loop {
                        std::thread::sleep(Duration::from_secs(1));
                        if STOPPED.load(Ordering::Relaxed) {
                            break;
                        }
                        let Some(Some(trace)) = TRACE.get() else {
                            break;
                        };
                        let mut trace = trace.lock().unwrap_or_else(|e| e.into_inner());
                        if trace.flush().is_err() || started.elapsed() >= Duration::from_secs(90) {
                            STOPPED.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                })
                .is_err()
            {
                STOPPED.store(true, Ordering::Relaxed);
            }
        });
    }
    enabled && !STOPPED.load(Ordering::Relaxed)
}

pub(super) fn record(
    window: u32,
    stage: &'static str,
    serial: Option<u64>,
    force: bool,
    add: impl FnOnce(&mut String),
) {
    if STOPPED.load(Ordering::Relaxed) {
        return;
    }
    let Some(Some(trace)) = TRACE.get() else {
        return;
    };
    if trace
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(window, stage, serial, force, add)
    {
        STOPPED.store(true, Ordering::Relaxed);
    }
}

pub(super) fn rects(
    out: &mut String,
    name: &str,
    rects: impl IntoIterator<Item = (i64, i64, u32, u32)>,
) {
    let _ = write!(out, "\t{name}=[");
    for (index, (x, y, w, h)) in rects.into_iter().enumerate() {
        if index == 16 {
            out.push_str("...");
            break;
        }
        if index != 0 {
            out.push(';');
        }
        let _ = write!(out, "{x},{y},{w},{h}");
    }
    out.push(']');
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_file_trace_samples_reuse_and_stops_before_formatting_past_its_cap() {
        let path =
            std::env::temp_dir().join(format!("scarlet-damage-trace-test-{}", std::process::id()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut trace = Trace::new(file, 0);
        for serial in 1..=16_000u64 {
            let stopped = trace.record(7, "backend", Some(serial), false, |out| {
                let _ = write!(out, "\tserial={serial}\tslot={}", (serial - 1) % 4);
                rects(out, "damage", [(64, 483, 284, 48)]);
            });
            if stopped {
                break;
            }
        }
        assert_eq!(trace.rows, MAX_ROWS);
        assert!(trace.record(7, "backend", None, true, |_| panic!(
            "exhausted trace formatted another row"
        )));
        drop(trace);
        let data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(data.lines().count(), MAX_ROWS);
        assert!(data.contains("serial=32\tslot=3"));
        assert!(data.contains("serial=51\tslot=2"));
        assert!(!data.contains("serial=33\t"));
        // A prime sampling stride visits every cyclic four-image slot.
        for slot in 0..4 {
            let serial = 34 + slot * 17;
            assert!(data.contains(&format!("serial={serial}\tslot={}", (serial - 1) % 4)));
        }
        assert!(data.len() < MAX_BYTES);
        std::fs::remove_file(path).unwrap();
    }
}
