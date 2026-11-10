//! Text-only native clipboard. One process-wide worker retains selection ownership
//! on X11 and bounds both native concurrency and caller wait times.
use scarlet_ui_core::clipboard::{
    ClipboardError, ClipboardProvider, MAX_TEXT_BYTES, TextClipboard,
};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Access {
    // Even generations are unfocused; odd generations are focused. Changing
    // focus invalidates requests even when focus returns before they finish.
    generation: AtomicU64,
    closed: AtomicBool,
}
impl Access {
    pub(crate) fn focus(&self, focused: bool) {
        let current = self.generation.load(Ordering::Acquire);
        if (current & 1 != 0) != focused {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    fn authorize(&self) -> Result<u64, ClipboardError> {
        let generation = self.generation.load(Ordering::Acquire);
        self.validate(generation)?;
        Ok(generation)
    }
    fn validate(&self, generation: u64) -> Result<(), ClipboardError> {
        if self.closed.load(Ordering::Acquire)
            || generation & 1 == 0
            || self.generation.load(Ordering::Acquire) != generation
        {
            Err(ClipboardError::AccessDenied)
        } else {
            Ok(())
        }
    }
}

enum Operation {
    Read,
    Write(String),
}
type Reply = Result<Option<String>, ClipboardError>;
struct Request {
    access: Arc<Access>,
    generation: u64,
    operation: Operation,
    cancelled: Arc<AtomicBool>,
    reply: mpsc::SyncSender<Reply>,
}
struct Worker {
    sender: mpsc::SyncSender<Request>,
    busy: Arc<AtomicBool>,
}
impl Worker {
    fn start(
        mut execute: impl FnMut(Operation, &dyn Fn() -> Result<(), ClipboardError>) -> Reply
        + Send
        + 'static,
    ) -> Result<Self, ClipboardError> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = busy.clone();
        std::thread::Builder::new()
            .name("native-text-clipboard".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let result = request.access.validate(request.generation).and_then(|()| {
                        if request.cancelled.load(Ordering::Acquire) {
                            return Err(unavailable("clipboard request expired"));
                        }
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute(request.operation, &|| {
                                request.access.validate(request.generation)?;
                                if request.cancelled.load(Ordering::Acquire) {
                                    Err(unavailable("clipboard request expired"))
                                } else {
                                    Ok(())
                                }
                            })
                        }))
                        .unwrap_or_else(|_| Err(unavailable("native clipboard failed")));
                        // Never deliver data obtained for an owner that lost access.
                        request.access.validate(request.generation)?;
                        result
                    });
                    worker_busy.store(false, Ordering::Release);
                    let _ = request.reply.send(result);
                }
            })
            .map_err(|_| unavailable("could not start native clipboard worker"))?;
        Ok(Self { sender, busy })
    }
    fn request(&self, access: &Arc<Access>, operation: Operation, timeout: Duration) -> Reply {
        let generation = access.authorize()?;
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ClipboardError::Busy);
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Request {
            access: access.clone(),
            generation,
            operation,
            cancelled: cancelled.clone(),
            reply,
        };
        if self.sender.try_send(request).is_err() {
            self.busy.store(false, Ordering::Release);
            return Err(unavailable("native clipboard worker stopped"));
        }
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                access.validate(generation)?;
                result
            }
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                // Keep busy until the native call exits; never accumulate stuck
                // workers or pending writes after a timeout.
                Err(unavailable("native clipboard timed out"))
            }
        }
    }
}
fn unavailable(message: &str) -> ClipboardError {
    ClipboardError::Unavailable(message.into())
}
fn worker() -> Result<&'static Worker, ClipboardError> {
    static WORKER: OnceLock<Result<Worker, ClipboardError>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let mut clipboard = None;
            Worker::start(move |operation, validate| {
                if clipboard.is_none() {
                    clipboard = Some(
                        arboard::Clipboard::new()
                            .map_err(|error| unavailable(&error.to_string()))?,
                    );
                }
                let clipboard = clipboard.as_mut().expect("initialized above");
                validate()?;
                match operation {
                    Operation::Read => match clipboard.get_text() {
                        Ok(text) if text.len() > MAX_TEXT_BYTES => Err(ClipboardError::TooLarge),
                        Ok(text) => Ok(Some(text)),
                        Err(arboard::Error::ContentNotAvailable) => Ok(None),
                        Err(error) => Err(unavailable(&error.to_string())),
                    },
                    Operation::Write(text) => clipboard
                        .set_text(text)
                        .map(|()| None)
                        .map_err(|error| unavailable(&error.to_string())),
                }
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}
struct NativeClipboard {
    access: Arc<Access>,
}
impl ClipboardProvider for NativeClipboard {
    fn read_text(&self) -> Reply {
        self.access.authorize()?;
        worker()?.request(&self.access, Operation::Read, TIMEOUT)
    }
    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        self.access.authorize()?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        worker()?
            .request(&self.access, Operation::Write(text.into()), TIMEOUT)
            .map(|_| ())
    }
}
pub(crate) fn service(access: Arc<Access>) -> TextClipboard {
    TextClipboard::new(NativeClipboard { access })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_and_window_lifetime_revoke_access() {
        let access = Access::default();
        assert_eq!(access.authorize(), Err(ClipboardError::AccessDenied));
        access.focus(true);
        let generation = access.authorize().unwrap();
        access.focus(false);
        access.focus(true);
        assert_eq!(
            access.validate(generation),
            Err(ClipboardError::AccessDenied)
        );
        access.close();
        assert_eq!(access.authorize(), Err(ClipboardError::AccessDenied));
    }
    #[test]
    fn timeout_retains_single_native_operation() {
        let (release, wait) = mpsc::channel();
        let worker = Worker::start(move |_, _| {
            wait.recv().unwrap();
            Ok(Some("日本 👩‍💻".into()))
        })
        .unwrap();
        let access = Arc::new(Access::default());
        access.focus(true);
        assert!(matches!(
            worker.request(&access, Operation::Read, Duration::from_millis(100)),
            Err(ClipboardError::Unavailable(_))
        ));
        assert_eq!(
            worker.request(&access, Operation::Read, TIMEOUT),
            Err(ClipboardError::Busy)
        );
        release.send(()).unwrap();
    }
    #[test]
    fn unicode_text_round_trips_on_worker() {
        let mut stored = None;
        let worker = Worker::start(move |operation, _| match operation {
            Operation::Read => Ok(stored.clone()),
            Operation::Write(text) => {
                stored = Some(text);
                Ok(None)
            }
        })
        .unwrap();
        let access = Arc::new(Access::default());
        access.focus(true);
        assert_eq!(worker.request(&access, Operation::Read, TIMEOUT), Ok(None));
        let text = "日本 👩‍💻\nsecond line";
        assert_eq!(
            worker.request(&access, Operation::Write(text.into()), TIMEOUT),
            Ok(None)
        );
        assert_eq!(
            worker.request(&access, Operation::Read, TIMEOUT),
            Ok(Some(text.into()))
        );
    }
    #[test]
    fn focus_loss_discards_in_flight_reads() {
        let access = Arc::new(Access::default());
        access.focus(true);
        let owner = access.clone();
        let worker = Worker::start(move |_, _| {
            owner.focus(false);
            owner.focus(true);
            Ok(Some("private text".into()))
        })
        .unwrap();
        assert_eq!(
            worker.request(&access, Operation::Read, TIMEOUT),
            Err(ClipboardError::AccessDenied)
        );
    }
    #[test]
    fn oversized_write_is_rejected_before_native_access() {
        let access = Arc::new(Access::default());
        access.focus(true);
        let provider = NativeClipboard { access };
        assert_eq!(
            provider.write_text(&"a".repeat(MAX_TEXT_BYTES + 1)),
            Err(ClipboardError::TooLarge)
        );
    }
    #[test]
    fn unfocused_requests_never_reach_native_clipboard() {
        let worker = Worker::start(|_, _| panic!("must not execute")).unwrap();
        let access = Arc::new(Access::default());
        assert_eq!(
            worker.request(&access, Operation::Read, TIMEOUT),
            Err(ClipboardError::AccessDenied)
        );
    }
}
