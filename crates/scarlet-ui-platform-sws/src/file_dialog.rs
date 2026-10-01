//! Existing Files/sbus request/reply adapter; no invented SWS wire messages.
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, Ordering};
use sbus_client::{Argument, Connection, Message};
use scarlet_desktop_config::*;
use scarlet_ui_core::file_dialog::*;
use std::sync::Mutex;

struct Session {
    result: Arc<Mutex<Option<FileDialogResult>>>,
    abandon: Arc<AtomicBool>,
}
impl FileDialogSession for Session {
    fn poll(&mut self) -> Option<FileDialogResult> {
        {
            #[cfg(feature = "std")]
            let mut result = self.result.lock().unwrap();
            #[cfg(not(feature = "std"))]
            let mut result = self.result.lock();
            result.take()
        }
    }
    fn cancel(&mut self) {
        self.abandon.store(true, Ordering::Release);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn begin(options: &FileDialog) -> Result<Box<dyn FileDialogSession>, FileDialogError> {
    options.validate()?;
    if options.mode == FileDialogMode::OpenMultiple {
        return Err(FileDialogError::Unsupported(
            "Files returns only one path despite allow_multiple".into(),
        ));
    }
    if !options.filters.is_empty() {
        return Err(FileDialogError::Unsupported(
            "Files cannot enforce arbitrary extension filters".into(),
        ));
    }
    let options = options.clone();
    let result = Arc::new(Mutex::new(None));
    let abandon = Arc::new(AtomicBool::new(false));
    let output = result.clone();
    let stopped = abandon.clone();
    std::thread::spawn(move || {
        let value = request(options, &stopped);
        #[cfg(feature = "std")]
        let mut result = output.lock().unwrap();
        #[cfg(not(feature = "std"))]
        let mut result = output.lock();
        *result = Some(value);
    });
    Ok(Box::new(Session { result, abandon }))
}
fn request(options: FileDialog, abandon: &AtomicBool) -> FileDialogResult {
    if abandon.load(Ordering::Acquire) {
        return Ok(FileDialogOutcome::Cancelled);
    }
    let directory = options
        .initial_directory
        .as_ref()
        .map(|p| {
            p.to_str()
                .ok_or_else(|| FileDialogError::InvalidOptions("Files requires UTF-8 paths".into()))
        })
        .transpose()?
        .unwrap_or("")
        .to_string();
    let (method, args): (&str, Vec<Argument>) = match options.mode {
        FileDialogMode::Open => (
            DESKTOP_FILE_MANAGER_OPEN_FILE_METHOD,
            vec![
                Argument::String(options.title),
                Argument::String(directory),
                Argument::String(String::new()),
                Argument::Boolean(false),
                Argument::Boolean(false),
            ],
        ),
        FileDialogMode::Save => (
            DESKTOP_FILE_MANAGER_SAVE_FILE_METHOD,
            vec![
                Argument::String(options.title),
                Argument::String(directory),
                Argument::String(options.default_name.unwrap_or_default()),
                Argument::String(String::new()),
            ],
        ),
        _ => {
            return Err(FileDialogError::Unsupported(
                "unsupported Files operation".into(),
            ));
        }
    };
    let platform = |e| FileDialogError::Platform(format!("Files/sbus: {e:?}"));
    // One connection keeps any Response arriving before MethodReturn in the
    // client's pending queue. Do not resend an ambiguous timed-out request.
    let mut connection = Connection::connect().map_err(platform)?;
    let reply = connection
        .call_method_timeout(
            DESKTOP_FILE_MANAGER_BUS_NAME,
            DESKTOP_FILE_MANAGER_OBJECT_PATH,
            DESKTOP_FILE_MANAGER_INTERFACE,
            method,
            args,
            2_000,
        )
        .map_err(platform)?;
    let Some(Argument::String(id)) = reply.first() else {
        return Err(FileDialogError::Platform(
            "Files returned no request id".into(),
        ));
    };
    if id.is_empty() {
        return Err(FileDialogError::Platform(
            "Files returned an empty request id".into(),
        ));
    }
    let id = id.clone();
    // The service has no remote cancel or caller-window field. Local cancellation
    // abandons this receipt and releases the worker; the remote picker may remain.
    // A missing Response (child launch/death) is an error, never user cancellation.
    let started = monotonic_ns();
    loop {
        if abandon.load(Ordering::Acquire) {
            return Ok(FileDialogOutcome::Cancelled);
        }
        if monotonic_ns().saturating_sub(started) > 600_000_000_000 {
            return Err(FileDialogError::Platform(
                "Files response deadline exceeded".into(),
            ));
        }
        let Some(message) = connection.receive_message_timeout(100).map_err(platform)? else {
            continue;
        };
        let Message::Signal {
            sender,
            path,
            interface,
            signal,
            args,
        } = message
        else {
            continue;
        };
        if sender != DESKTOP_FILE_MANAGER_BUS_NAME
            || path != DESKTOP_FILE_MANAGER_OBJECT_PATH
            || interface != DESKTOP_FILE_MANAGER_INTERFACE
            || signal != DESKTOP_FILE_MANAGER_RESPONSE_SIGNAL
        {
            continue;
        }
        if !matches!(args.first(), Some(Argument::String(response_id)) if response_id == &id) {
            continue;
        }
        let (Some(Argument::Boolean(accepted)), Some(Argument::String(path))) =
            (args.get(1), args.get(2))
        else {
            return Err(FileDialogError::Platform("malformed Files response".into()));
        };
        if !accepted {
            return Ok(FileDialogOutcome::Cancelled);
        }
        let path = FileDialogPath::from(path.clone());
        if !path.is_absolute() {
            return Err(FileDialogError::Platform(
                "Files returned an empty or relative path".into(),
            ));
        }
        return Ok(FileDialogOutcome::Selected(vec![path]));
    }
}

fn monotonic_ns() -> u64 {
    #[cfg(feature = "std")]
    {
        use std::sync::OnceLock;
        use std::time::Instant;
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        EPOCH
            .get_or_init(Instant::now)
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128) as u64
    }
    #[cfg(not(feature = "std"))]
    {
        use std::syscall::{Syscall, syscall0};
        // SAFETY: this clock query has no arguments or userspace memory effects.
        unsafe { syscall0(Syscall::MonotonicTime) as u64 }
    }
}
