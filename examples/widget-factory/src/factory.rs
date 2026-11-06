//! ScarletUI Widget Factory
//!
//! A standalone widget showcase application built with ScarletUI.
//! Displays all available widgets across multiple navigation pages
//! (Overview, Controls, Inputs, Display).

use scarlet_ui::NavigationLink;
use scarlet_ui::hstack;
use scarlet_ui::prelude::*;
use scarlet_ui::vstack;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

// Real scene content behind the stock controls. Changing the save counter
// changes this scene so stale/flattened backdrop caches are visible immediately.
#[derive(Clone)]
struct MaterialBackdrop {
    phase: usize,
}

struct MaterialBackdropRender {
    phase: usize,
    size: Size,
}

impl View for MaterialBackdrop {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::RenderElement::new(
            self.clone(),
            MaterialBackdropRender {
                phase: self.phase,
                size: Size::ZERO,
            },
        ))
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl scarlet_ui::ElementRenderObject for MaterialBackdropRender {
    fn layout(&mut self, constraints: scarlet_ui::LayoutConstraints) -> Size {
        self.size = constraints.constrain(Size::new(760.0, 140.0));
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn render(&mut self) {}
    fn hit_test(&self, _: Point) -> bool {
        false
    }
    fn paint_outsets(&self) -> scarlet_ui::EdgeInsets {
        scarlet_ui::EdgeInsets::all(12.0)
    }
    fn paint(&self, ctx: &mut scarlet_ui::renderer::PaintContext<'_>, origin: Point) -> bool {
        let panel = Rect::from_xywh(
            origin.x - 12.0,
            origin.y,
            self.size.width + 24.0,
            self.size.height,
        );
        ctx.push_rounded_clip(panel, 20.0);
        ctx.fill_rect(panel, Color::rgb(218, 225, 237));
        let colors = [
            Color::rgb(168, 198, 207),
            Color::rgb(213, 183, 181),
            Color::rgb(186, 185, 214),
        ];
        let cell = panel.size.width / 3.0;
        for index in 0..3 {
            let x = panel.origin.x + cell * index as f32;
            let rect = Rect::from_xywh(x, panel.origin.y, cell, panel.size.height);
            ctx.fill_rect(rect, colors[(index + self.phase) % colors.len()]);
            // Small actual scene detail makes the material's spatial blur
            // distinguishable from an ordinary translucent colored rectangle.
            for line in 0..7 {
                let y = panel.origin.y + 14.0 + line as f32 * 19.0;
                ctx.fill_rect(
                    Rect::from_xywh(x + 10.0, y, cell - 20.0, 1.0),
                    Color::WHITE.with_opacity(0.30),
                );
            }
        }
        ctx.pop_clip();
        true
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
    fn update(&mut self, view: &dyn View) -> scarlet_ui::element::UpdateResult {
        let Some(view) = view.as_any().downcast_ref::<MaterialBackdrop>() else {
            return scarlet_ui::element::UpdateResult::Replaced;
        };
        if self.phase == view.phase {
            return scarlet_ui::element::UpdateResult::NoChange;
        }
        self.phase = view.phase;
        scarlet_ui::element::UpdateResult::Updated
    }
}

fn split_layout_for_selection(selection: usize) -> (SplitAxis, SplitAxisPolicy) {
    match selection {
        1 => (SplitAxis::Vertical, SplitAxisPolicy::Fixed),
        2 => (SplitAxis::Horizontal, SplitAxisPolicy::AdaptiveStack),
        _ => (SplitAxis::Horizontal, SplitAxisPolicy::Fixed),
    }
}

fn split_minimum_extents_for_selection(selection: usize) -> (f32, f32) {
    match selection {
        1 | 2 => (40.0, 40.0),
        _ => (72.0, 120.0),
    }
}

fn tab_bar_placement_for_selection(selection: usize) -> TabBarPlacement {
    match selection {
        1 => TabBarPlacement::Bottom,
        2 => TabBarPlacement::Automatic,
        _ => TabBarPlacement::Top,
    }
}

/// A small drawing app using the shared GPU canvas and ordinary UI controls.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq)]
struct SketchStroke {
    points: Vec<Point>, // normalized paper coordinates, preserved through resize
    color: [f32; 4],
    width: f32,
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone)]
pub(crate) struct Specimen {
    strokes: State<Vec<SketchStroke>>,
    color: State<usize>,
    line_width: State<f32>,
    width: State<f32>,
    active: std::rc::Rc<std::cell::Cell<bool>>,
    touch_id: std::rc::Rc<std::cell::Cell<Option<u64>>>,
    revision: std::rc::Rc<std::cell::Cell<u64>>,
    canvas: SgfxCanvasHandle,
    mesh: scarlet_ui::SgfxMeshHandle,
    frame: State<Arc<SgfxCanvasFrame>>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl Specimen {
    const PAPER_HEIGHT: f32 = 184.0;
    const COLORS: [[f32; 4]; 4] = [
        [0.20, 0.24, 0.29, 1.0],
        [0.72, 0.24, 0.25, 1.0],
        [0.22, 0.43, 0.66, 1.0],
        [0.25, 0.51, 0.40, 1.0],
    ];

    pub(crate) fn new() -> Self {
        let ink = Self::COLORS[0];
        let ridge = SketchStroke {
            points: vec![
                Point::new(0.12, 0.76),
                Point::new(0.34, 0.29),
                Point::new(0.50, 0.57),
                Point::new(0.64, 0.38),
                Point::new(0.88, 0.76),
            ],
            color: ink,
            width: 2.5,
        };
        let horizon = SketchStroke {
            points: vec![Point::new(0.12, 0.80), Point::new(0.88, 0.80)],
            color: ink,
            width: 2.0,
        };
        let sun = SketchStroke {
            points: (0..=40)
                .map(|i| {
                    let t = i as f32 * std::f32::consts::TAU / 40.0;
                    Point::new(0.73 + t.cos() * 0.055, 0.27 + t.sin() * 0.11)
                })
                .collect(),
            color: Self::COLORS[1],
            width: 2.5,
        };
        let app = Self {
            strokes: State::new(StateId::new(120), vec![ridge, horizon, sun]),
            color: State::new(StateId::new(121), 0),
            line_width: State::new(StateId::new(122), 3.0),
            width: State::new(StateId::new(123), 400.0),
            active: Default::default(),
            touch_id: Default::default(),
            revision: Default::default(),
            canvas: SgfxCanvasHandle::new(),
            mesh: scarlet_ui::SgfxMeshHandle::new(),
            frame: State::new(
                StateId::new(124),
                Arc::new(SgfxCanvasFrame::new(0, Color::WHITE)),
            ),
        };
        app.refresh_frame();
        app
    }

    fn paper_width(&self) -> f32 {
        self.width.get() - 24.0
    }
    fn resize_specimen(&self, width: f32) {
        let width = width.max(300.0);
        if self.width.get() != width {
            self.active.set(false);
            self.touch_id.set(None);
            self.width.set(width);
            self.refresh_frame();
        }
    }
    fn point(&self, x: i32, y: i32) -> Point {
        Point::new(
            (x as f32 / self.paper_width()).clamp(0.0, 1.0),
            (y as f32 / Self::PAPER_HEIGHT).clamp(0.0, 1.0),
        )
    }
    fn begin_stroke(&self, x: i32, y: i32) {
        let mut strokes = self.strokes.get();
        if strokes.len() >= 128 || strokes.iter().map(|s| s.points.len()).sum::<usize>() >= 4096 {
            return;
        }
        strokes.push(SketchStroke {
            points: vec![self.point(x, y)],
            color: Self::COLORS[self.color.get()],
            width: self.line_width.get(),
        });
        self.active.set(true);
        self.strokes.set(strokes);
        self.refresh_frame();
    }
    fn extend_stroke(&self, x: i32, y: i32) {
        if !self.active.get() {
            return;
        }
        let point = self.point(x, y);
        let mut strokes = self.strokes.get();
        // Keep total mesh/upload memory bounded even during a long drawing session.
        if strokes.iter().map(|s| s.points.len()).sum::<usize>() >= 4096 {
            return;
        }
        let Some(stroke) = strokes.last_mut() else {
            return;
        };
        if stroke.points.len() >= 2048 {
            return;
        }
        if let Some(last) = stroke.points.last() {
            let dx = (point.x - last.x) * self.paper_width();
            let dy = (point.y - last.y) * Self::PAPER_HEIGHT;
            if dx * dx + dy * dy < 1.0 {
                return;
            }
        }
        stroke.points.push(point);
        self.strokes.set(strokes);
        self.refresh_frame();
    }
    fn clear(&self) {
        self.active.set(false);
        self.touch_id.set(None);
        self.strokes.set(Vec::new());
        self.refresh_frame();
    }
    fn undo(&self) {
        self.active.set(false);
        self.touch_id.set(None);
        let mut strokes = self.strokes.get();
        if strokes.pop().is_some() {
            self.strokes.set(strokes);
            self.refresh_frame();
        }
    }
    fn paper_event(&self, event: &Event) -> bool {
        use scarlet_ui::event::{GesturePhase, TouchGesture, TouchPhase};
        match event {
            Event::Touch(change) => {
                match change.phase {
                    TouchPhase::Down if self.touch_id.get().is_none() => {
                        self.active.set(false);
                        self.begin_stroke(change.x, change.y);
                        if self.active.get() {
                            self.touch_id.set(Some(change.id));
                        }
                    }
                    TouchPhase::Down => self.active.set(false),
                    TouchPhase::Move if self.touch_id.get() == Some(change.id) => {
                        self.extend_stroke(change.x, change.y);
                    }
                    TouchPhase::Up | TouchPhase::Cancel
                        if self.touch_id.get() == Some(change.id) =>
                    {
                        if change.phase == TouchPhase::Up {
                            self.extend_stroke(change.x, change.y);
                        }
                        self.active.set(false);
                        self.touch_id.set(None);
                    }
                    _ => return false,
                }
                true
            }
            // The arena cancels a pending press both when a drag wins and when
            // another contact intervenes. Only an awarded drag may resume it.
            Event::TouchGesture(TouchGesture::CancelPress { id })
                if self.touch_id.get() == Some(*id) =>
            {
                self.active.set(false);
                true
            }
            Event::TouchGesture(TouchGesture::Drag { id, phase, .. })
                if self.touch_id.get() == Some(*id) =>
            {
                match phase {
                    GesturePhase::Started => self.active.set(true),
                    GesturePhase::Cancelled | GesturePhase::Ended => self.active.set(false),
                    GesturePhase::Moved => {}
                }
                true
            }
            Event::TouchGesture(TouchGesture::Pinch { .. }) => self.active.replace(false),

            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                ..
            }) => {
                self.begin_stroke(*x, *y);
                true
            }
            Event::Mouse(MouseEvent::Moved { x, y }) if self.active.get() => {
                self.extend_stroke(*x, *y);
                true
            }
            Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                ..
            }) => {
                let active = self.active.get();
                self.extend_stroke(*x, *y);
                self.active.set(false);
                active
            }
            Event::Mouse(MouseEvent::ButtonCancelled { .. })
            | Event::Focus(scarlet_ui::event::FocusEvent::Lost) => {
                self.touch_id.set(None);
                self.active.replace(false)
            }
            _ => false,
        }
    }

    fn refresh_frame(&self) {
        let width = self.paper_width();
        let height = Self::PAPER_HEIGHT;
        let mut vertices = Vec::new();
        let mut triangle = |a: Point, b: Point, c: Point, color: [f32; 4]| {
            for p in [a, b, c] {
                vertices.push(SgfxCanvasVertex::new(
                    [2.0 * p.x / width - 1.0, 1.0 - 2.0 * p.y / height, 0.0, 1.0],
                    color,
                ));
            }
        };
        for stroke in self.strokes.get() {
            let points: Vec<Point> = stroke
                .points
                .iter()
                .map(|p| Point::new(p.x * width, p.y * height))
                .collect();
            let radius = stroke.width * 0.5;
            for pair in points.windows(2) {
                let a = pair[0];
                let b = pair[1];
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let length = (dx * dx + dy * dy).sqrt();
                if length < 0.001 {
                    continue;
                }
                let nx = -dy / length * radius;
                let ny = dx / length * radius;
                let corners = [
                    Point::new(a.x + nx, a.y + ny),
                    Point::new(a.x - nx, a.y - ny),
                    Point::new(b.x - nx, b.y - ny),
                    Point::new(b.x + nx, b.y + ny),
                ];
                triangle(corners[0], corners[1], corners[2], stroke.color);
                triangle(corners[0], corners[2], corners[3], stroke.color);
            }
            // Round joins and end caps, including a visible dot for a tap.
            for p in points {
                for i in 0..12 {
                    let a = i as f32 * std::f32::consts::TAU / 12.0;
                    let b = (i + 1) as f32 * std::f32::consts::TAU / 12.0;
                    triangle(
                        p,
                        Point::new(p.x + a.cos() * radius, p.y + a.sin() * radius),
                        Point::new(p.x + b.cos() * radius, p.y + b.sin() * radius),
                        stroke.color,
                    );
                }
            }
        }
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        let mut frame = SgfxCanvasFrame::new(revision, Color::rgb(252u8, 251u8, 248u8))
            .reference_aspect(width / height);
        if !vertices.is_empty() {
            frame = frame.draw(SgfxCanvasDraw::new(
                SgfxMesh::with_handle(self.mesh, revision, vertices),
                [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ],
            ));
        }
        self.frame.set(Arc::new(frame));
    }

    fn swatch(&self, index: usize) -> impl View + Clone + use<> {
        let selected = self.color.clone();
        let rgba = Self::COLORS[index];
        let color = Color::rgba_f32(rgba[0], rgba[1], rgba[2], rgba[3]);
        let button = Button::new("")
            .background_color(color)
            .on_click(move || selected.set(index));
        let button = if self.color.get() == index {
            button.icon(Icon::Check).icon_color(Color::WHITE)
        } else {
            button
        };
        button.frame(26.0, 26.0)
    }
    fn body(&self) -> impl View + Clone + use<> {
        let undo = self.clone();
        let clear = self.clone();
        let paper = self.clone();
        vstack![
            hstack![
                Text::new("Sketch").font_size(18.0),
                Spacer::new(),
                Button::new("Undo").on_click(move || undo.undo()),
                Button::new("Clear").on_click(move || clear.clear())
            ]
            .spacing(8.0),
            SgfxCanvas::from_state(
                self.canvas,
                self.paper_width(),
                Self::PAPER_HEIGHT,
                self.frame.clone()
            )
            .on_event(move |e| paper.paper_event(e))
            .border(Color::rgb(224u8, 223u8, 219u8), 1.0),
            hstack![
                self.swatch(0),
                self.swatch(1),
                self.swatch(2),
                self.swatch(3),
                Spacer::new(),
                Slider::new(self.line_width.clone())
                    .min(1.0)
                    .max(8.0)
                    .frame((self.paper_width() - 144.0).max(100.0), 26.0)
            ]
            .spacing(6.0),
        ]
        .spacing(10.0)
        .padding(12.0)
    }
}

