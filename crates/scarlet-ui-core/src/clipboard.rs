//! Window-bound, asynchronous access to the real platform text clipboard.
use crate::os::Mutex;
use alloc::{string::String, sync::Arc};
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    Unsupported,
    AccessDenied,
    TooLarge,
    Busy,
    Unavailable(String),
}
impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Desktop clipboard is unavailable on this backend"),
            Self::AccessDenied => f.write_str("Focus this window to access the clipboard"),
            Self::TooLarge => f.write_str("Clipboard text exceeds 256 KiB"),
            Self::Busy => f.write_str("A clipboard operation is already pending"),
            Self::Unavailable(reason) => write!(f, "Clipboard unavailable: {reason}"),
        }
    }
}

/// Backend implementation. Methods execute on a worker, never the event loop.
/// Each call must have a bounded wait and revalidate owner access where applicable.
pub trait ClipboardProvider: Send + Sync + 'static {
    fn read_text(&self) -> Result<Option<String>, ClipboardError>;
    fn write_text(&self, text: &str) -> Result<(), ClipboardError>;
}

struct Shared {
    provider: Arc<dyn ClipboardProvider>,
    busy: AtomicBool,
}
/// Cloneable service obtained from a live platform window. Clones share one
/// in-flight operation limit. A closed native window loses authorization.
#[derive(Clone)]
pub struct TextClipboard {
    shared: Arc<Shared>,
}
impl TextClipboard {
    pub fn new(provider: impl ClipboardProvider) -> Self {
        Self {
            shared: Arc::new(Shared {
                provider: Arc::new(provider),
                busy: AtomicBool::new(false),
            }),
        }
    }
    pub fn read_text(&self) -> ClipboardHandle<Option<String>> {
        self.start(|provider| {
            let text = provider.read_text()?;
            if text
                .as_ref()
                .is_some_and(|text| text.len() > MAX_TEXT_BYTES)
            {
                return Err(ClipboardError::TooLarge);
            }
            Ok(text)
        })
    }
    pub fn write_text(&self, text: String) -> ClipboardHandle<()> {
        if text.len() > MAX_TEXT_BYTES {
            return ClipboardHandle::ready(Err(ClipboardError::TooLarge));
        }
        self.start(move |provider| provider.write_text(&text))
    }
    fn start<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&dyn ClipboardProvider) -> Result<T, ClipboardError> + Send + 'static,
    ) -> ClipboardHandle<T> {
        if self
            .shared
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return ClipboardHandle::ready(Err(ClipboardError::Busy));
        }
        let shared = self.shared.clone();
        let handle = ClipboardHandle::pending();
        let completion = handle.result.clone();
        let task = move || {
            struct Release(Arc<Shared>);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0.busy.store(false, Ordering::Release);
                }
            }
            let release = Release(shared);
            #[cfg(feature = "std")]
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                operation(release.0.provider.as_ref())
            }))
            .unwrap_or_else(|_| {
                Err(ClipboardError::Unavailable(
                    "clipboard provider failed".into(),
                ))
            });
            #[cfg(not(feature = "std"))]
            let result = operation(release.0.provider.as_ref());
            // Publish readiness only after the service can accept the next op.
            drop(release);
            *completion.lock() = Some(result);
        };
        #[cfg(feature = "std")]
        let started = std::thread::Builder::new()
            .name("text-clipboard".into())
            .spawn(task)
            .map(|thread| drop(thread))
            .map_err(|_| ());
        #[cfg(not(feature = "std"))]
        let started = scarlet_std::thread::Builder::new()
            .spawn(task)
            .map(|thread| drop(thread))
            .map_err(|_| ());
        if started.is_err() {
            self.shared.busy.store(false, Ordering::Release);
            *handle.result.lock() = Some(Err(ClipboardError::Unavailable(
                "could not start clipboard worker".into(),
            )));
        }
        handle
    }
}

/// Poll with `try_take` from Application::on_idle. Dropping the handle discards
/// its eventual result; an already sent write is not cancelled or rolled back.
pub struct ClipboardHandle<T> {
    result: Arc<Mutex<Option<Result<T, ClipboardError>>>>,
}
impl<T> ClipboardHandle<T> {
    fn pending() -> Self {
        Self {
            result: Arc::new(Mutex::new(None)),
        }
    }
    fn ready(result: Result<T, ClipboardError>) -> Self {
        Self {
            result: Arc::new(Mutex::new(Some(result))),
        }
    }
    pub fn try_take(&self) -> Option<Result<T, ClipboardError>> {
        self.result.lock().take()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::sync::{Barrier, atomic::AtomicUsize};
    struct Failing;
    impl ClipboardProvider for Failing {
        fn read_text(&self) -> Result<Option<String>, ClipboardError> {
            panic!("provider failure");
        }
        fn write_text(&self, _: &str) -> Result<(), ClipboardError> {
            Ok(())
        }
    }
    #[test]
    fn provider_panic_completes_with_error_and_releases_busy() {
        let clipboard = TextClipboard::new(Failing);
        let task = clipboard.read_text();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(result) = task.try_take() {
                assert!(matches!(result, Err(ClipboardError::Unavailable(_))));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!clipboard.shared.busy.load(Ordering::Acquire));
    }
    struct Blocked {
        gate: Arc<Barrier>,
        writes: Arc<AtomicUsize>,
    }
    impl ClipboardProvider for Blocked {
        fn read_text(&self) -> Result<Option<String>, ClipboardError> {
            self.gate.wait();
            Ok(Some("日本 👩‍💻".into()))
        }
        fn write_text(&self, _: &str) -> Result<(), ClipboardError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    #[test]
    fn worker_is_nonblocking_busy_is_bounded_and_oversize_never_writes() {
        let gate = Arc::new(Barrier::new(2));
        let writes = Arc::new(AtomicUsize::new(0));
        let clipboard = TextClipboard::new(Blocked {
            gate: gate.clone(),
            writes: writes.clone(),
        });
        let read = clipboard.read_text();
        assert!(read.try_take().is_none());
        assert_eq!(
            clipboard.clone().write_text("x".into()).try_take(),
            Some(Err(ClipboardError::Busy))
        );
        assert_eq!(
            clipboard
                .write_text("x".repeat(MAX_TEXT_BYTES + 1))
                .try_take(),
            Some(Err(ClipboardError::TooLarge))
        );
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        gate.wait();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(result) = read.try_take() {
                assert_eq!(result.unwrap(), Some("日本 👩‍💻".into()));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(read.try_take().is_none());
    }
}
