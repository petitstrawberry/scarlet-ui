//! Runtime input-device environment and interaction-mode resolution.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicU64, Ordering};

const TABLET_KNOWN: u64 = 1 << 0;
const TABLET_ON: u64 = 1 << 1;
const LID_KNOWN: u64 = 1 << 2;
const LID_CLOSED: u64 = 1 << 3;
const DIRECT_TOUCH: u64 = 1 << 4;
const FINE_POINTER: u64 = 1 << 5;
const KEYBOARD: u64 = 1 << 6;
const PEN: u64 = 1 << 7;
const WINDOWING_KNOWN: u64 = 1 << 8;
const WINDOWING_FOCUSED: u64 = 1 << 9;
const TABLET_OVERRIDE_KNOWN: u64 = 1 << 10;
const TABLET_OVERRIDE_ACTIVE: u64 = 1 << 11;
const WINDOWING_OVERRIDE_KNOWN: u64 = 1 << 12;
const WINDOWING_OVERRIDE_ACTIVE: u64 = 1 << 13;

static CURRENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static CURRENT_GENERATION: AtomicU64 = AtomicU64::new(0);
static CURRENT_FLAGS: AtomicU64 = AtomicU64::new(FINE_POINTER | KEYBOARD);

#[cfg(test)]
std::thread_local! {
    static TEST_ENVIRONMENT_OVERRIDE: core::cell::Cell<Option<InputEnvironment>> = const {
        core::cell::Cell::new(None)
    };
}

/// The interface density selected solely from the current tablet posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionMode {
    /// Compact controls used while the device is not in tablet mode.
    Pointer,
    /// Tablet controls used while tablet mode is explicitly enabled.
    Touch,
}

/// System-wide window-management presentation selected by SWS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowingMode {
    /// Conventional independently positioned and sized windows.
    Freeform,
    /// Focused tablet-style windows with explicit multitasking affordances.
    Focused,
}

/// A snapshot of the platform's runtime input-device environment.
///
/// The generation is supplied by the platform and should increase whenever a
/// newer snapshot supersedes an older one. Optional tablet and lid states keep
/// "unknown" distinct from `false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEnvironment {
    /// Monotonic platform generation for this snapshot.
    pub generation: u64,
    /// Whether the device is explicitly in tablet mode, or `None` when unknown.
    pub tablet_mode: Option<bool>,
    /// Whether the device lid is closed, or `None` when unknown.
    pub lid_closed: Option<bool>,
    /// Whether a direct-touch input device is available.
    pub direct_touch: bool,
    /// Whether a fine pointer such as a mouse or trackpad is available.
    pub fine_pointer: bool,
    /// Whether a hardware keyboard is available.
    pub keyboard: bool,
    /// Whether pen input is available.
    pub pen: bool,
    /// Effective system-wide windowing policy, when the platform reports it.
    pub windowing_mode: Option<WindowingMode>,
    /// Whether posture is forced instead of hardware-driven, when known.
    pub tablet_mode_override_active: Option<bool>,
    /// Whether windowing policy is forced instead of posture-derived, when known.
    pub windowing_mode_override_active: Option<bool>,
}

impl InputEnvironment {
    /// Create an input-environment snapshot.
    ///
    /// # Arguments
    ///
    /// * `generation` - Monotonic platform snapshot generation
    /// * `tablet_mode` - Explicit tablet state, or `None` when unknown
    /// * `lid_closed` - Explicit lid state, or `None` when unknown
    /// * `direct_touch` - Direct-touch capability
    /// * `fine_pointer` - Mouse or trackpad capability
    /// * `keyboard` - Hardware keyboard capability
    /// * `pen` - Pen capability
    ///
    /// # Returns
    ///
    /// A complete immutable input-environment snapshot.
    pub const fn new(
        generation: u64,
        tablet_mode: Option<bool>,
        lid_closed: Option<bool>,
        direct_touch: bool,
        fine_pointer: bool,
        keyboard: bool,
        pen: bool,
    ) -> Self {
        Self {
            generation,
            tablet_mode,
            lid_closed,
            direct_touch,
            fine_pointer,
            keyboard,
            pen,
            windowing_mode: None,
            tablet_mode_override_active: None,
            windowing_mode_override_active: None,
        }
    }

    /// Attach system-wide presentation policy to this input snapshot.
    ///
    /// # Arguments
    ///
    /// * `windowing_mode` - Effective focused/freeform policy, when known.
    /// * `tablet_mode_override_active` - Whether posture is currently forced.
    /// * `windowing_mode_override_active` - Whether windowing policy is forced.
    ///
    /// # Returns
    ///
    /// The enriched immutable snapshot.
    pub const fn with_system_mode(
        mut self,
        windowing_mode: Option<WindowingMode>,
        tablet_mode_override_active: Option<bool>,
        windowing_mode_override_active: Option<bool>,
    ) -> Self {
        self.windowing_mode = windowing_mode;
        self.tablet_mode_override_active = tablet_mode_override_active;
        self.windowing_mode_override_active = windowing_mode_override_active;
        self
    }