#[cfg(any(target_arch = "wasm32", test))]
impl View for Specimen {
    fn create_element(&self) -> Box<dyn Element> {
        self.body().create_element()
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.color, &self.line_width, &self.width]
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
#[cfg(any(target_arch = "wasm32", test))]
impl Application for Specimen {
    fn on_window_created(
        &mut self,
        _: &scarlet_ui::WindowContext,
        window: &mut dyn scarlet_ui::platform::PlatformWindow,
    ) {
        self.resize_specimen(window.size().width);
    }
    fn on_window_resize(&mut self, _: &scarlet_ui::WindowContext, width: u32, _: u32) {
        self.resize_specimen(width as f32);
    }
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "specimen",
            Window::new("Sketch", self.body()).size(Size::new(self.width.get(), 300.0)),
        )
    }
}

// ScrollView supplies a finite viewport to its child. Factory pages instead
// measure their rows at that width and their natural height, so stacked examples
// remain reachable without hand-maintained content-height constants.
#[derive(Clone)]
struct FactoryPageContent<V: View + Clone>(V);

impl<V: View + Clone> View for FactoryPageContent<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(
            scarlet_ui::element::RenderElement::with_view_children_and_updater(
                self.clone(),
                |_| FactoryPageLayout(Size::ZERO),
                |_, _| scarlet_ui::element::UpdateResult::Updated,
                |view| vec![view.0.clone_view()],
            ),
        )
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.0.listenables()
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

