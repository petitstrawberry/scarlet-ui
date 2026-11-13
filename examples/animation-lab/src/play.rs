//! Playful custom drawing on the shared presentation clock.
use crate::{code::CodeBrowser, lab::Views, source::ExampleId};
use core::{any::Any, time::Duration};
use scarlet_ui::{
    animation::AnimatedValue,
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    prelude::*,
    renderer::PaintContext,
};

use crate::paint::{self as artwork, Scene};

const STAGE: Size = Size::new(436.0, 168.0);
const INK: Color = Color::rgb_f32(0.22, 0.17, 0.31);
const MUTED: Color = Color::rgb_f32(0.52, 0.47, 0.59);

#[derive(Clone)]
struct Model {
    running: bool,
    fast: bool,
    palette: usize,
    progress: f32,
    load_nonce: u32,
    burst_nonce: u32,
    wake_epoch: u32,
    touched: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            running: true,
            fast: false,
            palette: 0,
            progress: 0.0,
            load_nonce: 0,
            burst_nonce: 0,
            wake_epoch: 0,
            touched: false,
        }
    }
}

#[derive(Clone)]
struct Actions(State<Model>);

impl Actions {
    fn load(&self) {
        with_animation(Animation::ease_in_out(Duration::from_millis(2200)), || {
            self.0.update(|m| {
                m.touched = true;
                m.running = true;
                m.progress = 1.0;
                m.load_nonce = m.load_nonce.wrapping_add(1);
            });
        });
    }
    fn celebrate(&self) {
        self.0.update(|m| {
            m.touched = true;
            m.running = true;
            m.burst_nonce = m.burst_nonce.wrapping_add(1);
        });
    }
    fn colors(&self) {
        self.0.update(|m| {
            m.touched = true;
            m.palette = (m.palette + 1) % 3;
        });
    }
    fn speed(&self) {
        self.0.update(|m| {
            m.touched = true;
            m.fast = !m.fast;
        });
    }
    fn pause(&self) {
        self.0.update(|m| {
            m.touched = true;
            m.running = !m.running;
        });
    }
    fn reset(&self) {
        without_animation(|| {
            self.0.set(Model {
                running: false,
                touched: true,
                ..Model::default()
            })
        });
    }
}

#[derive(Clone)]
struct PlaygroundStage {
    scene: Scene,
    model: Model,
}