    /// Return the compact desktop environment used by backends without input discovery.
    ///
    /// # Returns
    ///
    /// An environment with a fine pointer and keyboard, and unknown posture states.
    pub const fn desktop() -> Self {
        Self::new(0, None, None, false, true, true, false).with_system_mode(
            Some(WindowingMode::Freeform),
            Some(false),
            Some(false),
        )
    }

    /// Resolve the effective interaction mode for this snapshot.
    ///
    /// Presentation follows tablet posture only. Input capabilities remain
    /// available through their dedicated accessors but never affect UI mode.
    ///
    /// # Returns
    ///
    /// Touch when tablet mode is explicitly enabled, otherwise pointer.
    pub const fn interaction_mode(self) -> InteractionMode {
        if matches!(self.tablet_mode, Some(true)) {
            InteractionMode::Touch
        } else {
            InteractionMode::Pointer
        }
    }

    /// Return the optional tablet-mode state.
    ///
    /// # Returns
    ///
    /// `Some` when the platform knows the posture, otherwise `None`.
    pub const fn tablet_mode(self) -> Option<bool> {
        self.tablet_mode
    }

    /// Return the optional lid-closed state.
    ///
    /// # Returns
    ///
    /// `Some` when the platform knows the lid state, otherwise `None`.
    pub const fn lid_closed(self) -> Option<bool> {
        self.lid_closed
    }

    /// Return whether direct-touch input is available.
    ///
    /// # Returns
    ///
    /// `true` when the environment includes a direct-touch device.
    pub const fn has_direct_touch(self) -> bool {
        self.direct_touch
    }

    /// Return whether a fine pointer is available.
    ///
    /// # Returns
    ///
    /// `true` when the environment includes a mouse, trackpad, or equivalent pointer.
    pub const fn has_fine_pointer(self) -> bool {
        self.fine_pointer
    }

    /// Return whether a hardware keyboard is available.
    ///
    /// # Returns
    ///
    /// `true` when the environment includes a hardware keyboard.
    pub const fn has_keyboard(self) -> bool {
        self.keyboard
    }

    /// Return whether pen input is available.
    ///
    /// # Returns
    ///
    /// `true` when the environment includes a pen device.
    pub const fn has_pen(self) -> bool {
        self.pen
    }

    /// Return the effective system-wide windowing policy.
    ///
    /// # Returns
    ///
    /// The platform policy, or `None` when it is not reported.
    pub const fn windowing_mode(self) -> Option<WindowingMode> {
        self.windowing_mode
    }

    /// Return whether tablet posture is currently overridden.
    ///
    /// # Returns
    ///
    /// The override state, or `None` when it is not reported.
    pub const fn tablet_mode_override_active(self) -> Option<bool> {
        self.tablet_mode_override_active
    }

    /// Return whether windowing policy is currently overridden.
    ///
    /// # Returns
    ///
    /// The override state, or `None` when it is not reported.
    pub const fn windowing_mode_override_active(self) -> Option<bool> {
        self.windowing_mode_override_active
    }
}

impl Default for InputEnvironment {
    fn default() -> Self {
        Self::desktop()
    }
}

/// Return the process-live input environment.
///
/// This process-wide seam keeps all open pipelines coherent today. A future
/// runner- or per-window environment may replace it without changing
/// `InputEnvironment`.
///
/// # Returns
///
/// The most recently installed process-live snapshot.
pub fn current_input_environment() -> InputEnvironment {
    #[cfg(test)]
    if let Some(environment) = TEST_ENVIRONMENT_OVERRIDE.with(core::cell::Cell::get) {
        return environment;
    }

    load_published_input_environment()
}

fn load_published_input_environment() -> InputEnvironment {
    loop {
        let sequence = CURRENT_SEQUENCE.load(Ordering::Acquire);
        if sequence & 1 != 0 {
            spin_loop();
            continue;
        }
        let generation = CURRENT_GENERATION.load(Ordering::Relaxed);
        let flags = CURRENT_FLAGS.load(Ordering::Relaxed);
        if CURRENT_SEQUENCE.load(Ordering::Acquire) == sequence {
            return unpack_environment(generation, flags);
        }
    }
}

fn unpack_environment(generation: u64, flags: u64) -> InputEnvironment {
    InputEnvironment::new(
        generation,
        option_flag(flags, TABLET_KNOWN, TABLET_ON),
        option_flag(flags, LID_KNOWN, LID_CLOSED),
        flags & DIRECT_TOUCH != 0,
        flags & FINE_POINTER != 0,
        flags & KEYBOARD != 0,
        flags & PEN != 0,
    )
    .with_system_mode(
        if flags & WINDOWING_KNOWN == 0 {
            None
        } else if flags & WINDOWING_FOCUSED != 0 {
            Some(WindowingMode::Focused)
        } else {
            Some(WindowingMode::Freeform)
        },
        option_flag(flags, TABLET_OVERRIDE_KNOWN, TABLET_OVERRIDE_ACTIVE),
        option_flag(flags, WINDOWING_OVERRIDE_KNOWN, WINDOWING_OVERRIDE_ACTIVE),
    )
}

