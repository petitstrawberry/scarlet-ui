//! Responsive card/layout transitions; no timer or application-level idle loop.

use crate::code::CodeBrowser;
use crate::source::ExampleId;
use core::time::Duration;
use scarlet_ui::{
    element::{ComponentElement, Element},
    prelude::*,
    state::{Listenable, generate_state_id},
    views::containers::ViewTuple,
};
use scarlet_ui_macros::View;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Layout {
    #[default]
    Board,
    Rows,
    Spotlight,
}

#[derive(Clone, Default)]
struct Model {
    layout: Layout,
    selected: usize,
    order: usize,
    revision: u32,
    dark: bool,
}

impl Model {
    fn key(&self) -> (Layout, usize, usize, u32, bool) {
        (
            self.layout,
            self.selected,
            self.order,
            self.revision,
            self.dark,
        )
    }
}

#[derive(Clone)]
struct Actions(State<Model>);
impl Actions {
    fn layout(&self, layout: Layout) {
        self.0.update(|s| s.layout = layout);
    }
    fn focus(&self, id: usize) {
        self.0.update(|s| {
            s.layout = if s.layout == Layout::Spotlight && s.selected == id {
                Layout::Board
            } else {
                Layout::Spotlight
            };
            s.selected = id;
        });
    }
    fn shuffle(&self) {
        self.0.update(|s| {
            s.order = (s.order + 1) % 3;
            if s.layout == Layout::Spotlight {
                s.selected = (s.selected + 1) % 3;
            }
        });
    }
    fn refresh(&self) {
        self.0.update(|s| s.revision = s.revision.wrapping_add(1));
    }
    fn theme(&self) {
        // Include the full-window background in the same transition as the cards.
        with_animation(Animation::spring(Duration::from_millis(850), 0.82), || {
            self.0.update(|s| s.dark = !s.dark);
        });
    }
    fn reset(&self) {
        without_animation(|| self.0.set(Model::default()));
    }
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: Color,
    card: Color,
    ink: Color,
    muted: Color,
    line: Color,
    well: Color,
    control: Color,
    selected: Color,
    selected_ink: Color,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: rgb(17, 23, 34),
                card: rgb(29, 39, 54),
                ink: rgb(239, 242, 247),
                muted: rgb(153, 166, 184),
                line: rgb(54, 68, 87),
                well: rgb(39, 51, 69),
                control: rgb(51, 65, 85),
                selected: rgb(183, 212, 255),
                selected_ink: rgb(17, 35, 59),
            }
        } else {
            Self {
                canvas: rgb(246, 244, 239),
                card: Color::WHITE,
                ink: rgb(32, 43, 59),
                muted: rgb(112, 119, 130),
                line: rgb(225, 225, 222),
                well: rgb(232, 232, 228),
                control: rgb(222, 230, 240),
                selected: rgb(43, 62, 88),
                selected_ink: Color::WHITE,
            }
        }
    }
}
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}
fn accent(id: usize) -> Color {
    [rgb(229, 112, 69), rgb(45, 152, 136), rgb(105, 116, 220)][id]
}

#[derive(Clone)]
struct Views(Vec<Box<dyn View>>);
impl ViewTuple for Views {
    fn create_elements(&self) -> Vec<Box<dyn Element>> {
        self.0.iter().map(|v| v.create_element()).collect()
    }
    fn clone_views(&self) -> Vec<Box<dyn View>> {
        self.0.clone()
    }
    fn collect_listenables<'a>(&'a self, collector: &mut Vec<&'a dyn Listenable>) {
        for view in &self.0 {
            collector.extend(view.listenables());
        }
    }
}

#[derive(Clone)]
pub(crate) struct LayoutLab {
    state: State<Model>,
    width: f32,
    code: CodeBrowser,
}

impl Default for LayoutLab {
    fn default() -> Self {
        Self {
            state: State::new(generate_state_id(), Model::default()),
            width: 960.0,
            code: CodeBrowser::default(),
        }
    }
}

impl LayoutLab {
    pub(crate) fn background_color(&self) -> Color {
        Palette::new(self.state.get().dark).canvas
    }
    /// Full viewport width including 16/24 px padding. The shell supplies a
    /// ScrollView with natural-height measurement; this component owns State.
    pub(crate) fn page(&self, width: f32, code: CodeBrowser) -> Self {
        Self {
            width: if width.is_finite() {
                width.max(0.0)
            } else {
                self.width
            },
            code,
            ..self.clone()
        }
    }

