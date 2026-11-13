//! One-shot display grants, separate from synchronous input ticks.
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
enum Demand {
    #[default]
    Idle,
    Requested,
    Ready(u64),
}

#[derive(Default)]
pub(crate) struct FramePacing {
    demand: Cell<Demand>,
    paused: Cell<bool>,
}

impl FramePacing {
    pub fn request(&self) {
        if matches!(self.demand.get(), Demand::Idle) {
            self.demand.set(Demand::Requested);
        }
    }

    /// Called only by the browser's animation-frame callback. Requesting or
    /// polling a frame never manufactures a display opportunity.
    pub fn on_animation_frame(&self, timestamp_ms: f64) {
        if !self.is_paused() && matches!(self.demand.get(), Demand::Requested) {
            self.demand
                .set(Demand::Ready((timestamp_ms.max(0.0) * 1_000_000.0) as u64));
        }
    }

    pub fn take_ready(&self) -> Option<u64> {
        if let Demand::Ready(timestamp_ns) = self.demand.get() {
            self.demand.set(Demand::Idle);
            Some(timestamp_ns)
        } else {
            None
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.get()
    }

    pub fn pause(&self) {
        self.paused.set(true);
        // Keep the runner's outstanding request, but replace any undelivered
        // grant with a fresh rAF opportunity after resume.
        if matches!(self.demand.get(), Demand::Ready(_)) {
            self.demand.set(Demand::Requested);
        }
    }

    pub fn resume(&self) {
        self.paused.set(false);
    }
}
