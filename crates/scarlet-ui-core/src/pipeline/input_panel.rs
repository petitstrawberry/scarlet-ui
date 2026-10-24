//! Per-window presentation and keyboard avoidance. Surface size never changes.
#[cfg(test)]
#[path = "input_panel_tests.rs"]
mod tests;

use crate::element::{Element, ElementId, ElementTree};
use crate::event::{Event, Phase};
use crate::geometry::{Point, Rect};

#[derive(Default)]
pub(super) struct OverlayState {
    tablet: bool,
    occlusion: Option<Rect>,
    dirty: bool,
    reveal: bool,
    focus: Option<(ElementId, u32, u32)>,
}

impl OverlayState {
    pub(super) fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::TabletPresentationChanged { active } => {
                self.tablet = *active;
                true
            }
            Event::InputPanelOcclusionChanged { occlusion } => {
                if self.occlusion != *occlusion {
                    self.occlusion = *occlusion;
                    self.dirty = true;
                    self.reveal = occlusion.is_some();
                }
                true
            }
            Event::Keyboard(_)
            | Event::TextInputCommit { .. }
            | Event::TextInputPreedit { .. }
            | Event::TextInputDeleteSurroundingText { .. } => {
                self.reveal = true;
                false
            }
            _ => false,
        }
    }

    pub(super) fn apply_presentation(&self, tree: &mut ElementTree) {
        if let Some(root) = tree.root_mut() {
            apply_tablet(root.as_mut(), self.tablet);
        }
    }

    pub(super) fn apply(&mut self, tree: &mut ElementTree) {
        let Some(root) = tree.root_mut() else { return };
        apply_tablet(root.as_mut(), self.tablet);
        if self.occlusion.is_none() && !self.dirty {
            return;
        }
        let focus = focused_identity(root.as_ref());
        let reveal = self.reveal || focus != self.focus;
        avoid(root.as_mut(), Point::ZERO, self.occlusion, reveal);
        self.focus = focused_identity(root.as_ref());
        self.reveal = false;
        self.dirty = false;
    }
}

fn apply_tablet(element: &mut dyn Element, active: bool) {
    if element.get_window_info().is_some() {
        element.handle_event(&Event::TabletPresentationChanged { active }, Phase::Target);
        return;
    }
    for child in element.children_mut() {
        apply_tablet(child.as_mut(), active);
    }
}

fn focused_identity(element: &dyn Element) -> Option<(ElementId, u32, u32)> {
    if element.wants_keyboard_focus()
        && let Some(state) = element.text_input_state()
    {
        return Some((element.id(), state.cursor_byte, state.anchor_byte));
    }
    element
        .children()
        .iter()
        .find_map(|child| focused_identity(child.as_ref()))
}

/// Return a subtree's focused caret relative to its own origin.
fn caret(element: &dyn Element) -> Option<Rect> {
    if element.wants_keyboard_focus()
        && let Some(state) = element.text_input_state()
    {
        return Some(state.cursor_rect);
    }
    for child in element.children() {
        if let Some(mut rect) = caret(child.as_ref()) {
            let position = child.position();
            rect.origin = Point::new(rect.origin.x + position.x, rect.origin.y + position.y);
            return Some(rect);
        }
    }
    None
}

fn avoid(element: &mut dyn Element, parent: Point, occlusion: Option<Rect>, reveal: bool) {
    let position = element.position();
    let origin = Point::new(parent.x + position.x, parent.y + position.y);
    for child in element.children_mut() {
        avoid(child.as_mut(), origin, occlusion, reveal);
    }
    let local = occlusion.map(|mut rect| {
        rect.origin = Point::new(rect.origin.x - origin.x, rect.origin.y - origin.y);
        rect
    });
    let target = caret(element);
    element.avoid_input_occlusion(local, target, reveal);
}