    pub(crate) const SMOKE_STEPS: usize = 9;

    pub(crate) fn smoke_step(&self, step: usize) {
        let actions = Actions(self.state.clone());
        match step {
            0 => actions.layout(Layout::Rows),
            1 => actions.shuffle(),
            2 => actions.refresh(),
            3 => actions.focus(0),
            4 => actions.focus(1),
            5 => actions.theme(),
            6 => actions.layout(Layout::Board),
            7 => actions.refresh(),
            8 => actions.reset(),
            _ => {}
        }
    }

    fn content(&self) -> impl View + Clone + 'static {
        let model = self.state.get();
        let p = Palette::new(model.dark);
        let padding = if self.width < 600.0 { 16.0 } else { 24.0 };
        let width = (self.width - padding * 2.0).max(0.0);
        let actions = Actions(self.state.clone());
        let mut height = 0.0f32;
        let cards = (0..3)
            .map(|id| {
                let pose = card_pose(&model, id, width);
                height = height.max(pose.y + pose.height);
                // Reorder through offsets, never through child order or new keys.
                Box::new(
                    MetricCard {
                        id,
                        model: model.clone(),
                        actions: actions.clone(),
                        width,
                    }
                    .key(id as u64),
                ) as Box<dyn View>
            })
            .collect();
        let caption = match model.layout {
            Layout::Board => "A little room for everything.",
            Layout::Rows => "Same cards. A different rhythm.",
            Layout::Spotlight => "Make space for what matters.",
        };
        scarlet_ui::vstack! {
            Text::new("Motion desk").font_size(28.0).color(p.ink),
            Text::new("A workspace that moves with you.").font_size(14.0).color(p.muted),
            layout_picker(&model, &actions, p, width.min(360.0)),
            controls(&model, &actions, p, width),
            code_links(&self.code, width),
            Text::new(caption).font_size(14.0).color(p.ink),
            Text::new(format!("SAMPLE SET {:02}", model.revision % 100 + 1)).font_size(11.0).color(p.muted),
            ZStack::new(Views(cards)).alignment(Alignment::TopLeading).frame(width, height),
            Text::new("Switch layouts before cards settle.\nFocus, shuffle or refresh the charts.").font_size(12.0).color(p.muted),
        }.alignment(Alignment::Leading).spacing(16.0).padding(padding).frame_width(self.width)
            .background(p.canvas).animation(Animation::spring(Duration::from_millis(850), 0.82), model.key())
    }
}

impl View for LayoutLab {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), |view| {
            Box::new(view.content())
        }))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.state]
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

struct CardPose {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    row: bool,
    small: bool,
}
fn card_pose(model: &Model, id: usize, width: f32) -> CardPose {
    let slot = (id + model.order) % 3;
    let wide = width >= 840.0;
    let third = ((width - 32.0) / 3.0).max(0.0);
    match model.layout {
        Layout::Board => CardPose {
            x: if wide {
                slot as f32 * (third + 16.0)
            } else {
                0.0
            },
            y: if wide { 0.0 } else { slot as f32 * 376.0 },
            width: if wide { third } else { width },
            height: 360.0,
            row: false,
            small: false,
        },
        Layout::Rows => {
            let row = width >= 720.0;
            let height = if row { 116.0 } else { 208.0 };
            CardPose {
                x: 0.0,
                y: slot as f32 * (height + 16.0),
                width,
                height,
                row,
                small: !row,
            }
        }
        Layout::Spotlight if model.selected == id => CardPose {
            x: 0.0,
            y: 0.0,
            width: if wide { width - third - 16.0 } else { width },
            height: 360.0,
            row: false,
            small: false,
        },
        Layout::Spotlight => {
            let peer = ((id + 3 - model.selected - 1) % 3) as f32;
            CardPose {
                x: if wide { width - third } else { 0.0 },
                y: if wide {
                    peer * 188.0
                } else {
                    376.0 + peer * 224.0
                },
                width: if wide { third } else { width },
                height: if wide { 172.0 } else { 208.0 },
                row: false,
                small: true,
            }
        }
    }
}

