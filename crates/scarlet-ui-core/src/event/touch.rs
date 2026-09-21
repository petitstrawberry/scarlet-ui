//! Native direct-touch contacts and a surface-local gesture arena.

use alloc::vec::Vec;

const MOVE_SLOP_SQUARED: i64 = 8 * 8;
const LONG_PRESS_NS: u64 = 600_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchChange {
    pub seat_id: u32,
    pub serial: u64,
    pub time_ns: u64,
    pub id: u64,
    pub phase: TouchPhase,
    pub x: i32,
    pub y: i32,
    pub pressure: Option<i32>,
    pub touch_major: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TouchFrame {
    pub seat_id: u32,
    pub serial: u64,
    pub time_ns: u64,
    pub changes: Vec<TouchChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Axis on which a control may claim a direct-touch drag ahead of scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchDragAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TouchGesture {
    Tap {
        id: u64,
        x: i32,
        y: i32,
    },
    LongPress {
        id: u64,
        x: i32,
        y: i32,
    },
    Scroll {
        id: u64,
        phase: GesturePhase,
        delta_x: i32,
        delta_y: i32,
    },
    /// UI-owned motion after a direct scroll contact has ended.
    ScrollMomentum {
        id: u64,
        phase: GesturePhase,
        delta_x: i32,
        delta_y: i32,
    },
    Drag {
        id: u64,
        phase: GesturePhase,
        x: i32,
        y: i32,
        delta_x: i32,
        delta_y: i32,
    },
    Pinch {
        first_id: u64,
        second_id: u64,
        phase: GesturePhase,
        center_x: i32,
        center_y: i32,
        scale: f32,
    },
    CancelPress {
        id: u64,
    },
}

impl TouchGesture {
    pub const fn primary_id(self) -> u64 {
        match self {
            Self::Tap { id, .. }
            | Self::LongPress { id, .. }
            | Self::Scroll { id, .. }
            | Self::ScrollMomentum { id, .. }
            | Self::Drag { id, .. }
            | Self::CancelPress { id } => id,
            Self::Pinch { first_id, .. } => first_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Pending,
    Scroll,
    Drag,
    Pinch,
    Suppressed,
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u64,
    start_x: i32,
    start_y: i32,
    x: i32,
    y: i32,
    start_ns: u64,
    scrollable: bool,
    drag_axis: Option<TouchDragAxis>,
    mode: Mode,
}

#[derive(Default)]
pub struct TouchArena {
    contacts: Vec<Contact>,
    pinch: Option<(u64, u64, f32, f32)>,
}

impl TouchArena {
    pub fn process(&mut self, change: TouchChange, scrollable: bool) -> Vec<TouchGesture> {
        self.process_with_drag_axis(change, scrollable, None)
    }

    /// Resolve a contact with the target control's optional drag direction.
    /// A perpendicular move may still scroll an ancestor.
    pub fn process_with_drag_axis(
        &mut self,
        change: TouchChange,
        scrollable: bool,
        drag_axis: Option<TouchDragAxis>,
    ) -> Vec<TouchGesture> {
        match change.phase {
            TouchPhase::Down => self.down(change, scrollable, drag_axis),
            TouchPhase::Move => self.moved(change),
            TouchPhase::Up | TouchPhase::Cancel => self.ended(change),
        }
    }

    fn down(
        &mut self,
        change: TouchChange,
        scrollable: bool,
        drag_axis: Option<TouchDragAxis>,
    ) -> Vec<TouchGesture> {
        self.contacts.retain(|contact| contact.id != change.id);
        self.contacts.push(Contact {
            id: change.id,
            start_x: change.x,
            start_y: change.y,
            x: change.x,
            y: change.y,
            start_ns: change.time_ns,
            scrollable,
            drag_axis,
            mode: Mode::Pending,
        });
        if self.contacts.len() == 2 {
            let first = self.contacts[0];
            let second = self.contacts[1];
            self.pinch = Some((first.id, second.id, distance(first, second).max(1.0), 1.0));
            self.contacts
                .iter_mut()
                .for_each(|contact| contact.mode = Mode::Pinch);
            let mut gestures = alloc::vec![
                TouchGesture::CancelPress { id: first.id },
                TouchGesture::CancelPress { id: second.id },
            ];
            if first.mode == Mode::Scroll {
                gestures.push(TouchGesture::Scroll {
                    id: first.id,
                    phase: GesturePhase::Cancelled,
                    delta_x: 0,
                    delta_y: 0,
                });
            } else if first.mode == Mode::Drag {
                gestures.push(TouchGesture::Drag {
                    id: first.id,
                    phase: GesturePhase::Cancelled,
                    x: first.x,
                    y: first.y,
                    delta_x: 0,
                    delta_y: 0,
                });
            }
            return gestures;
        }
        if self.contacts.len() > 2 {
            self.contacts.last_mut().unwrap().mode = Mode::Suppressed;
            return alloc::vec![TouchGesture::CancelPress { id: change.id }];
        }
        Vec::new()
    }

    fn moved(&mut self, change: TouchChange) -> Vec<TouchGesture> {
        let Some(index) = self
            .contacts
            .iter()
            .position(|contact| contact.id == change.id)
        else {
            return Vec::new();
        };
        let previous = self.contacts[index];
        self.contacts[index].x = change.x;
        self.contacts[index].y = change.y;
        if let Some((first_id, second_id, baseline, _)) = self.pinch
            && (change.id == first_id || change.id == second_id)
            && let (Some(first), Some(second)) = (
                self.contacts.iter().find(|contact| contact.id == first_id),
                self.contacts.iter().find(|contact| contact.id == second_id),
            )
        {
            let scale = distance(*first, *second) / baseline;
            if let Some((_, _, _, last_scale)) = self.pinch.as_mut() {
                *last_scale = scale;
            }
            return alloc::vec![TouchGesture::Pinch {
                first_id,
                second_id,
                phase: GesturePhase::Moved,
                center_x: first.x.saturating_add(second.x) / 2,
                center_y: first.y.saturating_add(second.y) / 2,
                scale,
            }];
        }
        let delta_x = change.x.saturating_sub(previous.x);
        let delta_y = change.y.saturating_sub(previous.y);
        match previous.mode {
            Mode::Pending => {
                let total_x = i64::from(change.x) - i64::from(previous.start_x);
                let total_y = i64::from(change.y) - i64::from(previous.start_y);
                if total_x * total_x + total_y * total_y < MOVE_SLOP_SQUARED {
                    return Vec::new();
                }
                let drag_claimed = !previous.scrollable
                    || match previous.drag_axis {
                        Some(TouchDragAxis::Horizontal) => total_x.abs() > total_y.abs(),
                        Some(TouchDragAxis::Vertical) => total_y.abs() > total_x.abs(),
                        None => false,
                    };
                self.contacts[index].mode = if drag_claimed {
                    Mode::Drag
                } else {
                    Mode::Scroll
                };
                let gesture = if !drag_claimed {
                    TouchGesture::Scroll {
                        id: change.id,
                        phase: GesturePhase::Started,
                        delta_x: change.x.saturating_sub(previous.start_x),
                        delta_y: change.y.saturating_sub(previous.start_y),
                    }
                } else {
                    TouchGesture::Drag {
                        id: change.id,
                        phase: GesturePhase::Started,
                        x: change.x,
                        y: change.y,
                        delta_x: change.x.saturating_sub(previous.start_x),
                        delta_y: change.y.saturating_sub(previous.start_y),
                    }
                };
                alloc::vec![TouchGesture::CancelPress { id: change.id }, gesture]
            }
            Mode::Scroll => alloc::vec![TouchGesture::Scroll {
                id: change.id,
                phase: GesturePhase::Moved,
                delta_x,
                delta_y,
            }],
            Mode::Drag => alloc::vec![TouchGesture::Drag {
                id: change.id,
                phase: GesturePhase::Moved,
                x: change.x,
                y: change.y,
                delta_x,
                delta_y,
            }],
            Mode::Pinch | Mode::Suppressed => Vec::new(),
        }
    }

    fn ended(&mut self, change: TouchChange) -> Vec<TouchGesture> {
        let Some(index) = self
            .contacts
            .iter()
            .position(|contact| contact.id == change.id)
        else {
            return Vec::new();
        };
        let contact = self.contacts.remove(index);
        if let Some((first_id, second_id, _, last_scale)) = self.pinch
            && (change.id == first_id || change.id == second_id)
        {
            self.pinch = None;
            self.contacts
                .iter_mut()
                .for_each(|contact| contact.mode = Mode::Suppressed);
            return alloc::vec![TouchGesture::Pinch {
                first_id,
                second_id,
                phase: if change.phase == TouchPhase::Cancel {
                    GesturePhase::Cancelled
                } else {
                    GesturePhase::Ended
                },
                center_x: change.x,
                center_y: change.y,
                scale: last_scale,
            }];
        }
        let terminal_phase = if change.phase == TouchPhase::Cancel {
            GesturePhase::Cancelled
        } else {
            GesturePhase::Ended
        };
        match contact.mode {
            Mode::Pending if change.phase == TouchPhase::Cancel => {
                alloc::vec![TouchGesture::CancelPress { id: change.id }]
            }
            Mode::Pending => {
                let total_x = i64::from(change.x) - i64::from(contact.start_x);
                let total_y = i64::from(change.y) - i64::from(contact.start_y);
                if total_x * total_x + total_y * total_y >= MOVE_SLOP_SQUARED {
                    return alloc::vec![TouchGesture::CancelPress { id: change.id }];
                }
                if change.time_ns.saturating_sub(contact.start_ns) >= LONG_PRESS_NS {
                    alloc::vec![TouchGesture::LongPress {
                        id: change.id,
                        x: change.x,
                        y: change.y
                    }]
                } else {
                    alloc::vec![TouchGesture::Tap {
                        id: change.id,
                        x: change.x,
                        y: change.y
                    }]
                }
            }
            Mode::Scroll => {
                let mut gestures = Vec::new();
                let delta_x = change.x.saturating_sub(contact.x);
                let delta_y = change.y.saturating_sub(contact.y);
                if change.phase == TouchPhase::Up && (delta_x != 0 || delta_y != 0) {
                    gestures.push(TouchGesture::Scroll {
                        id: change.id,
                        phase: GesturePhase::Moved,
                        delta_x,
                        delta_y,
                    });
                }
                gestures.push(TouchGesture::Scroll {
                    id: change.id,
                    phase: terminal_phase,
                    delta_x: 0,
                    delta_y: 0,
                });
                gestures
            }
            Mode::Drag => alloc::vec![TouchGesture::Drag {
                id: change.id,
                phase: terminal_phase,
                x: change.x,
                y: change.y,
                delta_x: 0,
                delta_y: 0,
            }],
            Mode::Pinch | Mode::Suppressed => Vec::new(),
        }
    }
}

fn distance(first: Contact, second: Contact) -> f32 {
    let x = first.x as f32 - second.x as f32;
    let y = first.y as f32 - second.y as f32;
    libm::sqrtf(x * x + y * y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(id: u64, phase: TouchPhase, x: i32, y: i32, time_ns: u64) -> TouchChange {
        TouchChange {
            seat_id: 0,
            serial: 1,
            time_ns,
            id,
            phase,
            x,
            y,
            pressure: None,
            touch_major: None,
        }
    }

    #[test]
    fn tap_cancels_when_scroll_wins() {
        let mut arena = TouchArena::default();
        assert!(
            arena
                .process(change(1, TouchPhase::Down, 0, 0, 1), true)
                .is_empty()
        );
        assert!(
            arena
                .process(change(1, TouchPhase::Move, 0, 3, 2), false)
                .is_empty()
        );
        assert_eq!(
            arena.process(change(1, TouchPhase::Move, 0, 12, 3), false),
            alloc::vec![
                TouchGesture::CancelPress { id: 1 },
                TouchGesture::Scroll {
                    id: 1,
                    phase: GesturePhase::Started,
                    delta_x: 0,
                    delta_y: 12
                },
            ]
        );
        assert_eq!(
            arena.process(change(1, TouchPhase::Up, 0, 12, 4), false),
            alloc::vec![TouchGesture::Scroll {
                id: 1,
                phase: GesturePhase::Ended,
                delta_x: 0,
                delta_y: 0
            },]
        );
    }

    #[test]
    fn second_contact_rejects_taps_and_reports_pinch() {
        let mut arena = TouchArena::default();
        arena.process(change(1, TouchPhase::Down, 0, 0, 1), false);
        assert_eq!(
            arena.process(change(2, TouchPhase::Down, 10, 0, 2), false),
            alloc::vec![
                TouchGesture::CancelPress { id: 1 },
                TouchGesture::CancelPress { id: 2 },
            ]
        );
        assert!(
            matches!(arena.process(change(2, TouchPhase::Move, 20, 0, 3), false).as_slice(),
            [TouchGesture::Pinch { scale, .. }] if (*scale - 2.0).abs() < 0.001)
        );
        assert!(matches!(
            arena.process(change(2, TouchPhase::Up, 20, 0, 4), false).as_slice(),
            [TouchGesture::Pinch { phase: GesturePhase::Ended, scale, .. }]
                if (*scale - 2.0).abs() < 0.001
        ));
        assert!(
            arena
                .process(change(1, TouchPhase::Up, 0, 0, 5), false)
                .is_empty()
        );
    }

    #[test]
    fn second_contact_cancels_an_active_scroll() {
        let mut arena = TouchArena::default();
        arena.process(change(1, TouchPhase::Down, 0, 0, 1), true);
        arena.process(change(1, TouchPhase::Move, 0, 20, 2), false);
        assert!(
            arena
                .process(change(2, TouchPhase::Down, 20, 0, 3), false)
                .contains(&TouchGesture::Scroll {
                    id: 1,
                    phase: GesturePhase::Cancelled,
                    delta_x: 0,
                    delta_y: 0,
                })
        );
        assert!(
            arena
                .process(change(1, TouchPhase::Up, 0, 20, 4), false)
                .iter()
                .all(|gesture| !matches!(gesture, TouchGesture::Scroll { .. }))
        );
    }

    #[test]
    fn release_includes_final_contact_motion_before_ending_scroll() {
        let mut arena = TouchArena::default();
        arena.process(change(1, TouchPhase::Down, 0, 0, 1), true);
        arena.process(change(1, TouchPhase::Move, 0, 20, 2), false);
        assert_eq!(
            arena.process(change(1, TouchPhase::Up, 0, 30, 3), false),
            alloc::vec![
                TouchGesture::Scroll {
                    id: 1,
                    phase: GesturePhase::Moved,
                    delta_x: 0,
                    delta_y: 10,
                },
                TouchGesture::Scroll {
                    id: 1,
                    phase: GesturePhase::Ended,
                    delta_x: 0,
                    delta_y: 0,
                },
            ]
        );
    }

    #[test]
    fn control_drag_direction_competes_with_ancestor_scroll() {
        let mut horizontal = TouchArena::default();
        horizontal.process_with_drag_axis(
            change(1, TouchPhase::Down, 20, 20, 1),
            true,
            Some(TouchDragAxis::Horizontal),
        );
        assert_eq!(
            horizontal.process(change(1, TouchPhase::Move, 40, 22, 2), false),
            alloc::vec![
                TouchGesture::CancelPress { id: 1 },
                TouchGesture::Drag {
                    id: 1,
                    phase: GesturePhase::Started,
                    x: 40,
                    y: 22,
                    delta_x: 20,
                    delta_y: 2,
                },
            ]
        );

        let mut vertical = TouchArena::default();
        vertical.process_with_drag_axis(
            change(2, TouchPhase::Down, 20, 20, 1),
            true,
            Some(TouchDragAxis::Horizontal),
        );
        assert_eq!(
            vertical.process(change(2, TouchPhase::Move, 22, 40, 2), false),
            alloc::vec![
                TouchGesture::CancelPress { id: 2 },
                TouchGesture::Scroll {
                    id: 2,
                    phase: GesturePhase::Started,
                    delta_x: 2,
                    delta_y: 20,
                },
            ]
        );
    }

    #[test]
    fn second_contact_cancels_an_active_control_drag() {
        let mut arena = TouchArena::default();
        arena.process_with_drag_axis(
            change(1, TouchPhase::Down, 20, 20, 1),
            true,
            Some(TouchDragAxis::Horizontal),
        );
        arena.process(change(1, TouchPhase::Move, 40, 20, 2), false);
        assert!(
            arena
                .process(change(2, TouchPhase::Down, 60, 20, 3), false)
                .contains(&TouchGesture::Drag {
                    id: 1,
                    phase: GesturePhase::Cancelled,
                    x: 40,
                    y: 20,
                    delta_x: 0,
                    delta_y: 0,
                })
        );
        assert!(
            arena
                .process(change(1, TouchPhase::Up, 40, 20, 4), false)
                .iter()
                .all(|gesture| !matches!(gesture, TouchGesture::Drag { .. }))
        );
    }
}