impl View for PlaygroundStage {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            StageRenderer::new,
            StageRenderer::update,
            |_| Vec::new(),
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Custom properties use the same animation clock and invalidation path as the
/// built-in modifiers. Paint consumes their displayed values, not State values.
struct StageRenderer {
    scene: Scene,
    model: Model,
    clock: AnimatedValue<f32>,
    progress: AnimatedValue<f32>,
    burst: AnimatedValue<f32>,
    size: Size,
}

impl StageRenderer {
    fn new(view: &PlaygroundStage) -> Self {
        let mut this = Self {
            scene: view.scene,
            model: view.model.clone(),
            clock: AnimatedValue::new(0.0),
            progress: AnimatedValue::new(view.model.progress),
            burst: AnimatedValue::new(1.0),
            size: STAGE,
        };
        if this.model.running {
            this.start_clock();
        }
        this
    }
    fn start_clock(&mut self) {
        let phase = self.clock.value().fract();
        let millis = match self.scene {
            Scene::Orbit => 2600,
            Scene::Zigzag => 3100,
            Scene::Jelly => 1450,
            Scene::Party => 3400,
        };
        let millis = if self.model.fast { millis / 2 } else { millis };
        self.clock = AnimatedValue::new(phase);
        self.clock.retarget(
            phase + 1.0,
            Some(Animation::linear(Duration::from_millis(millis)).repeat_forever(false)),
        );
    }
    fn start_burst(&mut self) {
        self.burst = AnimatedValue::new(0.0);
        self.burst
            .retarget(1.0, Some(Animation::ease_out(Duration::from_millis(1600))));
    }
    fn update(&mut self, view: &PlaygroundStage) -> UpdateResult {
        let transaction = Transaction::current();
        let new_scene = self.scene != view.scene;
        let new_load = self.model.load_nonce != view.model.load_nonce;
        let new_burst = self.model.burst_nonce != view.model.burst_nonce;
        let restart_clock = new_scene
            || self.model.fast != view.model.fast
            || self.model.wake_epoch != view.model.wake_epoch;
        let mut changed = new_scene
            || self.model.running != view.model.running
            || self.model.palette != view.model.palette;
        self.scene = view.scene;
        self.model = view.model.clone();

        if transaction.disables_animations {
            self.clock = AnimatedValue::new(0.0);
            self.progress = AnimatedValue::new(view.model.progress);
            self.burst = AnimatedValue::new(1.0);
            return UpdateResult::Updated;
        }
        if restart_clock || (self.model.running && !self.clock.is_active()) {
            self.start_clock();
            changed = true;
        }
        if new_load {
            // A new explicit load is a new operation, including when its target
            // is already 100%. Ordinary rebuilds preserve the existing clock.
            self.progress = AnimatedValue::new(0.0);
            self.burst = AnimatedValue::new(1.0);
            changed = true;
        }
        changed |= self
            .progress
            .retarget(view.model.progress, transaction.effective_animation());
        if new_burst {
            self.start_burst();
            changed = true;
        }
        if changed {
            UpdateResult::Updated
        } else {
            UpdateResult::NoChange
        }
    }
}

impl ElementRenderObject for StageRenderer {
    fn update_needs_layout(&self) -> bool {
        false
    }
    fn update_needs_subtree_paint(&self) -> bool {
        false
    }
    fn has_visual_animation(&self) -> bool {
        self.model.running
            && (self.clock.is_active() || self.progress.is_active() || self.burst.is_active())
    }
    fn advance_visual_animation(&mut self, delta: Duration) -> bool {
        if !self.model.running {
            return false;
        }
        let loading = self.progress.is_active();
        let changed =
            self.clock.advance(delta) | self.progress.advance(delta) | self.burst.advance(delta);
        if loading && !self.progress.is_active() && self.progress.value() >= 1.0 {
            self.start_burst();
            return true;
        }
        changed
    }
    fn finish_visual_animation(&mut self) -> bool {
        self.clock.finish() | self.progress.finish() | self.burst.finish()
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = constraints.constrain(STAGE);
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn paint(&self, context: &mut PaintContext<'_>, origin: Point) -> bool {
        artwork::paint(
            context,
            origin,
            self.size,
            self.scene,
            self.clock.value().fract(),
            self.progress.value().clamp(0.0, 1.0),
            self.burst.value().clamp(0.0, 1.0),
            self.model.palette,
        );
        true
    }
    fn render(&mut self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

fn stage_card(
    scene: Scene,
    model: &Model,
    title: &str,
    caption: &str,
    index: usize,
    width: f32,
    code: CodeBrowser,
) -> impl View + Clone + 'static + use<> {
    let tints = [
        rgb(255, 239, 243),
        rgb(235, 248, 244),
        rgb(243, 237, 255),
        rgb(255, 247, 227),
    ];
    let inner = (width - 32.0).max(0.0);
    scarlet_ui::vstack! {
        scarlet_ui::hstack! {
            Text::new(title).font_size(16.0).color(INK).frame_width((inner - 30.0).max(0.0)),
            Text::new(format!("0{}", index + 1)).font_size(12.0).color(MUTED),
        }.alignment(Alignment::Leading),
        PlaygroundStage { scene, model: model.clone() }.frame(inner, STAGE.height).clip_radius(12.0),
        Text::new(caption).font_size(12.0).color(MUTED),
        code.button([ExampleId::Donut, ExampleId::Zigzag, ExampleId::Jelly, ExampleId::Confetti][index], "View code", inner),
    }.alignment(Alignment::Leading).spacing(8.0).padding(16.0)
        .frame_width(width).background(tints[(index + model.palette) % tints.len()]).clip_radius(22.0)
        .animation(Animation::ease_in_out(Duration::from_millis(450)), model.palette)
}

fn control(
    label: &str,
    width: f32,
    color: Color,
    callback: impl Fn() + 'static,
) -> impl View + Clone + 'static {
    Button::new(label)
        .font_size(13.0)
        .material(GlassMaterial::button().tint(color.with_opacity(0.78)))
        .text_color(INK)
        .on_click(callback)
        .frame(width, 42.0)
}

#[derive(Clone)]
pub(crate) struct Playground {
    state: State<Model>,
}

impl Default for Playground {
    fn default() -> Self {
        Self {
            state: State::new(scarlet_ui::state::generate_state_id(), Model::default()),
        }
    }
}

impl Playground {
    pub(crate) fn page(
        &self,
        width: f32,
        code: CodeBrowser,
    ) -> impl View + Clone + 'static + use<> {
        PlayPage {
            state: self.state.clone(),
            width,
            code,
        }
    }
    pub(crate) fn wake(&self) {
        self.state
            .update(|m| m.wake_epoch = m.wake_epoch.wrapping_add(1));
    }
    pub(crate) fn boot(&self) {
        if !self.state.get().touched {
            Actions(self.state.clone()).load();
        }
    }
    pub(crate) fn smoke_step(&self, step: usize) {
        let action = Actions(self.state.clone());
        match step {
            0 | 7 => action.load(),
            1 => action.celebrate(),
            2 => action.colors(),
            3 => action.speed(),
            4 | 5 => action.pause(),
            6 => action.reset(),
            _ => {}
        }
    }
}

#[derive(Clone)]
struct PlayPage {
    state: State<Model>,
    width: f32,
    code: CodeBrowser,
}

impl View for PlayPage {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |page| Box::new(page.content()),
        ))
    }
    fn listenables(&self) -> Vec<&dyn scarlet_ui::state::Listenable> {
        vec![&self.state]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PlayPage {
    fn content(&self) -> impl View + Clone + 'static {
        let model = self.state.get();
        let padding = if self.width < 600.0 { 16.0 } else { 24.0 };
        let width = (self.width - padding * 2.0).max(0.0);
        let columns = if width >= 720.0 { 2 } else { 1 };
        let control_columns = if width >= 850.0 {
            6
        } else if width >= 450.0 {
            3
        } else {
            2
        };
        let button_width = (width - 10.0 * (control_columns - 1) as f32) / control_columns as f32;
        let labels = [
            "Load again",
            "Celebrate!",
            "Mix colors",
            if model.fast { "Slow down" } else { "Speed up" },
            if model.running { "Pause" } else { "Resume" },
            "Reset",
        ];
        let colors = [
            rgb(228, 214, 255),
            rgb(255, 216, 229),
            rgb(219, 243, 234),
            rgb(255, 237, 202),
            rgb(234, 230, 241),
            rgb(234, 230, 241),
        ];
        let buttons: Vec<Box<dyn View>> = labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let action = Actions(self.state.clone());
                Box::new(control(label, button_width, colors[i], move || match i {
                    0 => action.load(),
                    1 => action.celebrate(),
                    2 => action.colors(),
                    3 => action.speed(),
                    4 => action.pause(),
                    _ => action.reset(),
                })) as Box<dyn View>
            })
            .collect();
        let control_rows = buttons
            .chunks(control_columns)
            .map(|row| Box::new(HStack::new(Views(row.to_vec())).spacing(10.0)) as Box<dyn View>)
            .collect();
        let card_width = (width - 16.0 * (columns - 1) as f32) / columns as f32;
        let cards: Vec<Box<dyn View>> = [
            (
                Scene::Orbit,
                "DONUT CLUB",
                "Round and round. A happy passenger.",
            ),
            (Scene::Zigzag, "ZIGZAG EXPRESS", "Take the scenic route."),
            (
                Scene::Jelly,
                "JELLY BUDDY",
                "A bounce. A very soft landing.",
            ),
            (
                Scene::Party,
                "HOORAY MACHINE",
                "Catch a little constellation.",
            ),
        ]
        .iter()
        .enumerate()
        .map(|(i, (scene, title, caption))| {
            Box::new(
                stage_card(
                    *scene,
                    &model,
                    title,
                    caption,
                    i,
                    card_width,
                    self.code.clone(),
                )
                .key(i as u64),
            ) as Box<dyn View>
        })
        .collect();
        let card_rows = cards
            .chunks(columns)
            .map(|row| Box::new(HStack::new(Views(row.to_vec())).spacing(16.0)) as Box<dyn View>)
            .collect();
        scarlet_ui::vstack! {
            Text::new("Tiny loops. Big feelings.").font_size(if width < 450.0 { 23.0 } else { 28.0 }).color(INK),
            Text::new("Load it. Mix it. Make a little celebration.").font_size(13.0).color(MUTED),
            VStack::new(Views(control_rows)).spacing(10.0).alignment(Alignment::Leading),
            VStack::new(Views(card_rows)).spacing(16.0).alignment(Alignment::Leading),
            Text::new(if model.running { "Click Load again to start the story." } else { "Taking a breath. Resume when you're ready." })
                .font_size(12.0).color(MUTED),
        }.alignment(Alignment::Leading).spacing(16.0).padding(padding).frame_width(self.width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusing_a_stage_after_responsive_reflow_updates_artwork_and_period() {
        let mut stage = StageRenderer::new(&PlaygroundStage {
            scene: Scene::Jelly,
            model: Model::default(),
        });
        stage.update(&PlaygroundStage {
            scene: Scene::Zigzag,
            model: Model::default(),
        });
        assert!(stage.scene == Scene::Zigzag);
        stage.advance_visual_animation(Duration::from_millis(1550));
        assert!(
            (stage.clock.value() - 0.5).abs() < 0.0001,
            "Zigzag must use its own 3100ms clock after reuse"
        );
    }
}
