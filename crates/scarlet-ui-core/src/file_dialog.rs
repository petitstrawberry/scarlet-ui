//! Owner-bound asynchronous file selection. Backends never perform file I/O.
use crate::{os::Mutex, scene::WindowId};
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

/// Platform path without lossy UTF-8 conversion. Normal std builds preserve
/// native `PathBuf`/`OsString`; the legacy Scarlet runtime uses UTF-8 paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogPath {
    #[cfg(feature = "std")]
    inner: PathBuf,
    #[cfg(not(feature = "std"))]
    inner: String,
}
impl FileDialogPath {
    pub fn is_absolute(&self) -> bool {
        #[cfg(feature = "std")]
        {
            self.inner.is_absolute()
        }
        #[cfg(not(feature = "std"))]
        {
            self.inner.starts_with('/')
        }
    }
    pub fn to_str(&self) -> Option<&str> {
        #[cfg(feature = "std")]
        {
            self.inner.to_str()
        }
        #[cfg(not(feature = "std"))]
        {
            Some(&self.inner)
        }
    }
    fn contains_nul(&self) -> bool {
        #[cfg(feature = "std")]
        {
            self.inner.to_string_lossy().contains('\0')
        }
        #[cfg(not(feature = "std"))]
        {
            self.inner.contains('\0')
        }
    }
    #[cfg(feature = "std")]
    pub fn as_path(&self) -> &Path {
        &self.inner
    }
}
impl From<String> for FileDialogPath {
    fn from(value: String) -> Self {
        Self {
            inner: value.into(),
        }
    }
}
impl From<&str> for FileDialogPath {
    fn from(value: &str) -> Self {
        Self {
            inner: value.into(),
        }
    }
}
#[cfg(feature = "std")]
impl From<PathBuf> for FileDialogPath {
    fn from(inner: PathBuf) -> Self {
        Self { inner }
    }
}
#[cfg(feature = "std")]
impl From<FileDialogPath> for PathBuf {
    fn from(value: FileDialogPath) -> Self {
        value.inner
    }
}

