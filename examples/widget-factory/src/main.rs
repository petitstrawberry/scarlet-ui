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
use std::time::Instant;

#[derive(Clone)]
struct WidgetFactory {
    slider_value: State<f32>,
    toggle_on: State<bool>,
    text_value: State<String>,
    selected: State<usize>,
    text_document: State<TextDocument>,
    text_selection: State<TextSelection>,
    cube_canvas: SgfxCanvasHandle,
    cube_frame: State<Arc<SgfxCanvasFrame>>,
    cube_mesh: Arc<SgfxMesh>,
    cube_started_at: Instant,
    cube_revision: u64,
}

impl WidgetFactory {
    fn new() -> Self {
        let cube_mesh = cube_mesh();
        Self {
            slider_value: State::new(StateId::new(20), 0.42),
            toggle_on: State::new(StateId::new(21), true),
            text_value: State::new(StateId::new(22), String::from("Factory text field")),
            selected: State::new(StateId::new(23), 1usize),
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

    fn scroll_page<V: View + Clone>(
        &self,
        content: V,
        content_height: f32,
    ) -> impl View + Clone + use<V> {
        ScrollView::new(content)
            .vertical()
            .content_size(0.0, content_height)
    }

    fn row<V: View + Clone>(&self, name: &str, control: V) -> impl View + Clone + use<V> {
        hstack! {
            Text::new(name.to_owned())
                .font_size(14.0)
                .frame_width(150.0),
            control,
        }
        .spacing(18.0)
    }

    fn button(&self) -> impl View + Clone + use<> {
        Button::new("Factory Button")
    }

    fn text_field(&self) -> impl View + Clone + use<> {
        TextField::new(self.text_value.clone()).placeholder("Enter value")
    }

    fn slider(&self) -> impl View + Clone + use<> {
        Slider::new(self.slider_value.clone()).min(0.0).max(1.0)
    }

    fn toggle(&self) -> impl View + Clone + use<> {
        Toggle::new(self.toggle_on.clone())
    }

    fn progress(&self) -> impl View + Clone + use<> {
        ProgressView::new(0.68)
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
    }

    fn rectangle(&self) -> impl View + Clone + use<> {
        Rectangle::new()
            .fill(Color::rgb(235u8, 242u8, 255u8))
            .corner_radius(8.0)
            .border(1.0, Color::rgb(105u8, 135u8, 210u8))
            .frame(220.0, 28.0)
    }

    fn divider(&self) -> impl View + Clone + use<> {
        Divider::new().frame(220.0, 1.0)
    }

    fn sgfx_cube(&self) -> impl View + Clone + use<> {
        SgfxCanvas::from_state(self.cube_canvas, 320.0, 220.0, self.cube_frame.clone())
    }

    fn text_view(&self) -> impl View + Clone + use<> {
        TextView::with_document(self.text_document.clone(), self.text_selection.clone())
            .placeholder("Type something...")
            .font_size(14.0)
            .padding(8.0)
            .frame_height(200.0)
    }

    fn scroll_view(&self) -> impl View + Clone + use<> {
        ScrollView::new(
            vstack! {
                Text::new("ScrollView content").font_size(16.0),
                self.row("Track 01", ProgressView::new(0.20).frame(260.0, 18.0)),
                self.row("Track 02", ProgressView::new(0.42).frame(260.0, 18.0)),
                self.row("Track 03", ProgressView::new(0.64).frame(260.0, 18.0)),
                self.row("Track 04", ProgressView::new(0.82).frame(260.0, 18.0)),
                self.row("Track 05", ProgressView::new(0.34).frame(260.0, 18.0)),
                self.row("Track 06", ProgressView::new(0.56).frame(260.0, 18.0)),
                self.row("Track 07", ProgressView::new(0.74).frame(260.0, 18.0)),
                self.row("Track 08", ProgressView::new(0.12).frame(260.0, 18.0)),
            }
            .spacing(8.0)
            .padding(12.0),
        )
        .both_axes()
        .content_size(520.0, 360.0)
        .scrollbar_visibility(ScrollbarVisibility::Always)
        .frame(320.0, 160.0)
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
        .frame(240.0, 64.0)
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
        .frame(160.0, 96.0)
    }

    fn split_view(&self) -> impl View + Clone + use<> {
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
        .min_first(72.0)
        .min_second(120.0)
        .axis_policy(SplitAxisPolicy::AdaptiveStack)
        .adaptive_stack_narrow_width(420.0)
        .frame(320.0, 160.0)
    }

    fn tab_view(&self) -> impl View + Clone + use<> {
        TabView::new(vec![
            TabItem::new("Mixer", || {
                hstack! {
                    Text::new("Ch 1").frame(72.0, 42.0),
                    Text::new("Ch 2").frame(72.0, 42.0),
                    Text::new("Master").frame(96.0, 42.0),
                }
                .spacing(8.0)
                .padding(12.0)
            }),
            TabItem::new("Editor", || {
                vstack! {
                    Text::new("Region Inspector").font_size(13.0),
                    ProgressView::new(0.52).frame(220.0, 18.0),
                }
                .spacing(10.0)
                .padding(12.0)
            }),
        ])
        .tab_bar_placement(TabBarPlacement::Automatic)
        .frame(320.0, 150.0)
    }

    fn overview_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Widget Factory").font_size(28.0),
            Text::new("PaintCommand default rendering").font_size(15.0),
            self.row("ProgressView", self.progress()),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
            self.row("ScrollView both", self.scroll_view()),
            self.row("SplitView", self.split_view()),
            self.row("TabView", self.tab_view()),
        }
        .spacing(16.0)
        .padding(24.0);
        self.scroll_page(content, 760.0)
    }

