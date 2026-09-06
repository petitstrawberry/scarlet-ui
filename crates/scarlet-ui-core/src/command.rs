//! Application command queue for scene-level window actions.

use alloc::vec::Vec;

use crate::os::Mutex;
use crate::scene::SceneWindowKey;

/// Scene-level command emitted by user callbacks and consumed by the runner.
pub enum ApplicationCommand {
    /// Open a declared scene window only if no instance of its key is open.
    OpenWindow(SceneWindowKey),
    /// Open an additional runtime instance for a declared scene window.
    OpenNewWindow(SceneWindowKey),
    /// Dismiss all runtime instances of a declared scene window.
    DismissWindow(SceneWindowKey),
}

static APPLICATION_COMMANDS: Mutex<Vec<ApplicationCommand>> = Mutex::new(Vec::new());

/// Request that a declared scene window be opened.
///
/// When the runner handles this command, an already-open instance makes it a
/// no-op; it does not focus that instance. An undeclared key is also a no-op.
///
/// # Arguments
///
/// * `key` - Stable scene window key declared by `Application::scenes()`.
///
/// # Returns
///
/// Nothing. This queues a request, not an acknowledgment of window creation.
pub fn open_window(key: impl Into<SceneWindowKey>) {
    APPLICATION_COMMANDS
        .lock()
        .push(ApplicationCommand::OpenWindow(key.into()));
}

/// Request that a new runtime instance of a declared scene window be opened.
///
/// Unlike [`open_window`], this does not reuse an already-open instance with
/// the same scene key. It is intended for document windows, dialogs, and
/// other application windows that may be opened more than once.
///
/// # Arguments
///
/// * `key` - Stable scene window key declared by `Application::scenes()`.
///
/// # Returns
///
/// Nothing. The runner creates a fresh runtime identity when it handles the
/// request; an undeclared key is a no-op.
pub fn open_new_window(key: impl Into<SceneWindowKey>) {
    APPLICATION_COMMANDS
        .lock()
        .push(ApplicationCommand::OpenNewWindow(key.into()));
}

/// Request that a declared scene window be dismissed.
///
/// The runner closes every instance with this key, bypassing
/// `Application::on_window_close_requested`. A missing key is a no-op.
///
/// # Arguments
///
/// * `key` - Stable scene window key declared by `Application::scenes()`.
///
/// # Returns
///
/// Nothing. This queues a request, not an acknowledgment from the window system.
pub fn dismiss_window(key: impl Into<SceneWindowKey>) {
    APPLICATION_COMMANDS
        .lock()
        .push(ApplicationCommand::DismissWindow(key.into()));
}

pub(crate) fn take_application_commands() -> Vec<ApplicationCommand> {
    core::mem::take(&mut *APPLICATION_COMMANDS.lock())
}
