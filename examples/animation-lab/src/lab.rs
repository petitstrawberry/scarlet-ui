//! One application, with the same pages on native and WebGPU.
use crate::{basics::Basics, code::CodeBrowser, layout::LayoutLab, play::Playground};
use core::{any::Any, time::Duration};
use scarlet_ui::{
    NavigationLink, NavigationView, PlatformWindow, WindowContext,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    platform::WindowPlacement,
    prelude::*,
    renderer::PaintContext,
    views::containers::ViewTuple,
};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use std::{cell::RefCell, rc::Rc};
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

const SCENE: &str = "animation-lab";
const SURFACE: Color = Color::rgb_f32(1.0, 0.988, 0.996);

#[derive(Clone)]
pub(crate) struct AnimationLab {
    width: State<f32>,
    selected: State<usize>,
    code: [CodeBrowser; 3],
    play: Playground,
    basics: Basics,
    layout: LayoutLab,
    playback: Rc<RefCell<Playback>>,
}

struct Playback {
    started: Instant,
    booted: bool,
    smoke: bool,
    step: usize,
    frames: usize,
    surface: Option<Color>,
}

impl AnimationLab {
    pub(crate) fn new(smoke: bool) -> Self {
        let mut app = Self {
            width: State::new(scarlet_ui::state::generate_state_id(), 1000.0),
            selected: State::new(scarlet_ui::state::generate_state_id(), 0),
            code: std::array::from_fn(|_| CodeBrowser::default()),
            play: Playground::default(),
            basics: Basics::default(),
            layout: LayoutLab::default(),
            playback: Rc::new(RefCell::new(Playback {
                started: Instant::now(),
                booted: false,
                smoke,
                step: 0,
                frames: 0,
                surface: None,
            })),
        };
        app.selected = app.navigation().selected_index_state().clone();
        app
    }
    fn scroll_page<V: View + Clone>(&self, page: V) -> impl View + Clone + use<V> {
        let width = self.width.clone();
        // The page owns the scroll position. Its nested keys preserve animated
        // card identity, but must not anchor scrolling when controls reflow or
        // cards move. Otherwise the initial width measurement scrolls Play by
        // the change in the first card's position (44 px at a 715 px viewport).
        ScrollView::new(NaturalContent(page).key("animation-lab-page"))
            .vertical()
            // These pages scroll vertically. Browser pixel wheels have no
            // reliable gesture identity, so requiring a 4:1 axis ratio on
            // every sample drops ordinary diagonal finger motion mid-scroll.
            .exclusive_wheel_axis_lock(1.0, 0.0)
            .on_geometry_change(
                |geometry| {
                    let insets = geometry.safe_area_insets();
                    (geometry.size().width - insets.left - insets.right).max(0.0)
                },
                move |next| width.set(next),
            )
    }
    fn header(icon: Icon, code: CodeBrowser) -> NavigationHeader {
        NavigationHeader::toolbar(move || {
            let leading: Box<dyn View> = if code.is_open() {
                Box::new(code.back_button())
            } else {
                Box::new(IconView::new(icon).size(IconSize::Medium))
            };
            HStack::new(Views(vec![
                leading,
                Box::new(
                    Text::new(if code.is_open() {
                        "Source code"
                    } else {
                        "Animation Lab"
                    })
                    .text_style(TextStyle::Headline),
                ),
                Box::new(Spacer::new()),
            ]))
            .spacing(10.0)
        })
    }
    fn page(&self, index: usize) -> impl View + Clone + 'static + use<> {
        let code = self.code[index].clone();
        let width = self.width.get();
        let body: Box<dyn View> = if code.is_open() {
            Box::new(code.page(width))
        } else {
            match index {
                0 => Box::new(self.play.page(width, code.clone())),
                1 => Box::new(self.basics.page(width, code.clone())),
                _ => Box::new(self.layout.page(width, code.clone())),
            }
        };
        // Source locations start at their heading; resizing/wrapping keeps the location.
        self.scroll_page(VStack::new(Views(vec![body])).alignment(Alignment::Leading))
            .key(code.location_key())
    }
    fn surface(&self) -> Color {
        let selected = self.selected.get().min(2);
        if self.code[selected].is_open() {
            crate::code::SURFACE
        } else if selected == 2 {
            self.layout.background_color()
        } else {
            SURFACE
        }
    }
    fn navigation(&self) -> NavigationView<(NavigationLink, NavigationLink, NavigationLink)> {
        let play = self.clone();
        let basics = self.clone();
        let layout = self.clone();
        scarlet_ui::navigation! {
            NavigationLink::new("Play", move || play.page(0))
                .icon(Icon::Heart).custom_header(Self::header(Icon::Heart, self.code[0].clone())),
            NavigationLink::new("Basics", move || basics.page(1))
                .icon(Icon::Adjustments).custom_header(Self::header(Icon::Adjustments, self.code[1].clone())),
            NavigationLink::new("Layout", move || layout.page(2))
                .icon(Icon::LayoutGrid).custom_header(Self::header(Icon::LayoutGrid, self.code[2].clone())),
        }.presentation(NavigationPresentation::Automatic)
            .floating_bar_bottom_overlap(if cfg!(target_arch = "wasm32") { 12.0 } else { 0.0 })
            .shows_icons(true).sidebar_width(174.0)
            .minimum_content_size(Size::new(420.0, 0.0))
    }
    fn content(&self) -> impl View + Clone + 'static {
        // The window consumes no safe-area padding. Paint the full viewport;
        // NavigationView and ScrollView still keep controls inside the safe area.
        self.navigation().background(self.surface())
    }
    pub(crate) fn wake(&self) {
        self.play.wake();
    }
}

