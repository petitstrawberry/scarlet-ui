//! OS compatibility layer for ScarletUI.

#[cfg(feature = "std")]
pub struct Mutex<T> {
    inner: std::sync::Mutex<T>,
}

#[cfg(feature = "std")]
impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: std::sync::Mutex::new(value),
        }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.inner.lock().expect("scarlet-ui mutex poisoned")
    }
}

#[cfg(not(feature = "std"))]
pub use scarlet_std::fs::File;
#[cfg(not(feature = "std"))]
pub use scarlet_std::sync::Mutex;

/// Run work on a detached thread.
///
/// This is used for callbacks that may perform blocking IPC or filesystem I/O
/// and therefore must not run on the application event loop.
#[cfg(feature = "std")]
pub fn spawn_detached<F>(task: F)
where
    F: FnOnce() + Send + 'static,
{
    drop(std::thread::spawn(task));
}

/// Run work on a detached Scarlet userspace thread.
#[cfg(not(feature = "std"))]
pub fn spawn_detached<F>(task: F)
where
    F: FnOnce() + Send + 'static,
{
    drop(scarlet_std::thread::spawn(task));
}
