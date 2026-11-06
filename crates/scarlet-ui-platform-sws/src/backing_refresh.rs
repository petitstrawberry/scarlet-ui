//! Tablet decoration changes require a new backing even at the same extent.
use core::cell::Cell;

#[derive(Default)]
pub(super) struct BackingRefresh {
    configure_pending: Cell<bool>,
    sgfx_pending: Cell<bool>,
}

impl BackingRefresh {
    pub(super) fn decoration_changed(&self) {
        self.configure_pending.set(true);
    }

    pub(super) fn needs_resize(&self, size_matches: bool, backing_matches: bool) -> bool {
        self.configure_pending.get() || !size_matches || !backing_matches
    }

    /// Call only after the synchronous SHM replacement succeeds. A failed
    /// request must leave the force bit set for the next configure attempt.
    pub(super) fn backing_replaced(&self) {
        self.configure_pending.set(false);
        self.sgfx_pending.set(true);
    }

    pub(super) fn take_sgfx_replacement(&self) -> bool {
        self.sgfx_pending.replace(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_extent_decoration_configure_replaces_both_backings() {
        let refresh = BackingRefresh::default();
        assert!(!refresh.needs_resize(true, true));
        refresh.decoration_changed();
        assert!(refresh.needs_resize(true, true));
        // No allocation or generation change before a successful response.
        assert!(!refresh.take_sgfx_replacement());
        assert!(refresh.needs_resize(true, true));
        refresh.backing_replaced();
        assert!(!refresh.needs_resize(true, true));
        assert!(refresh.take_sgfx_replacement());
        assert!(!refresh.take_sgfx_replacement());
    }

    #[test]
    fn resize_round_trip_to_same_extent_still_replaces_sgfx_generation() {
        let refresh = BackingRefresh::default();
        assert!(refresh.needs_resize(true, false));
        refresh.backing_replaced();
        // Two configures can be handled before the next render. Even if the
        // final extent matches the old encoder, its generation must change.
        refresh.backing_replaced();
        assert!(refresh.take_sgfx_replacement());
        refresh.decoration_changed();
        assert!(refresh.needs_resize(true, true));
    }
}