#[derive(View, Clone)]
#[view(body = content)]
struct MetricCard {
    id: usize,
    model: Model,
    actions: Actions,
    width: f32,
}
impl MetricCard {
    fn content(&self) -> impl View + Clone + 'static {
        let p = Palette::new(self.model.dark);
        let pose = card_pose(&self.model, self.id, self.width);
        let selected = self.model.layout == Layout::Spotlight && self.model.selected == self.id;
        let value_x = if pose.row { pose.width * 0.30 } else { 16.0 };
        let value_y = if pose.row { 18.0 } else { 72.0 };
        let chart_x = if pose.row {
            pose.width * 0.46
        } else if pose.small {
            pose.width * 0.47
        } else {
            16.0
        };
        let chart_y = if pose.row {
            24.0
        } else if pose.small {
            78.0
        } else {
            164.0
        };
        let chart_width = if pose.row {
            pose.width * 0.27
        } else {
            (pose.width - chart_x - 16.0).max(0.0)
        };
        let chart_height = if pose.row {
            68.0
        } else if pose.small {
            pose.height - 132.0
        } else {
            96.0
        };
        let button_width = if pose.row {
            100.0
        } else {
            (pose.width - 32.0).max(0.0)
        };
        let button_height = if pose.small { 36.0 } else { 40.0 };
        let actions = self.actions.clone();
        let id = self.id;
        let surface = if selected {
            if self.model.dark {
                rgb(47, 48, 57)
            } else {
                rgb(255, 250, 240)
            }
        } else {
            p.card
        };
        ZStack::new((
            ZStack::new((Text::new(["A", "T", "O"][id])
                .font_size(18.0)
                .color(Color::WHITE),))
            .frame(32.0, 32.0)
            .background(accent(id))
            .clip_radius(10.0)
            .offset(16.0, 16.0),
            Text::new(["Aurora", "Tidal", "Orbit"][id])
                .font_size(19.0)
                .color(p.ink)
                .offset(60.0, 15.0),
            Text::new(["Creative space", "Team momentum", "Ideas in motion"][id])
                .font_size(12.0)
                .color(p.muted)
                .offset(60.0, 40.0),
            Text::new(["84", "128", "36"][id])
                .font_size(34.0)
                .color(p.ink)
                .offset(value_x, value_y),
            Text::new("this week")
                .font_size(11.0)
                .color(p.muted)
                .offset(value_x, value_y + 44.0),
            MiniChart {
                id,
                revision: self.model.revision,
                width: chart_width,
                height: chart_height,
                dark: self.model.dark,
            }
            .offset(chart_x, chart_y)
            .opacity(if pose.small { 0.70 } else { 1.0 }),
            Text::new(
                [
                    "Ideas explored this week",
                    "Small steps, moving together",
                    "Possibilities taking shape",
                ][id],
            )
            .font_size(12.0)
            .color(p.muted)
            .offset(16.0, 276.0)
            .opacity(if pose.row || pose.small { 0.0 } else { 1.0 }),
            glass_button(
                if selected { "Back to board" } else { "Focus" },
                p,
                selected,
            )
            .on_click(move || actions.focus(id))
            .frame(button_width, button_height)
            .offset(
                if pose.row { pose.width - 116.0 } else { 16.0 },
                if pose.row {
                    (pose.height - button_height) * 0.5
                } else {
                    pose.height - button_height - 14.0
                },
            ),
        ))
        .alignment(Alignment::TopLeading)
        .frame(pose.width, pose.height)
        .background(surface)
        .clip_radius(18.0)
        .border_rounded(if selected { p.selected } else { p.line }, 1.0, 18.0)
        .offset(pose.x, pose.y)
    }
}

#[derive(View, Clone)]
#[view(body = content)]
struct MiniChart {
    id: usize,
    revision: u32,
    width: f32,
    height: f32,
    dark: bool,
}
impl MiniChart {
    fn content(&self) -> impl View + Clone + 'static {
        let gap = (self.width / 40.0).clamp(2.0, 7.0);
        let width = ((self.width - gap * 7.0) / 8.0).max(0.0);
        let bars = (0..8)
            .map(|i| {
                let seed = (self.id as u32 * 19
                    + i as u32 * 23
                    + (self.revision % 97) * (11 + i as u32 * 3))
                    % 79;
                let height = self.height * (0.22 + seed as f32 / 100.0);
                Box::new(
                    Spacer::new()
                        .frame(width, height)
                        .background(accent(self.id))
                        .clip_radius(4.0)
                        .offset(i as f32 * (width + gap), self.height - height)
                        .opacity(0.48 + i as f32 * 0.07)
                        // Data refresh eases each bar; unchanged data inherits the layout spring.
                        .animation(
                            Animation::ease_in_out(Duration::from_millis(560 + i as u64 * 70)),
                            self.revision,
                        )
                        .key(i as u64),
                ) as Box<dyn View>
            })
            .collect();
        ZStack::new((
            Spacer::new()
                .frame(self.width, 1.0)
                .background(Palette::new(self.dark).line)
                .offset(0.0, self.height),
            ZStack::new(Views(bars))
                .alignment(Alignment::TopLeading)
                .frame(self.width, self.height),
        ))
        .alignment(Alignment::TopLeading)
        .frame(self.width, self.height)
    }
}