struct FactoryPageLayout(Size);
impl scarlet_ui::element::ElementRenderObject for FactoryPageLayout {
    fn layout(&mut self, constraints: scarlet_ui::element::LayoutConstraints) -> Size {
        self.0 = Size::new(constraints.max_width, 0.0);
        self.0
    }
    fn layout_with_children(
        &mut self,
        constraints: scarlet_ui::element::LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.0 = children
            .first_mut()
            .map(|child| {
                child.set_position(Point::ZERO);
                child.layout(scarlet_ui::element::LayoutConstraints::new(
                    constraints.max_width,
                    constraints.max_width,
                    0.0,
                    f32::INFINITY,
                ))
            })
            .unwrap_or(Size::ZERO);
        self.0
    }
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn size(&self) -> Size {
        self.0
    }
    fn render(&mut self) {}
    fn paint(&self, ctx: &mut scarlet_ui::renderer::PaintContext<'_>, origin: Point) -> bool {
        // The page owns the same opaque surface as its Window, so its static
        // glass and backdrop can be retained together as one scroll layer.
        ctx.fill_rect(
            Rect::new(origin, self.0),
            ColorPalette::light().window_background(),
        );
        true
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

#[derive(Clone)]
pub(crate) struct WidgetFactory {
    viewport: State<Size>,
    slider_value: State<f32>,
    toggle_on: State<bool>,
    toggle_off: State<bool>,
    text_value: State<String>,
    study_count: State<u32>,
    glass_quality: State<usize>,
    study_disabled_text: State<String>,
    study_disabled_selected: State<usize>,
    selected: State<usize>,
    split_layout: State<usize>,
    tab_bar_placement: State<usize>,
    text_document: State<TextDocument>,
    text_selection: State<TextSelection>,
    cube_canvas: SgfxCanvasHandle,
    cube_frame: State<Arc<SgfxCanvasFrame>>,
    cube_mesh: Arc<SgfxMesh>,
    cube_started_at: Instant,
    cube_revision: u64,
}

impl WidgetFactory {
    pub(crate) fn new() -> Self {
        let cube_mesh = cube_mesh();
        Self {
            viewport: State::new(StateId::new(29), Size::new(860.0, 560.0)),
            slider_value: State::new(StateId::new(20), 0.42),
            toggle_on: State::new(StateId::new(21), true),
            toggle_off: State::new(StateId::new(33), false),
            text_value: State::new(StateId::new(22), String::from("Factory text field")),
            selected: State::new(StateId::new(23), 1usize),
            study_count: State::new(StateId::new(30), 0),
            glass_quality: State::new(StateId::new(34), 2),
            study_disabled_text: State::new(StateId::new(31), String::from("Project name")),
            study_disabled_selected: State::new(StateId::new(32), 0),
            split_layout: State::new(StateId::new(27), 0usize),
            tab_bar_placement: State::new(StateId::new(28), 0usize),
            text_document: State::new(
                StateId::new(24),
                TextDocument::from_str(
                    "# Hello TextView\n\nType here...\n\n- Item 1\n- Item 2\n- Item 3\n",
                ),
            ),
            text_selection: State::new(StateId::new(25), TextSelection::collapsed(0)),
            cube_canvas: SgfxCanvasHandle::new(),
            cube_frame: State::new(StateId::new(26), cube_frame(0, 0.0, Arc::clone(&cube_mesh))),
            cube_mesh,
            cube_started_at: Instant::now(),
            cube_revision: 0,
        }
    }

    // One model for every platform; Web chooses an in-canvas presentation.
    fn menu_model(&self) -> MenuBarModel {
        let reset_slider = self.slider_value.clone();
        let reset_toggle = self.toggle_on.clone();
        let toggle = self.toggle_on.clone();
        let low = self.slider_value.clone();
        let high = self.slider_value.clone();
        let text = self.text_value.clone();
        MenuBarModel::new(vec![
            MenuItemModel::new("factory", "Factory").children(vec![
                MenuEntry::Item(
                    MenuItemModel::new("reset", "Reset controls")
                        .enabled(self.slider_value.get() != 0.42 || !self.toggle_on.get())
                        .on_activate(Arc::new(move || {
                            reset_slider.set(0.42);
                            reset_toggle.set(true);
                        })),
                ),
                MenuEntry::Separator,
                MenuEntry::Item(
                    MenuItemModel::new("unavailable", "Unavailable action").enabled(false),
                ),
            ]),
            MenuItemModel::new("view", "View").children(vec![MenuEntry::Item(
                MenuItemModel::new("toggle", "Toggle enabled")
                    .checked(self.toggle_on.get())
                    .on_activate(Arc::new(move || toggle.set(!toggle.get()))),
            )]),
            MenuItemModel::new("tools", "Tools").children(vec![
                MenuEntry::Item(MenuItemModel::new("slider", "Slider preset").children(vec![
                    MenuEntry::Item(MenuItemModel::new("quarter", "25 percent")
                        .on_activate(Arc::new(move || low.set(0.25)))),
                    MenuEntry::Item(MenuItemModel::new("three_quarters", "75 percent")
                        .on_activate(Arc::new(move || high.set(0.75)))),
                ])),
                // Kept visible/clickable, but Web must not steal the browser's
                // address-bar shortcut merely because an app lists it here.
                MenuEntry::Item(
                    MenuItemModel::new("sample_text", "Set sample text")
                        .shortcut("Ctrl+L")
                        .on_activate(Arc::new(move || {
                            text.set(String::from("Menu action worked"))
                        })),
                ),
            ]),
        ])
    }

    const SIDEBAR_WIDTH: f32 = 190.0;
    const MIN_CONTENT_WIDTH: f32 = 360.0;

    fn content_width(&self) -> f32 {
        let width = self.viewport.get().width;
        if width >= Self::SIDEBAR_WIDTH + Self::MIN_CONTENT_WIDTH {
            width - Self::SIDEBAR_WIDTH
        } else {
            width
        }
    }
    fn stacked_rows(&self) -> bool {
        self.content_width() < 560.0
    }
    fn page_padding(&self) -> f32 {
        if self.stacked_rows() { 16.0 } else { 24.0 }
    }
    fn row_width(&self) -> f32 {
        (self.content_width() - self.page_padding() * 2.0).max(0.0)
    }
    fn control_width(&self) -> f32 {
        (self.row_width() - if self.stacked_rows() { 0.0 } else { 168.0 }).max(0.0)
    }
    fn demo_width(&self, preferred: f32) -> f32 {
        self.control_width().min(preferred)
    }

    fn scroll_page<V: View + Clone>(&self, content: V) -> impl View + Clone + use<V> {
        ScrollView::new(FactoryPageContent(content)).vertical()
    }

    fn row<V: View + Clone>(&self, name: &str, control: V) -> impl View + Clone + use<V> {
        let label = Text::new(name.to_owned()).text_style(TextStyle::Callout);
        if self.stacked_rows() {
            Either::A(
                vstack![label, control]
                    .spacing(8.0)
                    .alignment(Alignment::Leading)
                    .frame_width(self.row_width()),
            )
        } else {
            Either::B(
                hstack![label.frame_width(150.0), control]
                    .spacing(18.0)
                    .alignment(Alignment::Top)
                    .frame_width(self.row_width()),
            )
        }
    }

    fn button(&self) -> impl View + Clone + use<> {
        Button::new("Factory Button")
    }

    fn text_field(&self) -> impl View + Clone + use<> {
        TextField::new(self.text_value.clone())
            .placeholder("Enter value")
            .frame_width(self.demo_width(320.0))
    }

    fn slider(&self) -> impl View + Clone + use<> {
        Slider::new(self.slider_value.clone())
            .min(0.0)
            .max(1.0)
            .frame_width(self.demo_width(320.0))
    }

    fn toggle(&self) -> impl View + Clone + use<> {
        hstack![
            hstack![
                Toggle::new(self.toggle_off.clone()),
                Text::new(if self.toggle_off.get() { "On" } else { "Off" })
                    .text_style(TextStyle::Callout)
                    .tone(TextTone::Secondary),
            ]
            .spacing(8.0)
            .alignment(Alignment::Center),
            hstack![
                Toggle::new(self.toggle_on.clone()),
                Text::new(if self.toggle_on.get() { "On" } else { "Off" })
                    .text_style(TextStyle::Callout)
                    .tone(TextTone::Secondary),
            ]
            .spacing(8.0)
            .alignment(Alignment::Center),
        ]
        .spacing(28.0)
        .alignment(Alignment::Center)
    }

    fn progress(&self) -> impl View + Clone + use<> {
        ProgressView::new(0.68).frame_width(self.demo_width(320.0))
    }

    fn select(&self) -> impl View + Clone + use<> {
        Select::new(
            vec![
                String::from("Compact"),
                String::from("Regular"),
                String::from("Expanded"),
            ],
            self.selected.clone(),
        )
        .width(self.demo_width(260.0))
    }

    fn rectangle(&self) -> impl View + Clone + use<> {
        Rectangle::new()
            .fill(Color::rgb(235u8, 242u8, 255u8))
            .corner_radius(8.0)
            .border(1.0, Color::rgb(105u8, 135u8, 210u8))
            .frame(self.demo_width(220.0), 28.0)
    }

    fn divider(&self) -> impl View + Clone + use<> {
        Divider::new().frame(self.demo_width(220.0), 1.0)
    }

    fn sgfx_cube(&self) -> impl View + Clone + use<> {
        SgfxCanvas::from_state(
            self.cube_canvas,
            self.row_width().min(320.0),
            220.0,
            self.cube_frame.clone(),
        )
    }

    fn text_view(&self) -> impl View + Clone + use<> {
        TextView::with_document(self.text_document.clone(), self.text_selection.clone())
            .placeholder("Type something...")
            .font_size(14.0)
            .padding(8.0)
            .frame(self.demo_width(320.0), 200.0)
    }

    fn scroll_view(&self) -> impl View + Clone + use<> {
        ScrollView::new(
            vstack! {
                Text::new("ScrollView content").font_size(16.0),
                hstack![Text::new("Track 01").frame_width(150.0), ProgressView::new(0.20).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 02").frame_width(150.0), ProgressView::new(0.42).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 03").frame_width(150.0), ProgressView::new(0.64).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 04").frame_width(150.0), ProgressView::new(0.82).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 05").frame_width(150.0), ProgressView::new(0.34).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 06").frame_width(150.0), ProgressView::new(0.56).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 07").frame_width(150.0), ProgressView::new(0.74).frame(260.0, 18.0)].spacing(18.0),
                hstack![Text::new("Track 08").frame_width(150.0), ProgressView::new(0.12).frame(260.0, 18.0)].spacing(18.0),
            }
            .spacing(8.0)
            .alignment(Alignment::Leading)
            .padding(12.0),
        )
        .both_axes()
        .content_size(520.0, 360.0)
        .scrollbar_visibility(ScrollbarVisibility::Always)
        .frame(self.demo_width(320.0), 160.0)
    }

    fn horizontal_scroll_view(&self) -> impl View + Clone + use<> {
        ScrollView::new(
            hstack! {
                Text::new("Clip 01").frame(120.0, 40.0),
                Text::new("Clip 02").frame(120.0, 40.0),
                Text::new("Clip 03").frame(120.0, 40.0),
                Text::new("Clip 04").frame(120.0, 40.0),
            }
            .spacing(8.0)
            .padding(8.0),
        )
        .horizontal()
        .content_size(560.0, 64.0)
        .frame(self.demo_width(240.0), 64.0)
    }

    fn vertical_scroll_view(&self) -> impl View + Clone + use<> {
        ScrollView::new(
            vstack! {
                Text::new("Row 01"),
                Text::new("Row 02"),
                Text::new("Row 03"),
                Text::new("Row 04"),
                Text::new("Row 05"),
                Text::new("Row 06"),
                Text::new("Row 07"),
                Text::new("Row 08"),
            }
            .spacing(8.0)
            .padding(8.0),
        )
        .vertical()
        .content_size(160.0, 240.0)
        .frame(self.demo_width(160.0), 96.0)
    }

    fn split_view(&self) -> impl View + Clone + use<> {
        let selection = self.split_layout.get();
        let (axis, axis_policy) = split_layout_for_selection(selection);
        let (min_first, min_second) = split_minimum_extents_for_selection(selection);
        SplitView::new(
            Text::new("Track List")
                .font_size(13.0)
                .padding(10.0)
                .background(Color::rgb(240u8, 243u8, 248u8)),
            Text::new("Arrange")
                .font_size(13.0)
                .padding(10.0)
                .background(Color::rgb(252u8, 252u8, 253u8)),
        )
        .fraction(0.34)
        .min_first(min_first)
        .min_second(min_second)
        .axis(axis)
        .axis_policy(axis_policy)
        .adaptive_stack_narrow_width(420.0)
        .frame(self.demo_width(320.0), 160.0)
    }

    fn split_layout_select(&self) -> impl View + Clone + use<> {
        Select::new(
            vec![
                String::from("Horizontal"),
                String::from("Vertical"),
                String::from("Automatic"),
            ],
            self.split_layout.clone(),
        )
        .width(self.demo_width(220.0))
    }

    fn tab_view(&self) -> impl View + Clone + use<> {
        TabView::new(vec![
            TabItem::new("Mixer", || {
                hstack! {
                    Text::new("Ch 1").alignment(Alignment::Center).frame(72.0, 32.0),
                    Text::new("Ch 2").alignment(Alignment::Center).frame(72.0, 32.0),
                    Text::new("Master").alignment(Alignment::Center).frame(72.0, 32.0),
                }
                .spacing(8.0)
                .padding(12.0)
                .alignment(Alignment::Top)
            }),
            TabItem::new("Editor", || {
                vstack! {
                    Text::new("Region Inspector").font_size(13.0),
                    ProgressView::new(0.52).frame(220.0, 18.0),
                }
                .spacing(10.0)
                .padding(12.0)
                .alignment(Alignment::Top)
            }),
        ])
        .tab_bar_placement(tab_bar_placement_for_selection(
            self.tab_bar_placement.get(),
        ))
        .frame(self.demo_width(320.0), 112.0)
    }

    fn tab_bar_placement_select(&self) -> impl View + Clone + use<> {
        Select::new(
            vec![
                String::from("Top"),
                String::from("Bottom"),
                String::from("Automatic"),
            ],
            self.tab_bar_placement.clone(),
        )
        .width(self.demo_width(220.0))
    }

    fn configurable_layouts(&self) -> impl View + Clone + use<> {
        // The ordinary layout gallery should show TabView itself. The colored
        // material stress scene belongs to Controls, not behind tab content.
        vstack! {
            self.row("Split layout", self.split_layout_select()),
            self.row("SplitView", self.split_view()),
            self.row("Tab bar", self.tab_bar_placement_select()),
            self.row("TabView", self.tab_view()),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
    }

    fn overview_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Widget Factory").text_style(TextStyle::Title),
            Text::new("PaintCommand default rendering").text_style(TextStyle::Callout).tone(TextTone::Secondary),
            self.row("ProgressView", self.progress()),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
            self.row("ScrollView both", self.scroll_view()),
            self.configurable_layouts(),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
        .padding(self.page_padding());
        self.scroll_page(content)
    }

    fn sgfx_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("SGFX").text_style(TextStyle::Title),
            Text::new("SGFX retained canvas").text_style(TextStyle::Callout).tone(TextTone::Secondary),
            self.sgfx_cube(),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
        .padding(self.page_padding());
        self.scroll_page(content)
    }

    // Trial surface: real stock controls, not a separate skin or HTML specimen.
    fn control_study(&self) -> impl View + Clone + use<> {
        let width = self.row_width().min(760.0);
        let count = self.study_count.clone();
        let button = vstack![
            Text::new("Action").text_style(TextStyle::Callout),
            Button::new("Save changes")
                .on_click(move || count.update(|value| *value += 1))
                .frame_width(130.0),
        ]
        .spacing(8.0)
        .alignment(Alignment::Leading);
        let field = vstack![
            Text::new("Project name").text_style(TextStyle::Callout),
            TextField::new(self.text_value.clone())
                .placeholder("Untitled project")
                .frame_width(if width < 550.0 {
                    width
                } else {
                    (width - 170.0) * 0.6
                }),
        ]
        .spacing(8.0)
        .alignment(Alignment::Leading);
        let select = vstack![
            Text::new("Density").text_style(TextStyle::Callout),
            Select::new(
                vec![
                    String::from("Compact"),
                    String::from("Regular"),
                    String::from("Expanded")
                ],
                self.selected.clone()
            )
            .style(SelectStyle::Menu)
            .width(if width < 550.0 {
                width
            } else {
                (width - 170.0) * 0.4
            }),
        ]
        .spacing(8.0)
        .alignment(Alignment::Leading);
        let controls = if width < 550.0 {
            Either::A(
                vstack![button, field, select]
                    .spacing(20.0)
                    .alignment(Alignment::Leading),
            )
        } else {
            Either::B(
                hstack![button.frame_width(130.0), field, select]
                    .spacing(20.0)
                    .alignment(Alignment::Top),
            )
        };
        vstack![
            Text::new("Control study").text_style(TextStyle::Title),
            Text::new("Buttons, inputs and menus with a shared material.")
                .text_style(TextStyle::Callout)
                .tone(TextTone::Secondary),
            hstack![
                Text::new("Glass quality").text_style(TextStyle::Callout),
                Select::new(
                    vec![
                        String::from("Tint only"),
                        String::from("Frosted"),
                        String::from("Refractive")
                    ],
                    self.glass_quality.clone()
                )
                .width(160.0)
                .on_change(|index| set_glass_quality(match index {
                    0 => GlassQuality::TintOnly,
                    1 => GlassQuality::Frosted,
                    _ => GlassQuality::Refractive,
                })),
            ]
            .spacing(12.0),
            ZStack::new((
                MaterialBackdrop {
                    phase: (self.study_count.get() % 3) as usize
                }
                .frame(width, if width < 550.0 { 300.0 } else { 140.0 }),
                controls,
            )),
            Text::new(format!(
                "Saved {} times. Save again to change the real backdrop behind the glass.",
                self.study_count.get()
            ))
            .text_style(TextStyle::Callout)
            .tone(TextTone::Secondary),
            Divider::new().frame(width, 1.0),
            self.row(
                "Disabled",
                vstack![
                    Button::new("Save changes").enabled(false),
                    TextField::new(self.study_disabled_text.clone())
                        .enabled(false)
                        .frame_width(self.demo_width(280.0)),
                    Select::new(
                        vec![String::from("Unavailable")],
                        self.study_disabled_selected.clone()
                    )
                    .enabled(false)
                    .style(SelectStyle::Menu)
                    .width(self.demo_width(280.0)),
                ]
                .spacing(12.0)
                .alignment(Alignment::Leading)
            ),
        ]
        .spacing(24.0)
        .alignment(Alignment::Leading)
    }

    fn controls_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            self.control_study(),
            Divider::new().frame(self.row_width(), 1.0),
            Text::new("Other controls").text_style(TextStyle::Headline),
            self.row("Button", self.button()),
            self.row("Toggle", self.toggle()),
            self.row("Slider", self.slider()),
            self.row("ProgressView", self.progress()),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
        .padding(self.page_padding());
        self.scroll_page(content)
    }

    fn inputs_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Inputs").text_style(TextStyle::Title),
            self.row("TextField", self.text_field()),
            self.row("Select", self.select()),
            self.row("Slider", self.slider()),
            self.row("TextView", self.text_view()),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
        .padding(self.page_padding());
        self.scroll_page(content)
    }

    fn icons(&self) -> impl View + Clone + use<> {
        hstack! {
            IconView::new(Icon::Folder)
                .size(IconSize::Large)
                .weight(IconWeight::Thin),
            IconView::new(Icon::Folder)
                .size(IconSize::Large)
                .weight(IconWeight::Normal),
            IconView::new(Icon::Folder)
                .size(IconSize::Large)
                .weight(IconWeight::Bold),
            IconView::new(Icon::Folder)
                .size(IconSize::Large)
                .filled(),
            IconView::new(Icon::Settings)
                .size(IconSize::Large)
                .filled()
                .color(Color::rgb(52u8, 120u8, 246u8)),
            IconView::new(Icon::Heart)
                .size(IconSize::Large)
                .filled()
                .color(Color::rgb(220u8, 55u8, 85u8)),
        }
        .spacing(14.0)
    }

    fn display_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Display").text_style(TextStyle::Title),
            self.row(
                "Text",
                vstack![
                    Text::new("Text hierarchy").text_style(TextStyle::Headline),
                    Text::new("Readable content, clear emphasis").text_style(TextStyle::Body),
                    Text::new("Supporting detail shares the same type family.")
                        .text_style(TextStyle::Callout)
                        .tone(TextTone::Secondary),
                ]
                .spacing(8.0)
                .alignment(Alignment::Leading),
            ),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
            self.row("Tabler Icons", self.icons()),
            self.row("ScrollView both", self.scroll_view()),
            self.row("ScrollView x", self.horizontal_scroll_view()),
            self.row("ScrollView y", self.vertical_scroll_view()),
            self.configurable_layouts(),
        }
        .spacing(16.0)
        .alignment(Alignment::Leading)
        .padding(self.page_padding());
        self.scroll_page(content)
    }
}

