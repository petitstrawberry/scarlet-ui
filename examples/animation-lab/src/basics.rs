//! Explicit and implicit motion. The shell owns scrolling and measures natural height.

use crate::{code::CodeBrowser, source::ExampleId};
use core::time::Duration;
use scarlet_ui::{
    element::{ComponentElement, Element},
    prelude::*,
    state::{Listenable, generate_state_id},
};

const INK: Color = Color::rgb_f32(0.12, 0.16, 0.24);
const MUTED: Color = Color::rgb_f32(0.38, 0.43, 0.52);
const OUTLINE: Color = Color::rgb_f32(0.79, 0.82, 0.88);
const PANEL_HEIGHT: f32 = 422.0;

#[derive(Clone)]
pub(crate) struct Basics {
    explicit: State<bool>,
    implicit: State<bool>,
    width: f32,
    code: CodeBrowser,
}

impl Default for Basics {
    fn default() -> Self {
        Self {
            explicit: State::new(generate_state_id(), false),
            implicit: State::new(generate_state_id(), false),
            width: 800.0,
            code: CodeBrowser::default(),
        }
    }
}

impl Basics {
    /// Full viewport width, including this page's 16/24 px horizontal padding.
    /// Clones share State; mounting this component subscribes to both controls.
    pub(crate) fn page(&self, width: f32, code: CodeBrowser) -> Self {
        Self {
            code,
            width: if width.is_finite() {
                width.max(0.0)
            } else {
                self.width
            },
            ..self.clone()
        }
    }

    pub(crate) const SMOKE_STEPS: usize = 7;

    /// Call once per step, allowing a rebuild between steps; 2/3 reverse 0/1.
    pub(crate) fn smoke_step(&self, step: usize) {
        match step {
            0 | 2 | 4 => self.toggle_explicit(),
            1 | 3 | 5 => self.implicit.update(|value| *value = !*value),
            6 => without_animation(|| {
                self.explicit.set(false);
                self.implicit.set(false);
            }),
            _ => {}
        }
    }

    fn toggle_explicit(&self) {
        with_animation(Animation::spring(Duration::from_millis(1400), 0.65), || {
            self.explicit.update(|value| *value = !*value);
        });
    }

    fn content(&self) -> impl View + Clone + 'static {
        let padding = if self.width < 600.0 { 16.0 } else { 24.0 };
        let width = (self.width - 2.0 * padding).max(0.0);
        let columns = if width >= 720.0 { 2 } else { 1 };
        let panel_width = (width - (columns - 1) as f32 * 16.0) / columns as f32;
        let stage_width = (panel_width - 32.0).max(0.0);
        let explicit = self.explicit.get();
        let implicit = self.implicit.get();
        let toggle_explicit = self.clone();
        let toggle_implicit = self.implicit.clone();
        let reset_explicit = self.explicit.clone();
        let reset_implicit = self.implicit.clone();
        let first = panel(
            "01  Explicit mutation",
            "Spring / 1400 ms / damping 0.65",
            explicit,
            panel_width,
            motion_stage(
                explicit,
                stage_width,
                Color::rgb(49u8, 93u8, 211u8),
                Color::rgb(170u8, 55u8, 129u8),
            ),
            move || toggle_explicit.toggle_explicit(),
            move || without_animation(|| reset_explicit.set(false)),
            self.code.clone(),
            ExampleId::Explicit,
        )
        .key(0u64);
        let second = panel(
            "02  Implicit view scope",
            "Ease in-out / 1200 ms",
            implicit,
            panel_width,
            motion_stage(
                implicit,
                stage_width,
                Color::rgb(0u8, 124u8, 117u8),
                Color::rgb(190u8, 90u8, 28u8),
            )
            .animation(
                Animation::ease_in_out(Duration::from_millis(1200)),
                implicit,
            ),
            move || toggle_implicit.update(|value| *value = !*value),
            move || without_animation(|| reset_implicit.set(false)),
            self.code.clone(),
            ExampleId::Implicit,
        )
        .offset(
            if columns == 2 {
                panel_width + 16.0
            } else {
                0.0
            },
            if columns == 2 {
                0.0
            } else {
                PANEL_HEIGHT + 16.0
            },
        )
        .key(1u64);