/// Selection operation. New operations may be added in future releases.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogMode {
    Open,
    OpenMultiple,
    Save,
}
/// Literal extensions without dots or wildcards (e.g. `wav`, `json`).
/// Multiple filters are a union; labels are hints, not filesystem validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}
/// Whether the provider must apply the extension filters to its chooser UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileDialogFilterPolicy {
    /// Return Unsupported when the chooser cannot apply the requested filters.
    #[default]
    Required,
    /// Allow an unfiltered chooser; the application validates the selected format.
    Optional,
}
/// File dialog options. Initial directory must be absolute; default name is a basename.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FileDialog {
    pub mode: FileDialogMode,
    pub title: String,
    pub initial_directory: Option<FileDialogPath>,
    pub default_name: Option<String>,
    pub filters: Vec<FileDialogFilter>,
    pub filter_policy: FileDialogFilterPolicy,
}
impl FileDialog {
    pub fn new(mode: FileDialogMode) -> Self {
        Self {
            mode,
            title: String::new(),
            initial_directory: None,
            default_name: None,
            filters: Vec::new(),
            filter_policy: FileDialogFilterPolicy::Required,
        }
    }
    /// Queue on the application runner. Call from a UI callback with its live runtime owner.
    /// No panel is shown until the runner processes this request. Independent concurrent
    /// runners are unsupported, as with scene commands. Poll the handle from `on_idle`.
    pub fn show(self, owner: WindowId) -> FileDialogHandle {
        let handle = FileDialogHandle::new();
        match self.validate() {
            Ok(()) => REQUESTS.lock().push(QueuedDialog {
                owner,
                options: self,
                handle: handle.clone(),
            }),
            Err(error) => handle.complete(Err(error)),
        }
        handle
    }
    pub fn validate(&self) -> Result<(), FileDialogError> {
        if self
            .initial_directory
            .as_ref()
            .is_some_and(|p| !p.is_absolute() || p.contains_nul())
        {
            return Err(FileDialogError::InvalidOptions(
                "initial directory must be absolute".into(),
            ));
        }
        if self.title.contains('\0')
            || self.default_name.as_ref().is_some_and(|n| {
                n.is_empty() || n.contains(['/', '\\', '\0']) || n == "." || n == ".."
            })
        {
            return Err(FileDialogError::InvalidOptions(
                "invalid title or default filename".into(),
            ));
        }
        if self.filters.iter().any(|f| {
            f.extensions.is_empty()
                || f.extensions.iter().any(|e| {
                    e.is_empty()
                        || !e
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                })
        }) {
            return Err(FileDialogError::InvalidOptions(
                "filters require literal extensions without dots or wildcards".into(),
            ));
        }
        Ok(())
    }
}
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogOutcome {
    Selected(Vec<FileDialogPath>),
    Cancelled,
}
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogError {
    Unsupported(String),
    InvalidOptions(String),
    OwnerClosed,
    Busy,
    Platform(String),
}
impl core::fmt::Display for FileDialogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
#[cfg(feature = "std")]
impl std::error::Error for FileDialogError {}
pub type FileDialogResult = Result<FileDialogOutcome, FileDialogError>;
struct Shared {
    result: Mutex<Option<FileDialogResult>>,
    cancelled: AtomicBool,
    completed: AtomicBool,
}
/// A cloneable receipt. `take_result` consumes the result once across all clones.
/// Dropping a receipt does not cancel; call `cancel` explicitly.
#[derive(Clone)]
pub struct FileDialogHandle {
    shared: Arc<Shared>,
}
impl FileDialogHandle {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                result: Mutex::new(None),
                cancelled: AtomicBool::new(false),
                completed: AtomicBool::new(false),
            }),
        }
    }
    pub fn cancel(&self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.shared.completed.load(Ordering::Acquire)
    }
    pub fn take_result(&self) -> Option<FileDialogResult> {
        self.shared.result.lock().take()
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.shared.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn complete(&self, result: FileDialogResult) {
        let mut slot = self.shared.result.lock();
        if !self.shared.completed.load(Ordering::Acquire) {
            *slot = Some(result);
            self.shared.completed.store(true, Ordering::Release);
        }
    }
}
/// Backend-owned UI-thread session. Polling must not block. Dropping/cancelling
/// must detach native completion safely; never access a closed owner afterward.
pub trait FileDialogSession {
    fn poll(&mut self) -> Option<FileDialogResult>;
    fn cancel(&mut self);
}
pub(crate) struct QueuedDialog {
    pub owner: WindowId,
    pub options: FileDialog,
    pub handle: FileDialogHandle,
}
static REQUESTS: Mutex<Vec<QueuedDialog>> = Mutex::new(Vec::new());
pub(crate) fn take_requests() -> Vec<QueuedDialog> {
    core::mem::take(&mut *REQUESTS.lock())
}
pub(crate) struct ActiveDialog {
    pub session: Box<dyn FileDialogSession>,
    pub handle: FileDialogHandle,
}
impl Drop for ActiveDialog {
    fn drop(&mut self) {
        self.session.cancel();
        self.handle.complete(Err(FileDialogError::OwnerClosed));
    }
}
/// Internal runner operation, also exercised using deterministic backend sessions.
pub(crate) fn poll_active(active: &mut Option<ActiveDialog>) -> bool {
    let Some(dialog) = active.as_mut() else {
        return false;
    };
    let result = if dialog.handle.cancelled() {
        dialog.session.cancel();
        Some(Ok(FileDialogOutcome::Cancelled))
    } else {
        dialog.session.poll()
    };
    if let Some(result) = result {
        dialog.handle.complete(result);
        *active = None;
        true
    } else {
        false
    }
}