#[cfg_attr(test, allow(dead_code))] // Unit tests publish through the thread-local override.
fn publish_input_environment(environment: InputEnvironment) {
    let mut sequence = CURRENT_SEQUENCE.load(Ordering::Relaxed);
    loop {
        if sequence & 1 != 0 {
            spin_loop();
            sequence = CURRENT_SEQUENCE.load(Ordering::Relaxed);
            continue;
        }
        match CURRENT_SEQUENCE.compare_exchange_weak(
            sequence,
            sequence.wrapping_add(1),
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => sequence = observed,
        }
    }

    CURRENT_GENERATION.store(environment.generation, Ordering::Relaxed);
    CURRENT_FLAGS.store(pack_flags(environment), Ordering::Relaxed);
    CURRENT_SEQUENCE.store(sequence.wrapping_add(2), Ordering::Release);
}

pub(crate) fn install_input_environment(environment: InputEnvironment) -> bool {
    let previous = current_input_environment();

    #[cfg(test)]
    {
        TEST_ENVIRONMENT_OVERRIDE.with(|slot| slot.set(Some(environment)));
        return previous != environment;
    }

    #[cfg(not(test))]
    {
        publish_input_environment(environment);
        previous != environment
    }
}

#[cfg(test)]
static ENVIRONMENT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) struct InputEnvironmentTestGuard {
    previous: InputEnvironment,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for InputEnvironmentTestGuard {
    fn drop(&mut self) {
        TEST_ENVIRONMENT_OVERRIDE.with(|slot| slot.set(Some(self.previous)));
    }
}

#[cfg(test)]
pub(crate) fn install_test_input_environment(
    environment: InputEnvironment,
) -> InputEnvironmentTestGuard {
    let lock = ENVIRONMENT_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = current_input_environment();
    install_input_environment(environment);
    InputEnvironmentTestGuard {
        previous,
        _lock: lock,
    }
}

const fn option_flag(flags: u64, known: u64, value: u64) -> Option<bool> {
    if flags & known == 0 {
        None
    } else {
        Some(flags & value != 0)
    }
}

#[cfg_attr(test, allow(dead_code))] // Unit tests publish through the thread-local override.
const fn pack_flags(environment: InputEnvironment) -> u64 {
    let mut flags = 0;
    if let Some(tablet) = environment.tablet_mode {
        flags |= TABLET_KNOWN;
        if tablet {
            flags |= TABLET_ON;
        }
    }
    if let Some(closed) = environment.lid_closed {
        flags |= LID_KNOWN;
        if closed {
            flags |= LID_CLOSED;
        }
    }
    if environment.direct_touch {
        flags |= DIRECT_TOUCH;
    }
    if environment.fine_pointer {
        flags |= FINE_POINTER;
    }
    if environment.keyboard {
        flags |= KEYBOARD;
    }
    if environment.pen {
        flags |= PEN;
    }
    if let Some(windowing_mode) = environment.windowing_mode {
        flags |= WINDOWING_KNOWN;
        if matches!(windowing_mode, WindowingMode::Focused) {
            flags |= WINDOWING_FOCUSED;
        }
    }
    if let Some(active) = environment.tablet_mode_override_active {
        flags |= TABLET_OVERRIDE_KNOWN;
        if active {
            flags |= TABLET_OVERRIDE_ACTIVE;
        }
    }
    if let Some(active) = environment.windowing_mode_override_active {
        flags |= WINDOWING_OVERRIDE_KNOWN;
        if active {
            flags |= WINDOWING_OVERRIDE_ACTIVE;
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_mode_resolution_obeys_only_tablet_posture() {
        let environment = |tablet, touch, pointer| {
            InputEnvironment::new(1, tablet, None, touch, pointer, true, false)
        };
        assert_eq!(
            environment(Some(true), false, true).interaction_mode(),
            InteractionMode::Touch
        );
        assert_eq!(
            environment(Some(false), true, true).interaction_mode(),
            InteractionMode::Pointer
        );
        assert_eq!(
            environment(None, true, false).interaction_mode(),
            InteractionMode::Pointer
        );
        assert_eq!(
            environment(None, false, false).interaction_mode(),
            InteractionMode::Pointer
        );
    }

    #[test]
    fn system_windowing_and_override_metadata_survive_atomic_encoding() {
        let environment =
            InputEnvironment::new(17, Some(true), Some(false), true, true, false, true)
                .with_system_mode(Some(WindowingMode::Focused), Some(true), Some(false));

        assert_eq!(unpack_environment(17, pack_flags(environment)), environment);
        assert_eq!(environment.windowing_mode(), Some(WindowingMode::Focused));
        assert_eq!(environment.tablet_mode_override_active(), Some(true));
        assert_eq!(environment.windowing_mode_override_active(), Some(false));
    }
}