impl View for WidgetFactory {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new(self.clone()))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![
            &self.viewport,
            &self.slider_value,
            &self.toggle_on,
            &self.toggle_off,
            &self.text_value,
            &self.study_count,
            &self.glass_quality,
            &self.study_disabled_text,
            &self.study_disabled_selected,
            &self.selected,
            &self.split_layout,
            &self.tab_bar_placement,
            &self.text_document,
            &self.text_selection,
        ]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Application for WidgetFactory {
    fn on_window_created(
        &mut self,
        _: &scarlet_ui::WindowContext,
        window: &mut dyn scarlet_ui::platform::PlatformWindow,
    ) {
        let size = window.size();
        let safe = window.safe_area_insets();
        self.viewport.set(Size::new(
            (size.width - safe.left - safe.right).max(0.0),
            (size.height - safe.top - safe.bottom).max(0.0),
        ));
    }

    fn scenes(&self) -> impl Scene {
        let overview = self.clone();
        let sgfx = self.clone();
        let controls = self.clone();
        let inputs = self.clone();
        let display = self.clone();
        let viewport = self.viewport.clone();

        WindowGroup::new(
            "main",
            Window::new(
                "Widget Factory",
                scarlet_ui::navigation! {
                    NavigationLink::new("Overview", move || overview.overview_page())
                        .icon(Icon::Home),
                    NavigationLink::new("Controls", move || controls.controls_page())
                        .icon(Icon::Adjustments),
                    NavigationLink::new("Inputs", move || inputs.inputs_page())
                        .icon(Icon::FileText),
                    NavigationLink::new("Display", move || display.display_page())
                        .icon(Icon::Photo),
                    NavigationLink::new("SGFX", move || sgfx.sgfx_page())
                        .icon(Icon::Package),
                }
                .presentation(NavigationPresentation::Automatic)
                .floating_bar_bottom_overlap(if cfg!(target_arch = "wasm32") {
                    12.0
                } else {
                    0.0
                })
                .shows_icons(true)
                .sidebar_width(Self::SIDEBAR_WIDTH)
                .minimum_content_size(Size::new(Self::MIN_CONTENT_WIDTH, 0.0))
                .on_geometry_change(
                    |geometry| {
                        let size = geometry.size();
                        let safe = geometry.safe_area_insets();
                        Size::new(
                            (size.width - safe.left - safe.right).max(0.0),
                            (size.height - safe.top - safe.bottom).max(0.0),
                        )
                    },
                    move |size| viewport.set(size),
                ),
            )
            .size(Size::new(860.0, 560.0))
            .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
            .menu_bar(self.menu_model()),
        )
    }

    fn on_idle(&mut self) {
        self.cube_revision = self.cube_revision.wrapping_add(1);
        let angle = self.cube_started_at.elapsed().as_secs_f32();
        self.cube_frame.set(cube_frame(
            self.cube_revision,
            angle,
            Arc::clone(&self.cube_mesh),
        ));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_native() -> scarlet_ui::Result<()> {
    let mut app = WidgetFactory::new();
    app.run()
}

fn cube_frame(revision: u64, angle: f32, mesh: Arc<SgfxMesh>) -> Arc<SgfxCanvasFrame> {
    let aspect = 320.0 / 220.0;
    let projection = perspective_matrix(core::f32::consts::FRAC_PI_4, aspect, 0.1, 20.0);
    let model = mat4_mul(
        translation_matrix(0.0, 0.0, -5.0),
        mat4_mul(
            rotation_y_matrix(angle * 0.9 + 0.72),
            rotation_x_matrix(angle * 0.55 - 0.48),
        ),
    );
    let transform = mat4_mul(projection, model);
    Arc::new(
        SgfxCanvasFrame::new(revision, Color::rgb(10u8, 16u8, 30u8))
            .depth_tested()
            .reference_aspect(aspect)
            .draw(SgfxCanvasDraw::new(mesh, transform)),
    )
}

fn cube_mesh() -> Arc<SgfxMesh> {
    let mut vertices = Vec::with_capacity(36);
    push_cube_face(
        &mut vertices,
        [
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
        [0.98, 0.28, 0.34, 1.0],
    );
    push_cube_face(
        &mut vertices,
        [
            [-1.0, -1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, -1.0, -1.0],
        ],
        [0.28, 0.52, 0.98, 1.0],
    );
    push_cube_face(
        &mut vertices,
        [
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
        ],
        [0.32, 0.86, 0.62, 1.0],
    );
    push_cube_face(
        &mut vertices,
        [
            [-1.0, -1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [-1.0, 1.0, -1.0],
        ],
        [0.98, 0.70, 0.25, 1.0],
    );
    push_cube_face(
        &mut vertices,
        [
            [-1.0, 1.0, -1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, -1.0],
        ],
        [0.72, 0.38, 0.96, 1.0],
    );
    push_cube_face(
        &mut vertices,
        [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
        ],
        [0.24, 0.78, 0.92, 1.0],
    );
    SgfxMesh::new(vertices)
}

fn push_cube_face(vertices: &mut Vec<SgfxCanvasVertex>, corners: [[f32; 3]; 4], color: [f32; 4]) {
    for index in [0usize, 1, 2, 0, 2, 3] {
        let [x, y, z] = corners[index];
        vertices.push(SgfxCanvasVertex::new([x, y, z, 1.0], color));
    }
}

fn mat4_mul(lhs: [f32; 16], rhs: [f32; 16]) -> [f32; 16] {
    let mut result = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut value = 0.0;
            for inner in 0..4 {
                value += lhs[inner * 4 + row] * rhs[column * 4 + inner];
            }
            result[column * 4 + row] = value;
        }
    }
    result
}

fn translation_matrix(x: f32, y: f32, z: f32) -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
    ]
}

