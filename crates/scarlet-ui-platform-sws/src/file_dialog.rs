//! Existing Files/sbus request/reply adapter; no invented SWS wire messages.
use alloc::{boxed::Box, format, string::ToString, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use sbus_client::{Argument, Connection, Error as BusError, Message};
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
    super::file_dialog_policy::native_filter(options, true)?;
    let options = options.clone();
    let result = Arc::new(Mutex::new(None));
    let abandon = Arc::new(AtomicBool::new(false));
    let output = result.clone();
    let stopped = abandon.clone();
    std::thread::Builder::new()
        .name("files-picker".into())
        .spawn(move || {
            let value = request(options, &stopped);
            #[cfg(feature = "std")]
            let mut result = output.lock().unwrap();
            #[cfg(not(feature = "std"))]
            let mut result = output.lock();
            *result = Some(value);
        })
        .map_err(|e| FileDialogError::Platform(format!("Files worker: {e:?}")))?;
    Ok(Box::new(Session { result, abandon }))
}
fn request(options: FileDialog, abandon: &AtomicBool) -> FileDialogResult {
    if abandon.load(Ordering::Acquire) {
        return Ok(FileDialogOutcome::Cancelled);
    }
    let platform = |e| FileDialogError::Platform(format!("Files/sbus: {e:?}"));
    let mut connection = Connection::connect().map_err(platform)?;
    let extensions_supported = if options.filters.is_empty() {
        false
    } else {
        match call_files(&mut connection, super::file_dialog_policy::CAPABILITIES_METHOD, vec![], abandon) {
            Ok(capabilities) => capabilities.iter().any(|capability| {
                matches!(capability, Argument::String(value) if value == super::file_dialog_policy::EXTENSION_CAPABILITY)
            }),
            // Older Files explicitly rejects this read-only query; no picker was created.
            Err(BusError::MethodFailed(message)) if message == "Unknown FileManager method" => false,
            Err(error) => return Err(platform(error)),
        }
    };
    let filter = super::file_dialog_policy::native_filter(&options, extensions_supported)?;
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
                Argument::String(filter),
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
                Argument::String(filter),
            ],
        ),
        _ => {
            return Err(FileDialogError::Unsupported(
                "unsupported Files operation".into(),
            ));
        }
    };
    // One connection keeps any Response arriving before MethodReturn in the
    // client's pending queue. Do not resend an ambiguous timed-out request.
    let reply = call_files(&mut connection, method, args, abandon).map_err(platform)?;
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

// A definite ServiceNotFound is safe to retry. Never resend an ambiguous
// timeout: the Files process might already have created a picker for that call.
fn call_files(
    connection: &mut Connection,
    method: &str,
    args: Vec<Argument>,
    abandon: &AtomicBool,
) -> Result<Vec<Argument>, BusError> {
    let call = |connection: &mut Connection| {
        connection.call_method_timeout(
            DESKTOP_FILE_MANAGER_BUS_NAME,
            DESKTOP_FILE_MANAGER_OBJECT_PATH,
            DESKTOP_FILE_MANAGER_INTERFACE,
            method,
            args.clone(),
            2_000,
        )
    };
    match call(connection) {
        Err(BusError::ServiceNotFound) => {}
        result => return result,
    }
    if abandon.load(Ordering::Acquire) {
        return Err(BusError::ServiceNotFound);
    }
    // Same activation route as Scarlet's video player, on a separate connection
    // so the picker connection retains early Response signals in its pending queue.
    let mut launcher = Connection::connect()?;
    launcher.call_method_timeout(
        DESKTOP_STEMD_BUS_NAME,
        DESKTOP_STEMD_OBJECT_PATH,
        DESKTOP_STEMD_INTERFACE,
        DESKTOP_STEMD_LAUNCH_OR_FOCUS_METHOD,
        vec![Argument::String(DESKTOP_FILES_APP_ID.into())],
        3_000,
    )?;
    for _ in 0..30 {
        if abandon.load(Ordering::Acquire) {
            return Err(BusError::ServiceNotFound);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        match call(connection) {
            Err(BusError::ServiceNotFound) => {}
            result => return result,
        }
    }
    Err(BusError::ServiceNotFound)
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
