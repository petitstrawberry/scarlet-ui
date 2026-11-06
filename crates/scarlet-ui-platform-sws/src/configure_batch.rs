//! SWS sends policy state before its matching configure. Publish them together,
//! even when dispatch stops between the messages, and keep only the latest size.
#[derive(Default)]
pub(super) struct ConfigureBatch {
    size: Option<(u32, u32)>,
    fullscreen: Option<bool>,
}

pub(super) struct Configure {
    pub size: (u32, u32),
    pub fullscreen: Option<bool>,
}

impl ConfigureBatch {
    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.size = Some((width, height));
    }

    pub(super) fn fullscreen(&mut self, active: bool) {
        // A later state invalidates an earlier queued extent: wait for the
        // configure that follows this state instead of pairing different turns.
        self.size = None;
        self.fullscreen = Some(active);
    }

    pub(super) fn take(&mut self) -> Option<Configure> {
        Some(Configure {
            size: self.size.take()?,
            fullscreen: self.fullscreen.take(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_waits_for_configure_across_dispatch_boundaries() {
        let mut batch = ConfigureBatch::default();
        batch.fullscreen(true);
        assert!(batch.take().is_none()); // callbacks/input may still progress
        batch.resize(1200, 800);
        let frame = batch.take().unwrap();
        assert_eq!(frame.size, (1200, 800));
        assert_eq!(frame.fullscreen, Some(true));
        assert!(batch.take().is_none());
    }

    #[test]
    fn queued_transitions_publish_only_final_size_and_chrome() {
        let mut batch = ConfigureBatch::default();
        batch.resize(1200, 800);
        batch.fullscreen(false);
        batch.resize(800, 600);
        let frame = batch.take().unwrap();
        assert_eq!(frame.size, (800, 600));
        assert_eq!(frame.fullscreen, Some(false));
    }

    #[test]
    fn newer_state_never_uses_an_older_configure() {
        let mut batch = ConfigureBatch::default();
        batch.fullscreen(true);
        batch.resize(1920, 1200);
        batch.fullscreen(false);
        assert!(batch.take().is_none());
        batch.resize(800, 600);
        let frame = batch.take().unwrap();
        assert_eq!(frame.size, (800, 600));
        assert_eq!(frame.fullscreen, Some(false));
    }

    #[test]
    fn configure_before_state_is_not_reused_for_fullscreen() {
        let mut batch = ConfigureBatch::default();
        batch.resize(800, 600);
        assert_eq!(batch.take().unwrap().size, (800, 600));
        batch.fullscreen(true);
        assert!(batch.take().is_none());
        // Every fullscreen reply sends state followed by a configure, even
        // for a refused, redundant or same-extent request.
        batch.resize(1920, 1200);
        assert_eq!(batch.take().unwrap().fullscreen, Some(true));
    }

    #[test]
    fn same_extent_configure_and_plain_resize_are_not_discarded() {
        let mut batch = ConfigureBatch::default();
        for fullscreen in [Some(true), Some(false), None] {
            if let Some(active) = fullscreen {
                batch.fullscreen(active);
            }
            batch.resize(800, 600);
            let frame = batch.take().unwrap();
            assert_eq!(frame.size, (800, 600));
            assert_eq!(frame.fullscreen, fullscreen);
        }
    }
}