    fn sgfx_page(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("SGFX").font_size(28.0),
            Text::new("SGFX retained canvas").font_size(15.0),
            self.sgfx_cube(),
        }
        .spacing(16.0)
        .padding(24.0)
    }

    fn controls_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Controls").font_size(24.0),
            self.row("Button", self.button()),
            self.row("Toggle", self.toggle()),
            self.row("Slider", self.slider()),
            self.row("ProgressView", self.progress()),
        }
        .spacing(16.0)
        .padding(24.0);
        self.scroll_page(content, 360.0)
    }

    fn inputs_page(&self) -> impl View + Clone + use<> {
        let content = vstack! {
            Text::new("Inputs").font_size(24.0),
            self.row("TextField", self.text_field()),
            self.row("Select", self.select()),
            self.row("Slider", self.slider()),
            Text::new("TextView (multi-line editor)").font_size(14.0),
            self.text_view(),
        }
        .spacing(16.0)
        .padding(24.0);
        self.scroll_page(content, 560.0)
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
            Text::new("Display").font_size(24.0),
            self.row("Text", Text::new("Factory text sample").font_size(16.0)),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
            self.row("Tabler Icons", self.icons()),
            self.row("ScrollView both", self.scroll_view()),
            self.row("ScrollView x", self.horizontal_scroll_view()),
            self.row("ScrollView y", self.vertical_scroll_view()),
            self.row("SplitView", self.split_view()),
            self.row("TabView", self.tab_view()),
        }
        .spacing(16.0)
        .padding(24.0);
        self.scroll_page(content, 940.0)
    }
}

impl View for WidgetFactory {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new(self.clone()))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![
            &self.slider_value,
            &self.toggle_on,
            &self.text_value,
            &self.selected,
            &self.text_document,
            &self.text_selection,
        ]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Application for WidgetFactory {
    fn scenes(&self) -> impl Scene {
        let overview = self.clone();
        let sgfx = self.clone();
        let controls = self.clone();
        let inputs = self.clone();
        let display = self.clone();

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
                .shows_icons(true)
                .sidebar_width(190.0),
            )
            .size(Size::new(860.0, 560.0)),
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

fn main() -> scarlet_ui::Result<()> {
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
