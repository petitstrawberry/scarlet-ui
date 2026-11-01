//! Browser presentation of the shared ScarletUI menu model and menu widgets.
use scarlet_ui_core::{
    View,
    color::ColorPalette,
    element::{Element, ElementRenderObject, LayoutConstraints},
    event::{Event, FocusEvent, KeyCode, KeyEvent, MouseButton, MouseEvent},
    geometry::{Point, Rect, Size},
    menu_model::{MenuBarModel, MenuEntry, MenuItemModel},
    renderer::PaintContext,
    views::{Menu, MenuBar, MenuItem, MenuItemContent, menu::MenuRenderObject},
};

const BAR_INSET_X: f32 = 8.0;
const BAR_INSET_Y: f32 = 2.0;
const BAR_HEIGHT: f32 = 28.0;
const POPUP_GAP: f32 = 4.0;
const CANVAS_MARGIN: f32 = 6.0;

/// Tracks only the scene/menu hover boundary; it does not own pointer capture.
#[derive(Default)]
pub(crate) struct HoverHandoff {
    scene_hover: bool,
}
impl HoverHandoff {
    pub fn route(
        &mut self,
        mouse: &MouseEvent,
        consumed: bool,
        content_offset: i32,
    ) -> Option<MouseEvent> {
        if consumed {
            if let MouseEvent::Moved { x, y } | MouseEvent::Entered { x, y } = mouse {
                if std::mem::take(&mut self.scene_hover) {
                    return Some(MouseEvent::Exited {
                        x: *x,
                        y: *y - content_offset,
                    });
                }
            }
        } else {
            match mouse {
                MouseEvent::Exited { .. } | MouseEvent::ButtonCancelled { .. } => {
                    self.scene_hover = false
                }
                MouseEvent::RelativeMotion { .. } => {}
                _ => self.scene_hover = true,
            }
        }
        None
    }
}

struct Popup {
    entries: Vec<MenuEntry>,
    render: MenuRenderObject,
    origin: Point,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Bar(usize),
    Row(usize, usize),
}