fn glass_button(label: &str, p: Palette, selected: bool) -> Button {
    // Tint the shared material without replacing its optics or fading labels.
    Button::new(label)
        .font_size(13.0)
        .padding(6.0)
        .material(
            GlassMaterial::button().tint(
                (if selected { p.selected } else { p.control }).with_opacity(0.78),
            ),
        )
        .text_color(if selected { p.selected_ink } else { p.ink })
}

fn layout_picker(
    model: &Model,
    actions: &Actions,
    p: Palette,
    width: f32,
) -> impl View + Clone + 'static + use<> {
    let slot_width = width / 3.0;
    let selected = match model.layout {
        Layout::Board => 0,
        Layout::Rows => 1,
        Layout::Spotlight => 2,
    };
    let buttons = [Layout::Board, Layout::Rows, Layout::Spotlight]
        .into_iter()
        .enumerate()
        .map(|(index, layout)| {
            let actions = actions.clone();
            Box::new(
                Button::new(["Board", "Rows", "Spotlight"][index])
                    .header_style()
                    .font_size(13.0)
                    .background_color(Color::CLEAR)
                    .border_color(Color::CLEAR)
                    .text_color(if index == selected {
                        p.selected_ink
                    } else {
                        p.ink
                    })
                    .on_click(move || actions.layout(layout))
                    .frame(slot_width, 42.0),
            ) as Box<dyn View>
        })
        .collect();
    ZStack::new((
        Spacer::new()
            .frame((slot_width - 6.0).max(0.0), 34.0)
            .background(p.selected)
            .clip_radius(9.0)
            .offset(3.0 + selected as f32 * slot_width, 4.0),
        HStack::new(Views(buttons)).spacing(0.0),
    ))
    .alignment(Alignment::TopLeading)
    .frame(width, 42.0)
    .background(p.well)
    .clip_radius(12.0)
}

fn controls(
    model: &Model,
    actions: &Actions,
    p: Palette,
    width: f32,
) -> impl View + Clone + 'static + use<> {
    let columns = if width >= 600.0 {
        4
    } else if width >= 220.0 {
        2
    } else {
        1
    };
    let cell = ((width - (columns - 1) as f32 * 10.0) / columns as f32).max(0.0);
    let labels = [
        "Shuffle",
        "Refresh",
        if model.dark {
            "Light theme"
        } else {
            "Dark theme"
        },
        "Reset",
    ];
    let buttons = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let actions = actions.clone();
            Box::new(
                glass_button(label, p, false)
                    .on_click(move || match i {
                        0 => actions.shuffle(),
                        1 => actions.refresh(),
                        2 => actions.theme(),
                        _ => actions.reset(),
                    })
                    .frame(cell, 42.0)
                    .offset(
                        (i % columns) as f32 * (cell + 10.0),
                        (i / columns) as f32 * 52.0,
                    )
                    .key(i as u64),
            ) as Box<dyn View>
        })
        .collect();
    ZStack::new(Views(buttons))
        .alignment(Alignment::TopLeading)
        .frame(width, (4 / columns) as f32 * 52.0 - 10.0)
}

fn code_links(code: &CodeBrowser, width: f32) -> impl View + Clone + 'static + use<> {
    let columns = if width >= 600.0 { 3 } else { 1 };
    let cell = ((width - (columns - 1) as f32 * 10.0) / columns as f32).max(0.0);
    let links = [
        (ExampleId::Layout, "Layout code"),
        (ExampleId::Charts, "Chart code"),
        (ExampleId::Theme, "Theme code"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (id, label))| {
        Box::new(
            code.button(id, label, cell)
                .frame(cell, 42.0)
                .offset(
                    (i % columns) as f32 * (cell + 10.0),
                    (i / columns) as f32 * 52.0,
                )
                .key(i as u64),
        ) as Box<dyn View>
    })
    .collect();
    ZStack::new(Views(links))
        .alignment(Alignment::TopLeading)
        .frame(width, (3 / columns) as f32 * 52.0 - 10.0)
}