fn rotation_x_matrix(angle: f32) -> [f32; 16] {
    let (sin, cos) = angle.sin_cos();
    [
        1.0, 0.0, 0.0, 0.0, 0.0, cos, sin, 0.0, 0.0, -sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn rotation_y_matrix(angle: f32) -> [f32; 16] {
    let (sin, cos) = angle.sin_cos();
    [
        cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn perspective_matrix(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let focal = 1.0 / (fov_y * 0.5).tan();
    [
        focal / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        focal,
        0.0,
        0.0,
        0.0,
        0.0,
        far / (near - far),
        -1.0,
        0.0,
        0.0,
        (near * far) / (near - far),
        0.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui::event::Phase;
    use scarlet_ui::views::SplitViewRenderObject;

    #[test]
    fn layout_gallery_keeps_glass_tabs_without_the_diagnostic_backdrop() {
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        fn inspect(element: &dyn Element, tabs: &mut usize, proofs: &mut usize) {
            if let Some(render) = element.render_object() {
                if render.as_any().is::<MaterialBackdropRender>() {
                    *proofs += 1;
                }
                if render
                    .as_any()
                    .is::<scarlet_ui::views::TabViewRenderObject>()
                {
                    *tabs += 1;
                    let mut ctx = PaintContext::new();
                    render.paint(&mut ctx, Point::ZERO);
                    assert_eq!(
                        ctx.commands()
                            .iter()
                            .filter(|command| matches!(
                                command,
                                PaintCommand::BackdropFilterRoundedRect { .. }
                            ))
                            .count(),
                        1,
                        "one glass bar with a flat selected face"
                    );
                }
            }
            for child in element.children() {
                inspect(child.as_ref(), tabs, proofs);
            }
        }
        let app = WidgetFactory::new();
        let mut root = app.configurable_layouts().create_element();
        root.layout(LayoutConstraints::tight(800.0, 600.0));
        let (mut tabs, mut proofs) = (0, 0);
        inspect(root.as_ref(), &mut tabs, &mut proofs);
        assert_eq!(tabs, 1);
        assert_eq!(
            proofs, 0,
            "diagnostic color card must not look like part of TabView"
        );

        let mut controls = app.controls_page().create_element();
        controls.layout(LayoutConstraints::tight(800.0, 600.0));
        inspect(controls.as_ref(), &mut tabs, &mut proofs);
        assert!(
            proofs > 0,
            "the explicit material stress study remains available"
        );
    }

    #[test]
    fn mixer_content_has_equal_columns_and_is_centered_inside_tab_view() {
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        fn labels(element: &dyn Element, origin: Point, out: &mut Vec<(String, Rect)>) {
            let origin = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            if let Some(render) = element.render_object()
                && render.as_any().is::<scarlet_ui::views::TextRenderObject>()
            {
                let mut paint = PaintContext::new();
                render.paint(&mut paint, origin);
                for command in paint.commands() {
                    if let PaintCommand::DrawText { text, .. } = command {
                        out.push((
                            text.clone(),
                            Rect::from_xywh(
                                origin.x,
                                origin.y,
                                render.size().width,
                                render.size().height,
                            ),
                        ));
                    }
                }
            }
            for child in element.children() {
                labels(child.as_ref(), origin, out);
            }
        }
        let app = WidgetFactory::new();
        for width in [375., 800., 1200.] {
            app.viewport.set(Size::new(width, 600.));
            for placement in [0, 1, 2] {
                app.tab_bar_placement.set(placement);
                let demo_width = app.demo_width(320.);
                let mut root = app.tab_view().create_element();
                root.layout(LayoutConstraints::tight(demo_width, 112.));
                let mut items = Vec::new();
                labels(root.as_ref(), Point::ZERO, &mut items);
                items.retain(|(label, _)| ["Ch 1", "Ch 2", "Master"].contains(&label.as_str()));
                assert_eq!(items.len(), 3);
                for (index, (label, rect)) in items.iter().enumerate() {
                    let center = rect.origin.x + rect.size.width * 0.5;
                    let expected = demo_width * 0.5 + (index as f32 - 1.) * 80.;
                    assert!(
                        (center - expected).abs() < 0.5,
                        "{label} at {center}, expected {expected} (width {width}, placement {placement})"
                    );
                    assert!((rect.origin.y - items[0].1.origin.y).abs() < 0.5);
                    assert!(rect.origin.y >= 0. && rect.origin.y + rect.size.height <= 112.);
                }
            }
        }
    }

    #[test]
    fn nested_scroll_demo_starts_with_readable_leading_heading_and_track_labels() {
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        fn labels(element: &dyn Element, origin: Point, out: &mut Vec<(String, Point)>) {
            let origin = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            if let Some(render) = element.render_object()
                && render.as_any().is::<scarlet_ui::views::TextRenderObject>()
            {
                let mut paint = PaintContext::new();
                render.paint(&mut paint, origin);
                for command in paint.commands() {
                    if let PaintCommand::DrawText { text, position, .. } = command {
                        out.push((text.clone(), *position));
                    }
                }
            }
            for child in element.children() {
                labels(child.as_ref(), origin, out);
            }
        }
        let app = WidgetFactory::new();
        for width in [375.0, 562.0, 1200.0] {
            app.viewport.set(Size::new(width, 600.0));
            let mut root = app.scroll_view().create_element();
            root.layout(LayoutConstraints::tight(app.demo_width(320.0), 160.0));
            let mut positions = Vec::new();
            labels(root.as_ref(), Point::ZERO, &mut positions);
            assert_eq!(positions.len(), 9);
            for (label, point) in positions {
                assert_eq!(
                    point.x, 12.0,
                    "{label} must start within the initial viewport"
                );
            }
        }
    }

    #[test]
    fn factory_pages_fit_responsive_widths_and_align_every_row_label() {
        use scarlet_ui::element::LayoutConstraints;
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        fn inspect(
            element: &dyn Element,
            origin: Point,
            root: bool,
            width: f32,
            labels: &mut Vec<(String, Rect)>,
            extent: &mut f32,
        ) {
            let origin = Point::new(
                origin.x + element.position().x,
                origin.y + element.position().y,
            );
            if let Some(render) = element.render_object() {
                let bounds = Rect::new(origin, render.size());
                assert!(
                    origin.x >= -0.5 && origin.x + bounds.size.width <= width + 0.5,
                    "{} exceeds page width{}: {:?}",
                    element.type_name_debug(),
                    width,
                    bounds
                );
                if render.as_any().is::<FactoryPageLayout>() {
                    *extent = bounds.size.height;
                }
                if *extent > 0.0 {
                    assert!(
                        origin.y + bounds.size.height <= *extent + 0.5,
                        "{} extends below the natural page height: {bounds:?} > {}",
                        element.type_name_debug(),
                        *extent
                    );
                }
                if render.as_any().is::<scarlet_ui::views::TextRenderObject>() {
                    let mut paint = PaintContext::new();
                    render.paint(&mut paint, origin);
                    for command in paint.commands() {
                        if let PaintCommand::DrawText { text, .. } = command {
                            labels.push((text.clone(), bounds));
                        }
                    }
                }
                // Deliberately oversized demo content is allowed only inside
                // a real clipping viewport (nested ScrollView/TextView/TabView).
                if !root && render.clip_bounds(origin).is_some() {
                    return;
                }
            }
            for child in element.children() {
                inspect(child.as_ref(), origin, false, width, labels, extent);
            }
        }
        let app = WidgetFactory::new();
        app.text_value.set("Retained across resize".into());
        app.slider_value.set(0.73);
        for width in [375.0, 390.0, 562.0, 860.0, 1200.0] {
            app.viewport.set(Size::new(width, 600.0));
            let pages: Vec<(&str, Box<dyn View>, &[&str])> = vec![
                (
                    "Overview",
                    Box::new(app.overview_page()),
                    &[
                        "ProgressView",
                        "Rectangle",
                        "Divider",
                        "ScrollView both",
                        "Split layout",
                        "SplitView",
                        "Tab bar",
                        "TabView",
                    ],
                ),
                (
                    "Controls",
                    Box::new(app.controls_page()),
                    &["Button", "Toggle", "Slider", "ProgressView"],
                ),
                (
                    "Inputs",
                    Box::new(app.inputs_page()),
                    &["TextField", "Select", "Slider", "TextView"],
                ),
                (
                    "Display",
                    Box::new(app.display_page()),
                    &[
                        "Text",
                        "Rectangle",
                        "Divider",
                        "Tabler Icons",
                        "ScrollView both",
                        "ScrollView x",
                        "ScrollView y",
                        "Split layout",
                        "SplitView",
                        "Tab bar",
                        "TabView",
                    ],
                ),
                ("SGFX", Box::new(app.sgfx_page()), &[]),
            ];
            for (name, page, row_labels) in pages {
                let mut root = page.create_element();
                root.layout(LayoutConstraints::tight(app.content_width(), 180.0));
                let mut labels = Vec::new();
                let mut extent = 0.0;
                inspect(
                    root.as_ref(),
                    Point::ZERO,
                    true,
                    app.content_width(),
                    &mut labels,
                    &mut extent,
                );
                let mut previous_bottom = 0.0;
                for expected in row_labels {
                    let (_, bounds) = labels
                        .iter()
                        .find(|(label, _)| label == expected)
                        .unwrap_or_else(|| panic!("missing {expected} in {name}: {labels:?}"));
                    assert!(
                        (bounds.origin.x - app.page_padding()).abs() < 0.5,
                        "{name} {width}: {expected} label is not aligned: {bounds:?}"
                    );
                    assert!(bounds.origin.y >= previous_bottom);
                    previous_bottom = bounds.origin.y + bounds.size.height;
                }
                assert!(
                    extent >= previous_bottom + app.page_padding() - 0.5,
                    "{name}: natural content height must include final row and bottom padding"
                );
                println!(
                    "{name} window={width} page={} extent={extent} labels={labels:?}",
                    app.content_width()
                );
                // A collapsed/short viewport must not collapse natural content.
                root.layout(LayoutConstraints::tight(app.content_width(), 0.0));
                let mut after = 0.0;
                inspect(
                    root.as_ref(),
                    Point::ZERO,
                    true,
                    app.content_width(),
                    &mut Vec::new(),
                    &mut after,
                );
                assert!((after - extent).abs() < 0.5);
                root.layout(LayoutConstraints::tight(app.content_width(), 180.0));
                root.handle_event(
                    &Event::Mouse(MouseEvent::Wheel {
                        delta_x: 0,
                        delta_y: -10_000,
                        x: 4,
                        y: 80,
                        phase: scarlet_ui::event::WheelPhase::Started,
                        source: scarlet_ui::event::ScrollSource::Trackpad,
                    }),
                    Phase::Target,
                );
                let content_bottom = root.children()[0].position().y + extent;
                assert!(
                    (content_bottom - 180.0).abs() < 0.5,
                    "{name} at {width}: final content must be reachable, bottom={content_bottom}"
                );
            }
        }
        assert_eq!(app.text_value.get(), "Retained across resize");
        assert_eq!(app.slider_value.get(), 0.73);
    }

    #[test]
    fn sketch_draws_with_stock_controls_and_retains_normalized_strokes() {
        use scarlet_ui::{ComponentElement, RenderingPipeline};
        fn bounds<T: 'static>(e: &dyn Element, parent: Point, out: &mut Vec<Rect>) {
            let p = Point::new(parent.x + e.position().x, parent.y + e.position().y);
            if let Some(r) = e.render_object() {
                if r.as_any().is::<T>() {
                    out.push(Rect::new(p, r.size()));
                }
            }
            for c in e.children() {
                bounds::<T>(c.as_ref(), p, out);
            }
        }
        fn rects<T: 'static>(p: &RenderingPipeline) -> Vec<Rect> {
            let mut out = Vec::new();
            bounds::<T>(p.element_tree().root().unwrap(), Point::ZERO, &mut out);
            out
        }
        fn event(p: &mut RenderingPipeline, event: MouseEvent) {
            p.handle_event(&Event::Mouse(event));
            p.render_with_damage();
        }
        fn click(p: &mut RenderingPipeline, r: Rect) {
            let x = (r.origin.x + r.size.width / 2.) as i32;
            let y = (r.origin.y + r.size.height / 2.) as i32;
            event(p, MouseEvent::Moved { x, y });
            event(
                p,
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
            );
            event(
                p,
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
            );
        }
        let app = Specimen::new();
        let mut p = RenderingPipeline::new();
        p.set_root(Box::new(ComponentElement::new_with_builder(
            app.clone(),
            |app| Box::new(app.body()),
        )));
        p.layout_initial();
        let initial = app.strokes.get();
        assert_eq!(initial.len(), 3);
        assert_eq!(app.frame.get().draw_count(), 1);
        for width in [320., 400., 460.] {
            app.resize_specimen(width);
            p.resize(Size::new(width, 300.));
            p.render_with_damage();
            let paper = rects::<scarlet_ui::SgfxCanvasRenderObject>(&p)[0];
            let buttons = rects::<scarlet_ui::views::ButtonRenderObject>(&p);
            let slider = rects::<scarlet_ui::views::SliderRenderObject>(&p)[0];
            eprintln!(
                "sketch width={width}: paper={paper:?}, buttons={buttons:?}, slider={slider:?}"
            );
            for r in buttons.into_iter().chain([paper, slider]) {
                assert!(
                    r.origin.x >= 0.
                        && r.origin.y >= 0.
                        && r.origin.x + r.size.width <= width + 0.1
                        && r.origin.y + r.size.height <= 300.1,
                    "{r:?}"
                );
            }
            assert_eq!(app.strokes.get(), initial);
        }
        let buttons = rects::<scarlet_ui::views::ButtonRenderObject>(&p);
        click(&mut p, buttons[4]);
        assert_eq!(app.color.get(), 2);
        let slider = rects::<scarlet_ui::views::SliderRenderObject>(&p)[0];
        let x = (slider.origin.x + slider.size.width * 0.85) as i32;
        let y = (slider.origin.y + slider.size.height / 2.) as i32;
        event(
            &mut p,
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        event(
            &mut p,
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        assert!(app.line_width.get() > 5.);
        let paper = rects::<scarlet_ui::SgfxCanvasRenderObject>(&p)[0];
        let x = paper.origin.x as i32 + 35;
        let y = paper.origin.y as i32 + 40;
        event(
            &mut p,
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        assert!(app.active.get());
        event(
            &mut p,
            MouseEvent::Moved {
                x: x + 35,
                y: y + 20,
            },
        );
        event(
            &mut p,
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 500,
                y: y + 30,
                click_count: 1,
            },
        );
        assert!(!app.active.get());
        let drawn = app.strokes.get();
        assert_eq!(drawn.len(), 4);
        assert_eq!(drawn[3].color, Specimen::COLORS[2]);
        assert_eq!(drawn[3].width, app.line_width.get());
        assert!(drawn[3].points.len() >= 3);
        assert_eq!(drawn[3].points.last().unwrap().x, 1.);
        app.resize_specimen(320.);
        p.resize(Size::new(320., 300.));
        p.render_with_damage();
        assert_eq!(app.strokes.get(), drawn);
        let paper = rects::<scarlet_ui::SgfxCanvasRenderObject>(&p)[0];
        let x = paper.origin.x as i32 + 20;
        let y = paper.origin.y as i32 + 20;
        event(
            &mut p,
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            },
        );
        event(
            &mut p,
            MouseEvent::ButtonCancelled {
                button: MouseButton::Left,
                x,
                y,
            },
        );
        assert!(!app.active.get());
        let canceled = app.strokes.get();
        event(
            &mut p,
            MouseEvent::Moved {
                x: x + 70,
                y: y + 20,
            },
        );
        assert_eq!(app.strokes.get(), canceled);
        let undo = rects::<scarlet_ui::views::ButtonRenderObject>(&p)[0];
        click(&mut p, undo);
        assert_eq!(app.strokes.get(), drawn);
        let clear = rects::<scarlet_ui::views::ButtonRenderObject>(&p)[1];
        click(&mut p, clear);
        assert!(app.strokes.get().is_empty());
        assert_eq!(app.frame.get().draw_count(), 0);
        app.undo();
        assert!(app.strokes.get().is_empty());
    }

    #[test]
    fn sketch_direct_touch_routes_local_coordinates_and_cancels_multitouch() {
        use scarlet_ui::event::{TouchChange, TouchFrame, TouchPhase};
        use scarlet_ui::{ComponentElement, RenderingPipeline};
        let app = Specimen::new();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(ComponentElement::new_with_builder(
            app.clone(),
            |app| Box::new(app.body()),
        )));
        pipeline.layout_initial();
        pipeline.resize(Size::new(400.0, 300.0));
        pipeline.render_with_damage();
        fn paper_origin(element: &dyn Element, parent: Point) -> Option<Point> {
            let origin = Point::new(
                parent.x + element.position().x,
                parent.y + element.position().y,
            );
            if element
                .render_object()
                .is_some_and(|render| render.as_any().is::<scarlet_ui::SgfxCanvasRenderObject>())
            {
                return Some(origin);
            }
            element
                .children()
                .iter()
                .find_map(|child| paper_origin(child.as_ref(), origin))
        }
        let paper = paper_origin(pipeline.element_tree().root().unwrap(), Point::ZERO).unwrap();
        let start = ((paper.x + 20.0) as i32, (paper.y + 20.0) as i32);
        let mut serial = 0;
        let mut send = |id, phase, x, y| {
            serial += 1;
            pipeline.handle_event(&Event::TouchFrame(TouchFrame {
                seat_id: 0,
                serial,
                time_ns: serial * 16_000_000,
                changes: vec![TouchChange {
                    seat_id: 0,
                    serial,
                    time_ns: serial * 16_000_000,
                    id,
                    phase,
                    x,
                    y,
                    pressure: None,
                    touch_major: None,
                }],
            }));
            pipeline.render_with_damage();
        };
        send(1, TouchPhase::Down, start.0, start.1);
        assert_eq!(app.strokes.get().len(), 4);
        assert_eq!(
            app.strokes.get()[3].points[0],
            Point::new(20.0 / 376.0, 20.0 / 184.0)
        );
        send(1, TouchPhase::Move, 90, 100);
        assert!(
            app.active.get(),
            "awarded drag keeps drawing after press cancellation"
        );
        send(1, TouchPhase::Up, 450, 250);
        assert!(!app.active.get());
        assert_eq!(
            app.strokes.get()[3].points.last(),
            Some(&Point::new(1.0, 1.0))
        );
        let released = app.strokes.get();
        send(1, TouchPhase::Move, 100, 100);
        assert_eq!(app.strokes.get(), released);

        // A second finger outside the paper must cancel even a pending stroke.
        send(2, TouchPhase::Down, start.0, start.1);
        send(3, TouchPhase::Down, 20, 250);
        assert!(!app.active.get());
        let interrupted = app.strokes.get();
        send(2, TouchPhase::Move, 150, 120);
        send(3, TouchPhase::Up, 20, 250);
        send(2, TouchPhase::Up, 150, 120);
        assert_eq!(app.strokes.get(), interrupted);
        send(4, TouchPhase::Down, 40, 80);
        send(4, TouchPhase::Move, 80, 110);
        send(4, TouchPhase::Cancel, 80, 110);
        let canceled = app.strokes.get();
        send(4, TouchPhase::Move, 120, 130);
        assert_eq!(app.strokes.get(), canceled);
        assert!(!app.active.get());
        assert_eq!(app.touch_id.get(), None);
    }

    #[test]
    fn sketch_total_point_budget_recovers_after_clear() {
        let app = Specimen::new();
        app.strokes.set(vec![SketchStroke {
            points: vec![Point::new(0.5, 0.5); 4096],
            color: Specimen::COLORS[0],
            width: 3.0,
        }]);
        app.begin_stroke(10, 10);
        assert!(!app.active.get());
        assert_eq!(app.strokes.get().len(), 1);
        app.active.set(true);
        app.extend_stroke(20, 20);
        assert_eq!(app.strokes.get()[0].points.len(), 4096);
        app.clear();
        app.begin_stroke(10, 10);
        assert!(app.active.get());
        assert_eq!(app.strokes.get()[0].points.len(), 1);
        assert_eq!(app.frame.get().draw_count(), 1);
    }

    #[test]
    fn factory_navigation_after_text_view_edit_needs_only_one_click() {
        use scarlet_ui::event::{KeyCode, KeyEvent, KeyModifiers};
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        use scarlet_ui::{ComponentElement, RenderingPipeline, SceneBuilder};

        fn find_element(
            element: &dyn Element,
            predicate: &dyn Fn(&dyn Element) -> bool,
        ) -> Option<scarlet_ui::ElementId> {
            if predicate(element) {
                return Some(element.id());
            }
            element
                .children()
                .iter()
                .find_map(|child| find_element(child.as_ref(), predicate))
        }
        fn sidebar_label(pipeline: &RenderingPipeline, label: &str) -> (i32, i32) {
            let tree = pipeline.element_tree();
            let id = find_element(tree.root().unwrap(), &|element| {
                element.type_name_debug().contains("NavigationSidebar")
            })
            .expect("sidebar");
            let path = tree.find_path_ids(id).unwrap();
            let element = tree.find_element(id).unwrap();
            let origin = path.iter().fold(Point::ZERO, |origin, id| {
                let pos = tree.find_element(*id).unwrap().position();
                Point::new(origin.x + pos.x, origin.y + pos.y)
            });
            let mut paint = PaintContext::new();
            element.render_object().unwrap().paint(&mut paint, origin);
            paint
                .commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::DrawText { text, position, .. } if text == label => {
                        Some((position.x as i32 + 4, position.y as i32 + 8))
                    }
                    _ => None,
                })
                .expect("sidebar label")
        }
        fn editor_id(pipeline: &RenderingPipeline) -> Option<scarlet_ui::ElementId> {
            find_element(pipeline.element_tree().root().unwrap(), &|element| {
                element.render_object().is_some_and(|render| {
                    render
                        .as_any()
                        .is::<scarlet_ui::views::TextViewRenderObject>()
                })
            })
        }
        fn click(pipeline: &mut RenderingPipeline, (x, y): (i32, i32)) {
            for event in [
                MouseEvent::Moved { x, y },
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
            ] {
                pipeline.handle_event(&Event::Mouse(event));
                pipeline.render_with_damage();
            }
        }
        let app = WidgetFactory::new();
        let mut pipeline = RenderingPipeline::new();
        // Exactly the application's scene builder and listenables, rather than
        // a cached NavigationView. State edits reconstruct the complete Window.
        pipeline.set_root(Box::new(ComponentElement::new_with_builder(
            app.clone(),
            |app| {
                let mut builder = SceneBuilder::new();
                app.scenes().build(&mut builder);
                builder.into_declarations().remove(0).view
            },
        )));
        pipeline.layout_initial();
        pipeline.render_with_damage();
        let inputs = sidebar_label(&pipeline, "Inputs");
        click(&mut pipeline, inputs);
        let id = editor_id(&pipeline).expect("Inputs page contains TextView");
        let path = pipeline.element_tree().find_path_ids(id).unwrap();
        let origin = path.iter().fold(Point::ZERO, |origin, id| {
            let pos = pipeline
                .element_tree()
                .find_element(*id)
                .unwrap()
                .position();
            Point::new(origin.x + pos.x, origin.y + pos.y)
        });
        click(&mut pipeline, (origin.x as i32 + 40, origin.y as i32 + 45));
        for event in [
            KeyEvent::Pressed {
                keycode: KeyCode::End,
                modifiers: KeyModifiers::default(),
            },
            KeyEvent::Char { c: 'x' },
            KeyEvent::Pressed {
                keycode: KeyCode::Backspace,
                modifiers: KeyModifiers::default(),
            },
        ] {
            assert!(pipeline.handle_event(&Event::Keyboard(event)));
            pipeline.render_with_damage();
        }
        let display = sidebar_label(&pipeline, "Display");
        click(&mut pipeline, display);
        assert!(
            editor_id(&pipeline).is_none(),
            "first click must replace Inputs with Display"
        );
    }

    #[test]
    fn shared_menu_reflects_control_state_and_keeps_browser_shortcut_visible() {
        let app = WidgetFactory::new();
        let model = app.menu_model();
        assert_eq!(
            model
                .items()
                .iter()
                .map(MenuItemModel::title)
                .collect::<Vec<_>>(),
            ["Factory", "View", "Tools"]
        );
        let MenuEntry::Item(reset) = &model.items()[0].children_value()[0] else {
            panic!("reset entry");
        };
        assert!(!reset.enabled_value());
        let MenuEntry::Item(toggle) = &model.items()[1].children_value()[0] else {
            panic!("toggle entry");
        };
        assert!(toggle.checked_value());
        let MenuEntry::Item(text) = &model.items()[2].children_value()[1] else {
            panic!("text entry");
        };
        assert_eq!(text.shortcut_value(), Some("Ctrl+L"));
        app.toggle_on.set(false);
        let refreshed = app.menu_model();
        let MenuEntry::Item(reset) = &refreshed.items()[0].children_value()[0] else {
            panic!("reset entry");
        };
        assert!(reset.enabled_value());
        let MenuEntry::Item(toggle) = &refreshed.items()[1].children_value()[0] else {
            panic!("toggle entry");
        };
        assert!(!toggle.checked_value());
    }

    #[test]
    fn factory_defaults_keep_the_original_layout() {
        assert_eq!(
            split_layout_for_selection(0),
            (SplitAxis::Horizontal, SplitAxisPolicy::Fixed)
        );
        assert_eq!(tab_bar_placement_for_selection(0), TabBarPlacement::Top);
    }

    #[test]
    fn vertical_split_modes_leave_room_to_drag_the_divider() {
        for selection in [1, 2] {
            let (axis, axis_policy) = split_layout_for_selection(selection);
            let (min_first, min_second) = split_minimum_extents_for_selection(selection);
            let split = SplitView::new(Text::new("A"), Text::new("B"))
                .axis(axis)
                .axis_policy(axis_policy)
                .adaptive_stack_narrow_width(420.0)
                .fraction(0.34)
                .min_first(min_first)
                .min_second(min_second);
            let mut render_object = SplitViewRenderObject::from_view(&split);

            render_object.layout(LayoutConstraints::tight(320.0, 160.0));
            assert_eq!(render_object.effective_axis(), SplitAxis::Vertical);

            let initial_extent = render_object.first_extent();
            let divider_y = initial_extent.round() as i32;
            assert!(render_object.handle_event(
                &Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 20,
                    y: divider_y,
                    click_count: 1,
                }),
                Phase::Target,
            ));
            assert!(render_object.handle_event(
                &Event::Mouse(MouseEvent::Moved { x: 20, y: 100 }),
                Phase::Target,
            ));
            assert!(render_object.first_extent() > initial_extent + 20.0);
        }
    }
    #[test]
    fn factory_demo_buttons_keep_the_visible_default_glass_material() {
        use scarlet_ui::element::LayoutConstraints;
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        fn inspect(element: &dyn Element) -> usize {
            let mut count = 0;
            if let Some(render) = element.render_object()
                && render
                    .as_any()
                    .is::<scarlet_ui::views::ButtonRenderObject>()
            {
                let mut paint = PaintContext::new();
                render.paint(&mut paint, Point::ZERO);
                let tint = paint
                    .commands()
                    .iter()
                    .find_map(|command| match command {
                        PaintCommand::BackdropFilterRoundedRect { tint, .. } => Some(*tint),
                        _ => None,
                    })
                    .expect("demo button should retain its glass material");
                let expected = GlassMaterial::button().tint;
                assert_eq!(
                    (tint.r, tint.g, tint.b),
                    (expected.r, expected.g, expected.b)
                );
                // Disabled buttons retain the same tint with lower opacity.
                assert!(tint.a <= expected.a);
                count += 1;
            }
            count
                + element
                    .children()
                    .iter()
                    .map(|child| inspect(child.as_ref()))
                    .sum::<usize>()
        }
        let app = WidgetFactory::new();
        for (mut element, expected_count) in [
            (app.button().create_element(), 1),
            (app.control_study().create_element(), 2),
        ] {
            element.layout(LayoutConstraints::loose(1000.0, 2000.0));
            assert_eq!(inspect(element.as_ref()), expected_count);
        }
    }

    #[test]
    fn material_backdrop_padding_fits_narrow_pages() {
        let app = WidgetFactory::new();
        for width in [375.0, 590.0, 1180.0] {
            app.viewport.set(Size::new(width, 600.0));
            let backdrop = MaterialBackdropRender {
                phase: 0,
                size: Size::new(app.row_width(), 140.0),
            };
            assert!(backdrop.paint_outsets().left <= app.page_padding());
            assert!(backdrop.paint_outsets().right <= app.page_padding());
        }
    }

    /// Release-only observational benchmark of the real Controls scene. It
    /// renders offscreen with the same CPU pipeline, not a native screenshot.
    #[test]
    #[ignore = "manual release CPU material performance measurement"]
    fn measure_factory_glass_frame_budget() {
        use scarlet_ui::event::{Event, MouseButton, MouseEvent};
        use scarlet_ui::renderer::{PaintCommand, PaintContext};
        use scarlet_ui::{ComponentElement, RenderingPipeline, SceneBuilder};
        fn label_position(element: &dyn Element, parent: Point) -> Option<(i32, i32)> {
            let origin = Point::new(
                parent.x + element.position().x,
                parent.y + element.position().y,
            );
            if element.type_name_debug().contains("NavigationSidebar") {
                let mut ctx = PaintContext::new();
                element.render_object().unwrap().paint(&mut ctx, origin);
                return ctx.commands().iter().find_map(|command| match command {
                    PaintCommand::DrawText { text, position, .. } if text == "Controls" => {
                        Some((position.x as i32 + 4, position.y as i32 + 8))
                    }
                    _ => None,
                });
            }
            element
                .children()
                .iter()
                .find_map(|child| label_position(child.as_ref(), origin))
        }
        for scale in [1000, 2000] {
            let app = WidgetFactory::new();
            let size = Size::new(1180.0, 812.0);
            app.viewport.set(size);
            let mut pipeline = RenderingPipeline::new();
            pipeline.set_root(Box::new(ComponentElement::new_with_builder(
                app.clone(),
                |app| {
                    let mut builder = SceneBuilder::new();
                    app.scenes().build(&mut builder);
                    builder.into_declarations().remove(0).view
                },
            )));
            pipeline.set_scale_milli(scale);
            pipeline.resize(size);
            pipeline.layout_initial();
            let (x, y) = label_position(pipeline.element_tree().root().unwrap(), Point::ZERO)
                .expect("Controls navigation label");
            pipeline.render_with_damage();
            for event in [
                MouseEvent::Moved { x, y },
                MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
                MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x,
                    y,
                    click_count: 1,
                },
            ] {
                pipeline.handle_event(&Event::Mouse(event));
                pipeline.render_with_damage();
            }
            for index in 1..=3 {
                app.study_count.set(index);
                assert!(pipeline.render_with_damage().is_some());
            }
            let mut milliseconds = Vec::new();
            for index in 100..120 {
                app.study_count.set(index);
                let start = std::time::Instant::now();
                assert!(pipeline.render_with_damage().is_some());
                milliseconds.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            milliseconds.sort_by(f64::total_cmp);
            assert!(
                pipeline.render_with_damage().is_none(),
                "idle glass must not keep repainting"
            );
            println!(
                "FACTORY_GLASS_CPU scale={} median_ms={:.3} p95_ms={:.3} logical=1180x812 samples=20",
                scale, milliseconds[10], milliseconds[19]
            );
        }
    }
    #[test]
    fn static_controls_retains_one_background_backed_scroll_recording() {
        use scarlet_ui::event::{Event, MouseEvent, ScrollSource, WheelPhase};
        use scarlet_ui::renderer::{BackendFrame, PaintBackend, PaintCommand, PaintContext};
        use scarlet_ui::{ComponentElement, RenderingPipeline};
        use std::{cell::RefCell, rc::Rc};
        struct Capture(Rc<RefCell<Vec<(u64, Rect)>>>);
        impl PaintBackend for Capture {
            fn supports_retained_display_lists(&self) -> bool {
                true
            }
            fn resize(&mut self, _: Size, _: u32) {}
            fn render<'a>(
                &'a mut self,
                ctx: &PaintContext<'_>,
                _: Color,
                _: Option<&[Rect]>,
                _: Option<&[scarlet_ui::compositor::DamageRect]>,
            ) -> scarlet_ui::Result<BackendFrame<'a>> {
                let mut lists = self.0.borrow_mut();
                lists.clear();
                for command in ctx.commands() {
                    if let PaintCommand::DrawDisplayList { list, .. } = command {
                        if list.reads_backdrop() {
                            if let Some(bounds) = list.raster_cache_bounds() {
                                lists.push((list.identity(), bounds));
                            }
                        }
                    }
                }
                Ok(BackendFrame::External)
            }
        }
        let app = WidgetFactory::new();
        let size = Size::new(500., 800.);
        app.viewport.set(size);
        let lists = Rc::new(RefCell::new(Vec::new()));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(ComponentElement::new_with_builder(
            app.clone(),
            |app| Box::new(app.controls_page().frame(500., 800.)),
        )));
        pipeline.set_paint_backend(Box::new(Capture(Rc::clone(&lists))));
        pipeline.set_scale_milli(2000);
        pipeline.resize(size);
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let original = lists.borrow().clone();
        assert_eq!(
            original.len(),
            1,
            "the static page and its glass background must remain a single cacheable layer"
        );
        assert_eq!(original[0].1.size.width, size.width);
        assert!(
            original[0].1.size.height > size.height,
            "the layer covers the entire scroll content"
        );
        for delta in [-4, -4, -40] {
            pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
                x: 100,
                y: 100,
                delta_x: 0,
                delta_y: delta,
                phase: WheelPhase::Moved,
                source: ScrollSource::Trackpad,
            }));
            pipeline.render_for_present().unwrap();
            assert_eq!(
                *lists.borrow(),
                original,
                "scrolling must reuse the immutable page recording"
            );
        }
        app.study_count.set(1);
        pipeline.render_for_present().unwrap();
        assert_eq!(lists.borrow().len(), 1);
        assert_ne!(
            lists.borrow()[0].0,
            original[0].0,
            "a changed backdrop must replace the cached recording"
        );
    }
    #[test]
    fn slider_notifications_preserve_unrelated_scroll_recordings_and_cache_lineage() {
        use scarlet_ui::renderer::{
            BackendFrame, DisplayList, PaintBackend, PaintCommand, PaintContext,
        };
        use scarlet_ui::{ComponentElement, RenderingPipeline};
        use std::{cell::RefCell, rc::Rc, sync::Arc};
        struct Capture(Rc<RefCell<Option<Arc<DisplayList>>>>);
        impl PaintBackend for Capture {
            fn supports_retained_display_lists(&self) -> bool {
                true
            }
            fn resize(&mut self, _: Size, _: u32) {}
            fn render<'a>(
                &'a mut self,
                ctx: &PaintContext<'_>,
                _: Color,
                _: Option<&[Rect]>,
                _: Option<&[scarlet_ui::compositor::DamageRect]>,
            ) -> scarlet_ui::Result<BackendFrame<'a>> {
                for command in ctx.commands() {
                    if let PaintCommand::DrawDisplayList { list, .. } = command {
                        if list
                            .raster_cache_bounds()
                            .is_some_and(|r| r.size.height > 800.)
                        {
                            *self.0.borrow_mut() = Some(list.clone());
                        }
                    }
                }
                Ok(BackendFrame::External)
            }
        }
        let app = WidgetFactory::new();
        app.viewport.set(Size::new(500., 800.));
        let capture = Rc::new(RefCell::new(None));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(Box::new(ComponentElement::new_with_builder(
            app.clone(),
            |app| {
                // Intentionally keep the broad subscription: recording reuse must
                // not depend on changing the application's State ownership model.
                let _ = app.slider_value.get();
                Box::new(app.controls_page().frame(500., 800.))
            },
        )));
        pipeline.set_paint_backend(Box::new(Capture(capture.clone())));
        pipeline.resize(Size::new(500., 800.));
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let original = capture.borrow().as_ref().unwrap().clone();
        let static_ids = original
            .context()
            .commands()
            .iter()
            .filter_map(|c| match c {
                PaintCommand::DrawDisplayList { origin, list } if origin.y < 500. => {
                    Some(list.identity())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            static_ids.len() > 10,
            "the page preserves separate child recordings"
        );
        for value in [0.1, 0.3, 0.6, 0.9, 0.42] {
            app.slider_value.set(value);
            pipeline.render_for_present().unwrap();
            let next = capture.borrow().as_ref().unwrap().clone();
            assert_eq!(next.cache_identity(), original.cache_identity());
            let ids = next
                .context()
                .commands()
                .iter()
                .filter_map(|c| match c {
                    PaintCommand::DrawDisplayList { list, .. } => Some(list.identity()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                static_ids.iter().all(|id| ids.contains(id)),
                "slider changes must not recreate unrelated records"
            );
            let damage = next
                .damage_since(&original, 1000)
                .expect("ordinary controls have bounded paint");
            assert!(
                damage.iter().all(|r| r.top() > 500.),
                "slider updates must not invalidate the static material study: {damage:?}"
            );
        }
    }
}
