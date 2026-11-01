//! Browser-independent PointerEvent contact bookkeeping, shared with host tests.
use scarlet_ui_core::event::{TouchChange, TouchFrame, TouchPhase};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct TouchContacts {
    contacts: BTreeMap<i32, (i32, i32)>,
    serial: u64,
    time_ns: u64,
}
impl TouchContacts {
    fn frame(
        &mut self,
        changes: Vec<(i32, TouchPhase, i32, i32)>,
        time_ms: f64,
    ) -> Option<TouchFrame> {
        if changes.is_empty() {
            return None;
        }
        self.serial = self.serial.saturating_add(1);
        let timestamp = if time_ms.is_finite() && time_ms >= 0.0 {
            (time_ms * 1_000_000.0) as u64
        } else {
            self.time_ns
        };
        self.time_ns = timestamp.max(self.time_ns.saturating_add(1));
        Some(TouchFrame {
            seat_id: 0,
            serial: self.serial,
            time_ns: self.time_ns,
            changes: changes
                .into_iter()
                .map(|(id, phase, x, y)| TouchChange {
                    seat_id: 0,
                    serial: self.serial,
                    time_ns: self.time_ns,
                    // Preserve the full signed browser pointer ID without collisions.
                    id: id as u32 as u64,
                    phase,
                    x,
                    y,
                    // Browser normalized pressure has no core-defined physical unit.
                    pressure: None,
                    touch_major: None,
                })
                .collect(),
        })
    }
    pub(crate) fn pointer(
        &mut self,
        kind: &str,
        id: i32,
        x: i32,
        y: i32,
        time_ms: f64,
    ) -> Option<TouchFrame> {
        let phase = match kind {
            "pointerdown" => {
                if self.contacts.contains_key(&id) {
                    return None;
                }
                self.contacts.insert(id, (x, y));
                TouchPhase::Down
            }
            "pointermove" => {
                *self.contacts.get_mut(&id)? = (x, y);
                TouchPhase::Move
            }
            "pointerup" => {
                self.contacts.remove(&id)?;
                TouchPhase::Up
            }
            "pointercancel" | "lostpointercapture" => {
                let (x, y) = self.contacts.remove(&id)?;
                return self.frame(vec![(id, TouchPhase::Cancel, x, y)], time_ms);
            }
            // Captured contact continues beyond the canvas; leave is not cancel.
            _ => return None,
        };
        self.frame(vec![(id, phase, x, y)], time_ms)
    }
    pub(crate) fn cancel_all(&mut self, time_ms: f64) -> Option<TouchFrame> {
        let changes = std::mem::take(&mut self.contacts)
            .into_iter()
            .map(|(id, (x, y))| (id, TouchPhase::Cancel, x, y))
            .collect();
        self.frame(changes, time_ms)
    }
}

/// Menus sit outside the scene's TouchArena. Only a contact claimed on Down is
/// translated for that existing menu controller; scene contacts remain touch.
#[derive(Default)]
pub(crate) struct MenuTouchRouting {
    owned: std::collections::BTreeSet<u64>,
}
impl MenuTouchRouting {
    pub(crate) fn route(
        &mut self,
        frame: &mut TouchFrame,
        mut handle: impl FnMut(&scarlet_ui_core::event::Event) -> (bool, Option<String>),
    ) -> Option<String> {
        use scarlet_ui_core::event::{Event, MouseButton, MouseEvent};
        let mut action = None;
        frame.changes.retain(|change| {
            let mouse = match change.phase {
                TouchPhase::Down => MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: change.x,
                    y: change.y,
                    click_count: 1,
                },
                TouchPhase::Move => MouseEvent::Moved {
                    x: change.x,
                    y: change.y,
                },
                TouchPhase::Up => MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: change.x,
                    y: change.y,
                    click_count: 1,
                },
                TouchPhase::Cancel => MouseEvent::ButtonCancelled {
                    button: MouseButton::Left,
                    x: change.x,
                    y: change.y,
                },
            };
            if change.phase == TouchPhase::Down {
                let (consumed, activated) = handle(&Event::Mouse(mouse));
                action = action.take().or(activated);
                if consumed {
                    self.owned.insert(change.id);
                }
                return !consumed;
            }
            if !self.owned.contains(&change.id) {
                return true;
            }
            let (_, activated) = handle(&Event::Mouse(mouse));
            action = action.take().or(activated);
            if matches!(change.phase, TouchPhase::Up | TouchPhase::Cancel) {
                self.owned.remove(&change.id);
            }
            false
        });
        action
    }
}