/// Completes queued requests even on runner errors or an empty startup scene.
pub(crate) struct RunnerDialogGuard;
impl Drop for RunnerDialogGuard {
    fn drop(&mut self) {
        for request in take_requests() {
            request.handle.complete(Err(FileDialogError::OwnerClosed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::Cell;
    struct Session {
        result: Option<FileDialogResult>,
        cancels: Rc<Cell<usize>>,
    }
    impl FileDialogSession for Session {
        fn poll(&mut self) -> Option<FileDialogResult> {
            self.result.take()
        }
        fn cancel(&mut self) {
            self.cancels.set(self.cancels.get() + 1);
        }
    }
    fn active(
        result: Option<FileDialogResult>,
    ) -> (Option<ActiveDialog>, FileDialogHandle, Rc<Cell<usize>>) {
        let handle = FileDialogHandle::new();
        let cancels = Rc::new(Cell::new(0));
        (
            Some(ActiveDialog {
                session: Box::new(Session {
                    result,
                    cancels: cancels.clone(),
                }),
                handle: handle.clone(),
            }),
            handle,
            cancels,
        )
    }
    #[test]
    fn selection_is_consumed_once_and_not_overwritten_by_drop() {
        let selected = Ok(FileDialogOutcome::Selected(vec![
            FileDialogPath::from("/tmp/a.wav"),
            FileDialogPath::from("/tmp/b.wav"),
        ]));
        let (mut active, handle, _) = active(Some(selected.clone()));
        assert!(poll_active(&mut active));
        assert!(handle.is_finished());
        assert_eq!(handle.clone().take_result(), Some(selected));
        assert_eq!(handle.take_result(), None);
    }
    #[test]
    fn cancel_takes_precedence_over_late_backend_selection() {
        let (mut active, handle, cancels) = active(Some(Ok(FileDialogOutcome::Selected(vec![
            FileDialogPath::from("/tmp/a"),
        ]))));
        handle.cancel();
        assert!(poll_active(&mut active));
        assert!(cancels.get() > 0);
        assert_eq!(handle.take_result(), Some(Ok(FileDialogOutcome::Cancelled)));
    }
    #[test]
    fn closing_owner_is_an_error_and_detaches_pending_backend() {
        let (active, handle, cancels) = active(None);
        drop(active);
        assert_eq!(
            handle.take_result(),
            Some(Err(FileDialogError::OwnerClosed))
        );
        assert_eq!(cancels.get(), 1);
    }
    #[test]
    fn cancel_and_platform_failure_remain_distinct() {
        for value in [
            Ok(FileDialogOutcome::Cancelled),
            Err(FileDialogError::Platform("disconnected".into())),
        ] {
            let (mut active, handle, _) = active(Some(value.clone()));
            assert!(poll_active(&mut active));
            assert_eq!(handle.take_result(), Some(value));
        }
    }
    #[cfg(all(feature = "std", unix))]
    #[test]
    fn native_path_bytes_survive_roundtrip_and_nul_directory_is_rejected() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', b't', 0xff]));
        let value = FileDialogPath::from(path.clone());
        assert_eq!(value.to_str(), None);
        assert_eq!(PathBuf::from(value), path);
        let mut options = FileDialog::new(FileDialogMode::Open);
        options.initial_directory = Some(FileDialogPath::from("/tmp/a\0b"));
        assert!(options.validate().is_err());
    }
    #[test]
    fn options_reject_relative_directories_and_unsafe_basenames_or_filters() {
        let mut options = FileDialog::new(FileDialogMode::Save);
        options.initial_directory = Some(FileDialogPath::from("relative"));
        assert!(options.validate().is_err());
        options.initial_directory = Some(FileDialogPath::from("/tmp"));
        for name in ["", "../x", "..", "a/b", "a\\b", "a\0b"] {
            options.default_name = Some(name.into());
            assert!(options.validate().is_err(), "{name}");
        }
        options.default_name = Some("日本語.wav".into());
        options.filters = vec![FileDialogFilter {
            name: "Audio".into(),
            extensions: vec!["*.wav".into()],
        }];
        assert!(options.validate().is_err());
        options.filters[0].extensions = vec!["wav".into(), "WAV".into()];
        assert!(options.validate().is_ok());
    }
}