        scarlet_ui::vstack! {
            Text::new("The basics").font_size(28.0).color(INK),
            Text::new("Resize, recolor, move and fade.").font_size(14.0).color(MUTED),
            // The two panel identities survive the row/column breakpoint.
            ZStack::new((first, second)).alignment(Alignment::TopLeading)
                .frame(width, if columns == 2 { PANEL_HEIGHT } else { PANEL_HEIGHT * 2.0 + 16.0 }),
            Text::new("Tap again while a tile is moving.\nReset settles immediately.").font_size(13.0).color(MUTED),
        }.alignment(Alignment::Leading).spacing(16.0).padding(padding).frame_width(self.width)
    }
}

impl View for Basics {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), |view| {
            Box::new(view.content())
        }))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.explicit, &self.implicit]
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn motion_stage(
    expanded: bool,
    width: f32,
    start: Color,
    end: Color,
) -> impl View + Clone + 'static {
    let small = (width - 32.0).clamp(0.0, 80.0);
    let large = (width * 0.40).min(144.0);
    let destination = (width - large - 16.0).max(0.0);
    let tile = ZStack::new((Text::new("MOVE").font_size(14.0).color(Color::WHITE),))
        .frame(
            if expanded { large } else { small },
            if expanded { 96.0 } else { 64.0 },
        )
        .background(if expanded { end } else { start })
        .clip_radius(10.0)
        .offset(
            if expanded { destination } else { 16.0 },
            if expanded { 52.0 } else { 20.0 },
        )
        .opacity(if expanded { 0.5 } else { 1.0 });
    ZStack::new((
        Rectangle::new()
            .fill(Color::rgb(235u8, 239u8, 246u8))
            .corner_radius(10.0)
            .frame(width, 180.0),
        Rectangle::new()
            .fill(Color::CLEAR)
            .corner_radius(10.0)
            .border(1.0, OUTLINE)
            .frame(small, 64.0)
            .offset(16.0, 20.0),
        Rectangle::new()
            .fill(Color::CLEAR)
            .corner_radius(10.0)
            .border(1.0, OUTLINE)
            .frame(large, 96.0)
            .offset(destination, 52.0),
        Text::new("A")
            .font_size(12.0)
            .color(MUTED)
            .offset(16.0, 96.0),
        Text::new("B")
            .font_size(12.0)
            .color(MUTED)
            .offset(destination, 158.0),
        tile,
    ))
    .alignment(Alignment::TopLeading)
    .frame(width, 180.0)
}

fn panel(
    title: &str,
    curve: &str,
    expanded: bool,
    width: f32,
    stage: impl View + Clone + 'static,
    on_toggle: impl Fn() + 'static,
    on_reset: impl Fn() + 'static,
    code: CodeBrowser,
    example: ExampleId,
) -> impl View + Clone + 'static {
    let inner = (width - 32.0).max(0.0);
    let reset_width = ((inner - 10.0) * 0.36).max(0.0);
    scarlet_ui::vstack! {
        Text::new(title).font_size(18.0).color(INK),
        Text::new(curve).font_size(12.0).color(MUTED),
        stage,
        Text::new(if expanded { "Target B / expanded / 50%" } else { "Target A / compact / 100%" }).font_size(13.0).color(INK),
        scarlet_ui::hstack! {
            Button::new("Animate / reverse").font_size(13.0).material(GlassMaterial::button().tint(INK.with_opacity(0.78))).text_color(Color::WHITE)
                .on_click(on_toggle).frame((inner - reset_width - 10.0).max(0.0), 38.0),
            Button::new("Reset to A").font_size(13.0).material(GlassMaterial::button().tint(Color::rgb(241u8, 244u8, 249u8).with_opacity(0.78))).text_color(INK)
                .on_click(on_reset).frame(reset_width, 38.0),
        }.spacing(10.0),
        Text::new("Outlines mark both endpoints.").font_size(12.0).color(MUTED),
        code.button(example, "View code", inner),
    }.alignment(Alignment::Leading).spacing(10.0).padding(16.0).frame(width, PANEL_HEIGHT)
        .background(Color::WHITE).clip_radius(14.0).border_rounded(OUTLINE, 1.0, 14.0)
}