impl View for AnimationLab {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |app| Box::new(app.content()),
        ))
    }
    fn listenables(&self) -> Vec<&dyn scarlet_ui::state::Listenable> {
        let mut states: Vec<&dyn scarlet_ui::state::Listenable> = vec![&self.width, &self.selected];
        states.extend(self.layout.listenables());
        for code in &self.code {
            states.extend(code.listenables());
        }
        states
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Application for AnimationLab {
    fn scenes(&self) -> impl Scene {
        Window::new("ScarletUI / Animation Lab", self.clone())
            .scene_key(SCENE)
            .size(Size::new(1120.0, 830.0))
            .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
            .placement(WindowPlacement::At { x: 60, y: 50 })
    }
    fn on_window_created(&mut self, _: &WindowContext, window: &mut dyn PlatformWindow) {
        // Initial layout has already measured the page inside navigation.
        // Overwriting it with the full window width includes the sidebar and
        // leaves the right-hand content clipped until the next resize.
        for code in &self.code {
            code.attach_clipboard(window.text_clipboard().ok());
        }
        self.playback.borrow_mut().started = Instant::now();
    }
    fn on_frame_presented(&mut self, _: &WindowContext) {
        self.playback.borrow_mut().frames += 1;
    }
    fn on_window_suspended_changed(&mut self, _: &WindowContext, suspended: bool) {
        if !suspended {
            self.wake();
        }
    }
    fn on_idle(&mut self) {
        for code in &self.code {
            code.poll_copy();
        }
        let surface = self.surface();
        if self.playback.borrow().surface != Some(surface) {
            self.playback.borrow_mut().surface = Some(surface);
            sync_browser_surface(surface);
        }
        let action = {
            let mut p = self.playback.borrow_mut();
            let elapsed = p.started.elapsed();
            if !p.booted && elapsed >= Duration::from_millis(650) {
                p.booted = true;
                Some(0)
            } else if p.smoke {
                let deadlines = [
                    2800, 3500, 4200, 4900, 5400, 6000, 6400, 9000, 9500, 9800, 10100, 10400,
                    11000, 11300, 11600, 12500, 13200, 13900, 14600, 15300, 16000, 16700, 17400,
                    18100, 18800, 19800, 20200, 23000,
                ];
                if p.step < deadlines.len() && elapsed >= Duration::from_millis(deadlines[p.step]) {
                    p.step += 1;
                    println!(
                        "animation lab action {} at {} ms",
                        p.step,
                        elapsed.as_millis()
                    );
                    if p.step == deadlines.len() {
                        println!(
                            "animation lab smoke complete: {} frames presented",
                            p.frames
                        );
                    }
                    Some(p.step)
                } else {
                    None
                }
            } else {
                None
            }
        };
        match action {
            Some(0) => self.play.boot(),
            Some(step @ 1..=7) => self.play.smoke_step(step),
            Some(8) => self.navigation().selected_index_state().set(1),
            Some(step) if (9..9 + Basics::SMOKE_STEPS).contains(&step) => {
                self.basics.smoke_step(step - 9)
            }
            Some(16) => self.navigation().selected_index_state().set(2),
            Some(step) if (17..17 + LayoutLab::SMOKE_STEPS).contains(&step) => {
                self.layout.smoke_step(step - 17)
            }
            Some(26) => self.navigation().selected_index_state().set(0),
            Some(27) => self.play.smoke_step(0),
            Some(28) => dismiss_window(SCENE),
            _ => {}
        }
    }
}

/// Dynamic rows preserve order and forward every child's subscriptions.
#[derive(Clone)]
pub(crate) struct Views(pub Vec<Box<dyn View>>);
impl ViewTuple for Views {
    fn create_elements(&self) -> Vec<Box<dyn Element>> {
        self.0.iter().map(|v| v.create_element()).collect()
    }
    fn clone_views(&self) -> Vec<Box<dyn View>> {
        self.0.clone()
    }
    fn collect_listenables<'a>(&'a self, into: &mut Vec<&'a dyn scarlet_ui::state::Listenable>) {
        for view in &self.0 {
            into.extend(view.listenables());
        }
    }
}

/// Measure page height from its rows, even though ScrollView supplies a finite viewport.
#[derive(Clone)]
struct NaturalContent<V: View + Clone>(V);
impl<V: View + Clone> View for NaturalContent<V> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |_| NaturalLayout(Size::ZERO),
            |_, _| UpdateResult::Updated,
            |view| vec![view.0.clone_view()],
        ))
    }
    fn listenables(&self) -> Vec<&dyn scarlet_ui::state::Listenable> {
        self.0.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct NaturalLayout(Size);
impl ElementRenderObject for NaturalLayout {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.0 = Size::new(constraints.max_width, 0.0);
        self.0
    }
    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        self.0 = children
            .first_mut()
            .map(|child| {
                child.set_position(Point::ZERO);
                child.layout(LayoutConstraints::new(
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
    fn paint(&self, _: &mut PaintContext<'_>, _: Point) -> bool {
        // The viewport owns the surface, including the safe area. An opaque
        // page-height fill here would cut the theme transition off at its edges.
        true
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn sync_browser_surface(color: Color) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(root) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = root.style().set_property(
                "--lab-surface",
                &format!(
                    "rgb({},{},{})",
                    (color.r * 255.0).round() as u8,
                    (color.g * 255.0).round() as u8,
                    (color.b * 255.0).round() as u8
                ),
            );
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = color;
}

#[cfg(test)]
#[path = "lab_tests.rs"]
mod tests;