pub(crate) struct WebMenu {
    model: MenuBarModel,
    model_json: String,
    bar: Option<Box<dyn Element>>,
    bar_bounds: Vec<Rect>,
    popups: Vec<Popup>,
    top: Option<usize>,
    hovered_bar: Option<usize>,
    pressed: Option<Hit>,
    size: Size,
    dirty: bool,
    suppress_text_until_keyup: bool,
    suppressed_release: Option<MouseButton>,
}
impl Default for WebMenu {
    fn default() -> Self {
        Self {
            model: MenuBarModel::default(),
            model_json: String::new(),
            bar: None,
            bar_bounds: Vec::new(),
            popups: Vec::new(),
            top: None,
            hovered_bar: None,
            pressed: None,
            size: Size::ZERO,
            dirty: false,
            suppress_text_until_keyup: false,
            suppressed_release: None,
        }
    }
}
impl WebMenu {
    pub fn has_menu(&self) -> bool {
        !self.model.items().is_empty()
    }
    pub fn height(&self) -> f32 {
        if self.has_menu() { BAR_HEIGHT } else { 0.0 }
    }
    pub fn content_size(&self, canvas: Size) -> Size {
        Size::new(canvas.width, (canvas.height - self.height()).max(1.0))
    }
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
    pub fn set_model(&mut self, model: &MenuBarModel) {
        let json = model.to_json();
        self.model = model.clone();
        if self.model_json != json {
            self.model_json = json;
            self.close();
            self.rebuild_bar();
            self.dirty = true;
        }
    }
    pub fn resize(&mut self, size: Size) {
        if self.size != size {
            self.size = size;
            self.close();
            self.rebuild_bar();
            self.dirty = self.has_menu();
        }
    }
    fn rebuild_bar(&mut self) {
        self.bar_bounds.clear();
        if !self.has_menu() {
            self.bar = None;
            return;
        }
        let items = self
            .model
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let palette = ColorPalette::default();
                let active = self.top == Some(index);
                let view = MenuItem::new(item.title())
                    .font_size(13.0)
                    .padding(10.0)
                    .selected(item.enabled_value() && (active || self.hovered_bar == Some(index)))
                    .interaction_background_colors(
                        palette.menu_hover(),
                        if active {
                            palette.menu_active()
                        } else {
                            palette.menu_hover()
                        },
                    );
                if item.enabled_value() {
                    view
                } else {
                    view.foreground_color(ColorPalette::default().text_tertiary())
                }
            })
            .collect();
        let mut bar = MenuBar::new(items).spacing(2.0).create_element();
        bar.layout(LayoutConstraints {
            min_width: 0.0,
            max_width: (self.size.width - BAR_INSET_X * 2.0).max(1.0),
            min_height: self.height() - BAR_INSET_Y * 2.0,
            max_height: self.height() - BAR_INSET_Y * 2.0,
        });
        // Paint and hit testing share the same inset positions.
        for child in bar.children_mut() {
            let origin = child.position();
            child.set_position(Point::new(origin.x + BAR_INSET_X, origin.y + BAR_INSET_Y));
        }
        self.bar_bounds = bar.children().iter().map(|child| child.bounds()).collect();
        self.bar = Some(bar);
    }
    fn close(&mut self) {
        if !self.popups.is_empty() || self.top.is_some() || self.hovered_bar.is_some() {
            self.popups.clear();
            self.top = None;
            self.hovered_bar = None;
            self.pressed = None;
            self.dirty = true;
            self.rebuild_bar();
        }
    }
    fn open_popup(&mut self, entries: Vec<MenuEntry>, preferred: Point, parent_left: Option<f32>) {
        let contents: Vec<_> = entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.to_menu_content(),
                MenuEntry::Separator => MenuItemContent::separator(),
            })
            .collect();
        let margin_x = CANVAS_MARGIN.min((self.size.width - 1.0).max(0.0) / 2.0);
        let width = Menu::new(contents.clone())
            .preferred_width()
            .min((self.size.width - margin_x * 2.0).max(1.0));
        let render = MenuRenderObject::new_adaptive(contents, width);
        let extent = render.size();
        let mut x = preferred.x;
        if x + extent.width > self.size.width - margin_x {
            x = parent_left
                .map(|left| left - extent.width - POPUP_GAP)
                .unwrap_or(self.size.width - extent.width - margin_x);
        }
        // Align the first submenu row to its parent row, excluding shared padding.
        let y = preferred.y
            - if parent_left.is_some() {
                render.item_rect(0).map_or(0.0, |rect| rect.origin.y)
            } else {
                0.0
            };
        let margin_y = CANVAS_MARGIN.min((self.size.height - extent.height).max(0.0) / 2.0);
        let origin = Point::new(
            x.max(margin_x),
            y.min((self.size.height - extent.height - margin_y).max(margin_y))
                .max(margin_y),
        );
        self.popups.push(Popup {
            entries,
            render,
            origin,
        });
        self.dirty = true;
    }
    fn open_top(&mut self, index: usize) {
        let Some(item) = self.model.items().get(index).cloned() else {
            return;
        };
        if !item.enabled_value() {
            return;
        }
        self.popups.clear();
        self.top = Some(index);
        if !item.children_value().is_empty() {
            let x = self.bar_bounds.get(index).map_or(0.0, |rect| rect.origin.x);
            self.open_popup(
                item.children_value().to_vec(),
                Point::new(x, self.height() + POPUP_GAP),
                None,
            );
        }
        self.rebuild_bar();
        self.dirty = true;
    }
    fn hit(&self, point: Point) -> Option<Hit> {
        for (depth, popup) in self.popups.iter().enumerate().rev() {
            if let Some(index) = popup
                .render
                .hit_test(point.x - popup.origin.x, point.y - popup.origin.y)
            {
                return Some(Hit::Row(depth, index));
            }
            // Popup padding still blocks clicks into the scene underneath.
            if Rect::new(popup.origin, popup.render.size()).contains(point) {
                return None;
            }
        }
        self.bar_bounds
            .iter()
            .position(|rect| rect.contains(point))
            .map(Hit::Bar)
    }
    fn row(&self, depth: usize, index: usize) -> Option<MenuItemModel> {
        match self.popups.get(depth)?.entries.get(index)? {
            MenuEntry::Item(item) if item.enabled_value() => Some(item.clone()),
            _ => None,
        }
    }
    fn hover_row(&mut self, depth: usize, index: usize) {
        let old = self.popups[depth].render.hovered();
        if old == Some(index) {
            return;
        }
        self.popups.truncate(depth + 1);
        self.popups[depth].render.set_hovered(Some(index));
        self.dirty = true;
        if let Some(item) = self.row(depth, index) {
            if !item.children_value().is_empty() {
                let popup = &self.popups[depth];
                let row = popup
                    .render
                    .item_rect(index)
                    .unwrap_or(Rect::new(Point::ZERO, Size::ZERO));
                let next = Point::new(
                    popup.origin.x + popup.render.size().width + POPUP_GAP,
                    popup.origin.y + row.origin.y,
                );
                let left = popup.origin.x;
                self.open_popup(item.children_value().to_vec(), next, Some(left));
            }
        }
    }
    fn activate(&mut self, hit: Hit) -> Option<String> {
        let item = match hit {
            Hit::Bar(index) => self
                .model
                .items()
                .get(index)
                .filter(|i| i.enabled_value())
                .cloned(),
            Hit::Row(depth, index) => self.row(depth, index),
        }?;
        if !item.children_value().is_empty() {
            return None;
        }
        let id = item.id().to_owned();
        self.close();
        Some(id)
    }
    /// Returns whether input was consumed, plus an action for the shared registry.
    pub fn handle(&mut self, event: &Event) -> (bool, Option<String>) {
        if !self.has_menu() {
            return (false, None);
        }
        if self.suppress_text_until_keyup {
            match event {
                Event::Keyboard(KeyEvent::Char { .. }) => return (true, None),
                Event::Keyboard(KeyEvent::Released { .. }) => {
                    self.suppress_text_until_keyup = false;
                    return (true, None);
                }
                _ => {}
            }
        }
        match event {
            Event::Focus(FocusEvent::Lost) => {
                self.suppress_text_until_keyup = false;
                self.suppressed_release = None;
                self.close();
                (false, None)
            }
            Event::Keyboard(KeyEvent::Pressed { keycode, modifiers })
                if !self.popups.is_empty() || self.top.is_some() =>
            {
                if modifiers.control || modifiers.alt || modifiers.super_key {
                    return (false, None);
                }
                match keycode {
                    KeyCode::Escape | KeyCode::Tab => {
                        self.close();
                        (true, None)
                    }
                    KeyCode::Up | KeyCode::Down => {
                        if let Some(depth) = self.popups.len().checked_sub(1) {
                            let popup = &self.popups[depth];
                            let enabled: Vec<_> = popup
                                .entries
                                .iter()
                                .enumerate()
                                .filter_map(|(i, e)| match e {
                                    MenuEntry::Item(item) if item.enabled_value() => Some(i),
                                    _ => None,
                                })
                                .collect();
                            if !enabled.is_empty() {
                                let current = popup
                                    .render
                                    .hovered()
                                    .and_then(|i| enabled.iter().position(|x| *x == i));
                                let next = match (current, keycode) {
                                    (Some(i), KeyCode::Up) => {
                                        (i + enabled.len() - 1) % enabled.len()
                                    }
                                    (Some(i), _) => (i + 1) % enabled.len(),
                                    (None, KeyCode::Up) => enabled.len() - 1,
                                    _ => 0,
                                };
                                self.popups[depth].render.set_hovered(Some(enabled[next]));
                                self.dirty = true;
                            }
                        }
                        (true, None)
                    }
                    KeyCode::Enter | KeyCode::Space | KeyCode::Right => {
                        self.suppress_text_until_keyup = true;
                        if let Some(depth) = self.popups.len().checked_sub(1) {
                            if let Some(index) = self.popups[depth].render.hovered() {
                                if let Some(item) = self.row(depth, index) {
                                    if !item.children_value().is_empty() {
                                        self.popups[depth].render.set_hovered(None);
                                        self.hover_row(depth, index);
                                        return (true, None);
                                    }
                                    if *keycode != KeyCode::Right {
                                        return (true, self.activate(Hit::Row(depth, index)));
                                    }
                                }
                            }
                        }
                        if *keycode == KeyCode::Right {
                            self.step_top(1);
                        }
                        (true, None)
                    }
                    KeyCode::Left => {
                        if self.popups.len() > 1 {
                            self.popups.pop();
                            if let Some(parent) = self.popups.last_mut() {
                                parent.render.set_hovered(None);
                            }
                            self.dirty = true;
                        } else {
                            self.step_top(-1);
                        }
                        (true, None)
                    }
                    _ => (false, None),
                }
            }
            Event::Keyboard(KeyEvent::Char { .. } | KeyEvent::Released { .. })
                if !self.popups.is_empty() =>
            {
                (true, None)
            }
            Event::Mouse(mouse) => self.mouse(mouse),
            _ => (false, None),
        }
    }
    fn step_top(&mut self, direction: isize) {
        let Some(current) = self.top else {
            return;
        };
        let count = self.model.items().len();
        for step in 1..=count {
            let next =
                (current as isize + direction * step as isize).rem_euclid(count as isize) as usize;
            if self.model.items()[next].enabled_value() {
                self.open_top(next);
                break;
            }
        }
    }
    fn mouse(&mut self, mouse: &MouseEvent) -> (bool, Option<String>) {
        if let MouseEvent::ButtonReleased { button, .. }
        | MouseEvent::ButtonCancelled { button, .. } = mouse
        {
            if self.suppressed_release == Some(*button) {
                self.suppressed_release = None;
                return (true, None);
            }
        }
        let (x, y) = match mouse {
            MouseEvent::Moved { x, y }
            | MouseEvent::Entered { x, y }
            | MouseEvent::Exited { x, y }
            | MouseEvent::ButtonPressed { x, y, .. }
            | MouseEvent::ButtonReleased { x, y, .. }
            | MouseEvent::ButtonCancelled { x, y, .. }
            | MouseEvent::Wheel { x, y, .. } => (*x as f32, *y as f32),
            _ => return (false, None),
        };
        let point = Point::new(x, y);
        let hit = self.hit(point);
        let over_menu = y < self.height()
            || self
                .popups
                .iter()
                .any(|p| Rect::new(p.origin, p.render.size()).contains(point));
        match mouse {
            MouseEvent::ButtonPressed {
                button: MouseButton::Right,
                ..
            } => {
                // Application menus already have a visible menubar. A context
                // click belongs to the scene/browser, not a duplicate menu tree.
                self.close();
                (false, None)
            }
            MouseEvent::ButtonReleased {
                button: MouseButton::Right,
                ..
            } => (false, None),
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                ..
            } => {
                if let Some(Hit::Bar(index)) = hit {
                    if self.top == Some(index) && !self.popups.is_empty() {
                        self.close();
                    } else {
                        self.open_top(index);
                    }
                }
                if let Some(Hit::Row(depth, index)) = hit {
                    self.hover_row(depth, index);
                }
                if !over_menu && !self.popups.is_empty() {
                    self.close();
                    self.suppressed_release = Some(MouseButton::Left);
                    return (true, None);
                }
                self.pressed = hit;
                (over_menu, None)
            }
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                ..
            } => {
                let pressed = self.pressed.take();
                let action = if hit.is_some() && hit == pressed {
                    self.activate(hit.unwrap())
                } else {
                    None
                };
                (over_menu || pressed.is_some(), action)
            }
            MouseEvent::Moved { .. } | MouseEvent::Entered { .. } => {
                let bar = if let Some(Hit::Bar(index)) = hit {
                    Some(index)
                } else {
                    None
                };
                if bar != self.hovered_bar {
                    self.hovered_bar = bar;
                    self.rebuild_bar();
                    self.dirty = true;
                }
                if let Some(Hit::Bar(index)) = hit {
                    if self.top.is_some() && self.top != Some(index) {
                        self.open_top(index);
                    }
                } else if let Some(Hit::Row(depth, index)) = hit {
                    self.hover_row(depth, index);
                }
                (over_menu || !self.popups.is_empty(), None)
            }
            MouseEvent::ButtonCancelled { .. } => {
                self.pressed = None;
                (over_menu, None)
            }
            MouseEvent::Exited { .. } => {
                self.hovered_bar = None;
                self.rebuild_bar();
                self.dirty = true;
                (false, None)
            }
            MouseEvent::Wheel { .. } => (over_menu || !self.popups.is_empty(), None),
            _ => (over_menu, None),
        }
    }
    pub fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>) {
        if !self.has_menu() {
            return;
        }
        ctx.fill_rect(
            Rect::from_xywh(0.0, 0.0, self.size.width, self.height()),
            ColorPalette::default().surface(),
        );
        ctx.fill_rect(
            Rect::from_xywh(0.0, self.height() - 1.0, self.size.width, 1.0),
            ColorPalette::default().divider(),
        );
        if let Some(bar) = &self.bar {
            for child in bar.children() {
                if let Some(render) = child.render_object() {
                    render.paint(ctx, child.position());
                }
            }
        }
        for popup in &self.popups {
            popup.render.paint(ctx, popup.origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::event::KeyModifiers;

    fn menu() -> WebMenu {
        scarlet_ui_core::graphics::set_default_font(include_bytes!(
            "../../../examples/web-counter/assets/DejaVuSans.ttf"
        ))
        .unwrap();
        let mut menu = WebMenu::default();
        menu.resize(Size::new(640.0, 480.0));
        menu.set_model(&MenuBarModel::new(vec![
            MenuItemModel::new("file", "File").children(vec![
                MenuEntry::Item(MenuItemModel::new("off", "Disabled").enabled(false)),
                MenuEntry::Item(MenuItemModel::new("open", "Open")),
                MenuEntry::Separator,
                MenuEntry::Item(
                    MenuItemModel::new("sub", "More").children(vec![MenuEntry::Item(
                        MenuItemModel::new("nested", "Nested action"),
                    )]),
                ),
            ]),
        ]));
        menu.take_dirty();
        menu
    }
    fn mouse(menu: &mut WebMenu, down: bool, point: Point) -> (bool, Option<String>) {
        let mouse = if down {
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: point.x as i32,
                y: point.y as i32,
                click_count: 1,
            }
        } else {
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: point.x as i32,
                y: point.y as i32,
                click_count: 1,
            }
        };
        menu.handle(&Event::Mouse(mouse))
    }
    fn row_point(menu: &WebMenu, depth: usize, index: usize) -> Point {
        let popup = &menu.popups[depth];
        let rect = popup.render.item_rect(index).unwrap();
        Point::new(
            popup.origin.x + 10.0,
            popup.origin.y + rect.origin.y + rect.size.height / 2.0,
        )
    }
    #[test]
    fn no_menu_preserves_content_and_input() {
        let mut menu = WebMenu::default();
        assert_eq!(
            menu.content_size(Size::new(640.0, 480.0)),
            Size::new(640.0, 480.0)
        );
        assert_eq!(
            mouse(&mut menu, true, Point::new(10.0, 10.0)),
            (false, None)
        );
    }
    #[test]
    fn disabled_rows_never_activate_and_enabled_rows_dispatch_shared_id() {
        let mut menu = menu();
        menu.open_top(0);
        let disabled = row_point(&menu, 0, 0);
        mouse(&mut menu, true, disabled);
        assert_eq!(mouse(&mut menu, false, disabled), (true, None));
        let enabled = row_point(&menu, 0, 1);
        mouse(&mut menu, true, enabled);
        assert_eq!(
            mouse(&mut menu, false, enabled),
            (true, Some("open".into()))
        );
        assert!(menu.popups.is_empty());
        assert!(menu.take_dirty());
    }
    #[test]
    fn nested_rows_use_shared_geometry_and_dispatch_leaf() {
        let mut menu = menu();
        menu.open_top(0);
        menu.hover_row(0, 3);
        assert_eq!(menu.popups.len(), 2);
        let nested = row_point(&menu, 1, 0);
        mouse(&mut menu, true, nested);
        assert_eq!(
            mouse(&mut menu, false, nested),
            (true, Some("nested".into()))
        );
    }
    #[test]
    fn menubar_popup_clamps_to_canvas_and_escape_dismisses() {
        let mut menu = menu();
        menu.resize(Size::new(180.0, 480.0));
        menu.open_top(0);
        let popup = &menu.popups[0];
        assert!(popup.origin.x + popup.render.size().width <= 180.0);
        assert!(popup.origin.y + popup.render.size().height <= 480.0);
        assert!(
            menu.handle(&Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Escape,
                modifiers: KeyModifiers::empty(),
            }))
            .0
        );
        assert!(menu.popups.is_empty());
    }
    #[test]
    fn space_activation_does_not_insert_text_into_scene_after_dismissal() {
        let mut menu = menu();
        menu.open_top(0);
        menu.popups[0].render.set_hovered(Some(1));
        assert_eq!(
            menu.handle(&Event::Keyboard(KeyEvent::Pressed {
                keycode: KeyCode::Space,
                modifiers: KeyModifiers::empty()
            })),
            (true, Some("open".into()))
        );
        assert_eq!(
            menu.handle(&Event::Keyboard(KeyEvent::Char { c: ' ' })),
            (true, None)
        );
        assert!(
            menu.handle(&Event::Keyboard(KeyEvent::Released {
                keycode: KeyCode::Space,
                modifiers: KeyModifiers::empty()
            }))
            .0
        );
        assert!(!menu.handle(&Event::Keyboard(KeyEvent::Char { c: 'a' })).0);
    }
    #[test]
    fn browser_reserved_shortcut_is_not_intercepted() {
        let mut menu = menu();
        menu.open_top(0);
        assert!(
            !menu
                .handle(&Event::Keyboard(KeyEvent::Pressed {
                    keycode: KeyCode::Char('l'),
                    modifiers: KeyModifiers {
                        control: true,
                        ..KeyModifiers::empty()
                    }
                }))
                .0
        );
    }
    #[test]
    fn outside_press_dismisses_without_dispatch_or_click_through() {
        let mut menu = menu();
        menu.open_top(0);
        assert_eq!(
            mouse(&mut menu, true, Point::new(600.0, 400.0)),
            (true, None)
        );
        assert!(menu.popups.is_empty());
        assert!(menu.take_dirty());
        assert_eq!(
            mouse(&mut menu, false, Point::new(600.0, 400.0)),
            (true, None)
        );
    }
    #[test]
    fn right_click_does_not_substitute_application_menu_or_consume_scene_input() {
        let mut menu = menu();
        for (x, y) in [(639, 479), (10, 10)] {
            menu.open_top(0);
            assert_eq!(
                menu.handle(&Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Right,
                    x,
                    y,
                    click_count: 1,
                })),
                (false, None)
            );
            assert!(menu.popups.is_empty());
            assert_eq!(
                menu.handle(&Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Right,
                    x,
                    y,
                    click_count: 1,
                })),
                (false, None)
            );
        }
        assert!(menu.has_menu());
        menu.open_top(0);
        assert!(!menu.popups.is_empty(), "the visible menubar still works");
    }
    #[test]
    fn release_dispatches_action_exactly_once() {
        let mut menu = menu();
        menu.open_top(0);
        let point = row_point(&menu, 0, 1);
        mouse(&mut menu, true, point);
        assert_eq!(mouse(&mut menu, false, point).1, Some("open".into()));
        assert_eq!(mouse(&mut menu, false, point).1, None);
    }
    #[test]
    fn checked_model_change_refreshes_but_identical_model_does_not_redraw() {
        let mut menu = menu();
        let model = MenuBarModel::new(vec![MenuItemModel::new("toggle", "Toggle").checked(true)]);
        menu.set_model(&model);
        assert!(menu.model.items()[0].checked_value());
        assert!(menu.take_dirty());
        menu.set_model(&model);
        assert!(!menu.take_dirty());
        menu.set_model(&MenuBarModel::new(vec![
            MenuItemModel::new("toggle", "Toggle").checked(false),
        ]));
        assert!(!menu.model.items()[0].checked_value());
        assert!(menu.take_dirty());
    }
    #[test]
    fn menubar_insets_match_hit_bounds_and_reserved_content_height() {
        let mut menu = menu();
        assert_eq!(menu.height(), 28.0);
        assert_eq!(menu.content_size(Size::new(640.0, 480.0)).height, 452.0);
        let first = menu.bar_bounds[0];
        assert_eq!(first.origin, Point::new(BAR_INSET_X, BAR_INSET_Y));
        assert_eq!(first.size.height, 24.0);
        assert!(menu.hit(Point::new(1.0, 1.0)).is_none());
        assert!(matches!(
            menu.hit(Point::new(first.origin.x + 2.0, first.origin.y + 2.0)),
            Some(Hit::Bar(0))
        ));
        mouse(
            &mut menu,
            true,
            Point::new(first.origin.x + 2.0, first.origin.y + 2.0),
        );
        assert_eq!(menu.popups[0].origin.y, menu.height() + POPUP_GAP);
    }
    #[test]
    fn submenu_uses_shared_width_and_aligned_row_with_gap() {
        let mut menu = menu();
        menu.open_top(0);
        menu.hover_row(0, 3);
        let parent = &menu.popups[0];
        let child = &menu.popups[1];
        assert_eq!(
            child.origin.x,
            parent.origin.x + parent.render.size().width + POPUP_GAP
        );
        let parent_row = parent.render.item_rect(3).unwrap();
        let child_row = child.render.item_rect(0).unwrap();
        assert_eq!(
            child.origin.y + child_row.origin.y,
            parent.origin.y + parent_row.origin.y
        );
        assert_eq!(
            child.render.size().width,
            Menu::new(
                child
                    .entries
                    .iter()
                    .map(|entry| match entry {
                        MenuEntry::Item(item) => item.to_menu_content(),
                        MenuEntry::Separator => MenuItemContent::separator(),
                    })
                    .collect()
            )
            .preferred_width()
        );
    }
    #[test]
    fn menu_hover_handoff_exits_scene_once_then_rearms_on_scene_input() {
        let mut handoff = HoverHandoff::default();
        let scene = MouseEvent::Moved { x: 40, y: 80 };
        let menu = MouseEvent::Moved { x: 60, y: 10 };
        assert_eq!(handoff.route(&scene, false, 28), None);
        assert_eq!(
            handoff.route(&menu, true, 28),
            Some(MouseEvent::Exited { x: 60, y: -18 })
        );
        assert_eq!(handoff.route(&menu, true, 28), None);
        assert_eq!(handoff.route(&scene, false, 28), None);
        assert_eq!(
            handoff.route(&menu, true, 28),
            Some(MouseEvent::Exited { x: 60, y: -18 })
        );
    }
    #[test]
    fn menu_hover_handoff_does_not_duplicate_real_exit_or_consume_actions() {
        let mut handoff = HoverHandoff::default();
        handoff.route(&MouseEvent::Moved { x: 40, y: 80 }, false, 28);
        handoff.route(&MouseEvent::Exited { x: -1, y: -1 }, false, 28);
        assert_eq!(
            handoff.route(&MouseEvent::Moved { x: 20, y: 10 }, true, 28),
            None
        );
        let release = MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: 20,
            y: 10,
            click_count: 1,
        };
        assert_eq!(handoff.route(&release, true, 28), None);
    }
}
