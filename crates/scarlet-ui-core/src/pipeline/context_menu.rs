//! Window-local context menu state shared by every backend.
use crate::element::ElementRenderObject;
use crate::event::{Event, FocusEvent, KeyCode, KeyEvent, MouseButton, MouseEvent};
use crate::geometry::{Point, Size};
use crate::renderer::PaintContext;
use crate::views::menu::MenuRenderObject;
use crate::views::{Menu, MenuAction};

pub(super) struct ContextMenuPopup {
    menu: Menu,
    renderer: MenuRenderObject,
    origin: Point,
    pressed: Option<usize>,
}
impl ContextMenuPopup {
    pub fn new(menu: Menu, point: Point, viewport: Size) -> Self {
        let renderer = menu.popup_renderer(viewport.width);
        let size = renderer.size();
        let origin = Point::new(
            point.x.max(0.0).min((viewport.width - size.width).max(0.0)),
            point
                .y
                .max(0.0)
                .min((viewport.height - size.height).max(0.0)),
        );
        Self {
            menu,
            renderer,
            origin,
            pressed: None,
        }
    }
    pub fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>) {
        self.renderer.paint(ctx, self.origin);
    }
    #[allow(deprecated)]
    pub fn composite(&mut self, buffer: &mut crate::buffer::Buffer) {
        self.renderer
            .layout(crate::element::LayoutConstraints::unconstrained());
        self.renderer.render();
        if let Some(source) = self.renderer.get_buffer() {
            buffer.composite(source, self.origin.x as i32, self.origin.y as i32, 1.0);
        }
    }
    fn actionable(&self, index: usize) -> bool {
        self.menu
            .items()
            .get(index)
            .is_some_and(|item| item.enabled_value() && item.action_value() == MenuAction::Item)
    }
    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.renderer
            .hit_test(x as f32 - self.origin.x, y as f32 - self.origin.y)
            .filter(|index| self.actionable(*index))
    }
    /// Returns (consume event, dismiss popup).
    pub fn handle(&mut self, event: &Event) -> (bool, bool) {
        match event {
            Event::Mouse(MouseEvent::Moved { x, y }) => {
                self.renderer.set_hovered(self.hit(*x, *y));
                (true, false)
            }
            Event::Mouse(MouseEvent::ButtonPressed { button, x, y, .. }) => {
                self.pressed = self.hit(*x, *y);
                (
                    true,
                    *button != MouseButton::Left
                        || self
                            .renderer
                            .hit_test(*x as f32 - self.origin.x, *y as f32 - self.origin.y)
                            .is_none(),
                )
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                ..
            }) => {
                if let Some(index) = self.hit(*x, *y)
                    && self.pressed == Some(index)
                {
                    self.renderer.invoke_item(index);
                    return (true, true);
                }
                self.pressed = None;
                (true, false)
            }
            Event::Mouse(MouseEvent::ButtonReleased { .. }) => (true, false),
            Event::Keyboard(KeyEvent::Pressed { keycode, .. }) => {
                match keycode {
                    KeyCode::Escape | KeyCode::Tab => return (true, true),
                    KeyCode::Enter => {
                        if let Some(index) = self.renderer.hovered() {
                            self.renderer.invoke_item(index);
                            return (true, true);
                        }
                    }
                    KeyCode::Up | KeyCode::Down => {
                        let len = self.menu.items().len();
                        let start =
                            self.renderer
                                .hovered()
                                .unwrap_or(if *keycode == KeyCode::Down {
                                    len.saturating_sub(1)
                                } else {
                                    0
                                });
                        for step in 1..=len {
                            let index = if *keycode == KeyCode::Down {
                                (start + step) % len
                            } else {
                                (start + len - step) % len
                            };
                            if self.actionable(index) {
                                self.renderer.set_hovered(Some(index));
                                break;
                            }
                        }
                    }
                    _ => {}
                }
                (true, false)
            }
            Event::Focus(FocusEvent::Lost)
            | Event::Resize { .. }
            | Event::WindowSuspendedChanged { suspended: true } => (false, true),
            Event::Mouse(MouseEvent::Wheel { .. }) => (true, true),
            Event::TextInputCommit { .. }
            | Event::TextInputPreedit { .. }
            | Event::TextInputSelection { .. }
            | Event::TextInputDeleteSurroundingText { .. }
            | Event::TextInputDone { .. }
            | Event::TextInputReveal { .. }
            | Event::Keyboard(_)
            | Event::Mouse(_)
            | Event::TouchFrame(_)
            | Event::Touch(_) => (true, false),
            _ => (false, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::KeyModifiers;
    use crate::views::MenuItemContent;
    use alloc::{rc::Rc, vec};
    use core::cell::Cell;
    fn press(keycode: KeyCode) -> Event {
        Event::Keyboard(KeyEvent::Pressed {
            keycode,
            modifiers: KeyModifiers::default(),
        })
    }
    #[test]
    fn keyboard_skips_disabled_and_separators_and_activates_once() {
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        let mut popup = ContextMenuPopup::new(
            Menu::new(vec![
                MenuItemContent::new("Disabled").enabled(false),
                MenuItemContent::separator(),
                MenuItemContent::new("Run").callback(move || count.set(count.get() + 1)),
            ]),
            Point::ZERO,
            Size::new(400.0, 400.0),
        );
        assert_eq!(popup.handle(&press(KeyCode::Down)), (true, false));
        assert_eq!(popup.renderer.hovered(), Some(2));
        assert_eq!(popup.handle(&press(KeyCode::Enter)), (true, true));
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn modal_popup_blocks_ime_edits_and_closes_on_suspension() {
        let mut popup = ContextMenuPopup::new(
            Menu::new(vec![MenuItemContent::new("Run")]),
            Point::ZERO,
            Size::new(300.0, 200.0),
        );
        assert_eq!(
            popup.handle(&Event::TextInputSelection {
                cursor_byte: 2,
                anchor_byte: 0
            }),
            (true, false)
        );
        assert_eq!(
            popup.handle(&Event::TextInputCommit {
                context_id: 1,
                serial: 1,
                text: "ignored".into()
            }),
            (true, false)
        );
        assert_eq!(
            popup.handle(&Event::WindowSuspendedChanged { suspended: true }),
            (false, true)
        );
    }

    #[test]
    fn clamps_bottom_right_and_dismisses_without_activation() {
        let mut popup = ContextMenuPopup::new(
            Menu::new(vec![MenuItemContent::new("Run")]),
            Point::new(399.0, 299.0),
            Size::new(400.0, 300.0),
        );
        assert!(popup.origin.x + popup.renderer.size().width <= 400.0);
        assert!(popup.origin.y + popup.renderer.size().height <= 300.0);
        assert_eq!(
            popup.handle(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: 0,
                y: 0,
                click_count: 1
            })),
            (true, true)
        );
        assert_eq!(popup.handle(&press(KeyCode::Escape)), (true, true));
    }
    #[test]
    fn release_without_press_does_not_activate() {
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        let mut popup = ContextMenuPopup::new(
            Menu::new(vec![
                MenuItemContent::new("Run").callback(move || count.set(count.get() + 1)),
            ]),
            Point::ZERO,
            Size::new(400.0, 400.0),
        );
        assert_eq!(
            popup.handle(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 20,
                y: 15,
                click_count: 1
            })),
            (true, false)
        );
        assert_eq!(calls.get(), 0);
        popup.handle(&Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 20,
            y: 15,
            click_count: 1,
        }));
        assert_eq!(
            popup.handle(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 20,
                y: 15,
                click_count: 1
            })),
            (true, true)
        );
        assert_eq!(calls.get(), 1);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::super::RenderingPipeline;
    use super::*;
    use crate::Color;
    use crate::view::{View, ViewExt};
    use crate::views::{MenuItemContent, Rectangle};
    use alloc::vec;
    #[test]
    fn popup_paints_above_scene_and_dismissal_repairs_pixels() {
        for paint in [false, true] {
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_paint_enabled(paint);
            pipeline.set_root(
                Rectangle::new()
                    .fill(Color::BLUE)
                    .frame(300.0, 180.0)
                    .create_element(),
            );
            pipeline.layout_initial();
            pipeline.resize(Size::new(300.0, 180.0));
            let before = pipeline.render().expect("CPU frame").as_slice().to_vec();
            pipeline.open_context_menu_at(
                Menu::new(vec![MenuItemContent::new("Command")]),
                Point::new(10.0, 10.0),
            );
            let opened = pipeline.render().expect("open frame").as_slice().to_vec();
            assert!(opened != before, "popup should paint, paint={paint}");
            pipeline.handle_event(&Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Escape,
                modifiers: crate::event::KeyModifiers::default(),
            }));
            let closed = pipeline.render().expect("closed frame").as_slice().to_vec();
            let mismatch = closed.iter().zip(&before).position(|(a, b)| a != b);
            assert_eq!(closed.len(), before.len());
            assert!(
                mismatch.is_none(),
                "dismissal repairs popup pixels, paint={paint}, first mismatch={mismatch:?}"
            );
        }
    }

    #[test]
    fn modifier_opens_pipeline_menu_and_consumes_dismissal_click_pair() {
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            Rectangle::new()
                .fill(Color::BLUE)
                .frame(100.0, 100.0)
                .context_menu(Menu::new(vec![MenuItemContent::new("Custom")]))
                .create_element(),
        );
        pipeline.layout_initial();
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Right,
                x: 20,
                y: 20,
                click_count: 1
            }))
        );
        assert!(pipeline.context_menu.is_some());
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: -1,
                y: -1,
                click_count: 1
            }))
        );
        assert!(pipeline.context_menu.is_none());
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: -1,
                y: -1,
                click_count: 1
            }))
        );
        assert!(!pipeline.context_menu_swallow_release);
    }
}
