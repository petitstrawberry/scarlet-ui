//! Real ScarletUI control raster study. No browser, window server, or image dependency.
//! cargo run -p scarlet-ui-core --example control-study -- /tmp/control-study
//! Optional --baseline omits unsupported states from an unmodified comparison build.
use scarlet_ui_core::{
    element::{Element, LayoutConstraints},
    event::{Event, FocusEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 860.0;

struct Specimen {
    element: Box<dyn Element>,
    origin: Point,
    size: Size,
    label: String,
}

fn state<T: Clone + 'static>(value: T) -> State<T> {
    State::new(scarlet_ui_core::generate_state_id(), value)
}

fn deliver(element: &mut dyn Element, event: Event) {
    element.handle_event(&event, Phase::Target);
}

fn posture(element: &mut dyn Element, row: usize, column: usize) {
    let at = (8, 8);
    if row == 1 || row == 2 || row == 3 {
        deliver(
            element,
            Event::Mouse(MouseEvent::Entered { x: at.0, y: at.1 }),
        );
        deliver(
            element,
            Event::Mouse(MouseEvent::Moved { x: at.0, y: at.1 }),
        );
    }
    if row == 2 || row == 3 {
        deliver(
            element,
            Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x: at.0,
                y: at.1,
                click_count: 1,
            }),
        );
        // Select opens on its real click activation; Button remains held down.
        if column == 2 {
            deliver(
                element,
                Event::Mouse(MouseEvent::ButtonReleased {
                    button: MouseButton::Left,
                    x: at.0,
                    y: at.1,
                    click_count: 1,
                }),
            );
        }
    }
    if row == 4 || row == 3 {
        deliver(element, Event::Focus(FocusEvent::Gained));
    }
}

fn paint_tree<'a>(element: &'a dyn Element, ctx: &mut PaintContext<'a>, origin: Point) {
    if let Some(render) = element.render_object() {
        render.paint(ctx, origin);
    }
    for child in element.children() {
        let p = child.position();
        paint_tree(
            child.as_ref(),
            ctx,
            Point::new(origin.x + p.x, origin.y + p.y),
        );
    }
}

fn write_ppm(path: &Path, buffer: &scarlet_ui_core::Buffer) -> io::Result<()> {
    let mut file = io::BufWriter::new(fs::File::create(path)?);
    write!(file, "P6\n{} {}\n255\n", buffer.width(), buffer.height())?;
    for pixel in buffer.as_slice() {
        file.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])?;
    }
    file.flush()
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let baseline = args.iter().any(|arg| arg == "--baseline");
    let output = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("SCARLET_CONTROL_STUDY_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("target/control-study"));
    fs::create_dir_all(&output)?;
    scarlet_ui_core::graphics::set_default_font(include_bytes!(
        "../../../examples/web-counter/assets/DejaVuSans.ttf"
    ))
    .expect("checked-in font is valid");

    let rows = [
        ("Rest", 150.0),
        ("Pointer hover", 236.0),
        ("Pressed / open", 322.0),
        ("Disabled", 506.0),
        ("Keyboard focus", 592.0),
    ];
    let columns = [
        ("Button", 190.0, 160.0),
        ("TextField", 388.0, 248.0),
        ("Select", 674.0, 278.0),
    ];
    let mut specimens = Vec::new();
    let mut metadata = String::from(
        "ScarletUI actual control elements; DejaVu Sans; CPU paint commands.\nLogical dimensions, not rescaled screenshots. CPU antialiased contours, real backdrop blur and soft drop shadows.\n",
    );
    for (row, (name, y)) in rows.iter().enumerate() {
        if baseline && row >= 3 {
            continue;
        }
        for (column, (kind, x, width)) in columns.iter().enumerate() {
            let disabled = row == 3;
            let mut element: Box<dyn Element> = match column {
                0 => Button::new("Continue").enabled(!disabled).create_element(),
                1 => TextField::new(state(String::from("Project Atlas")))
                    .placeholder("Project name")
                    .enabled(!disabled)
                    .create_element(),
                _ => Select::new(
                    vec!["Workspace".into(), "Personal".into(), "Shared team".into()],
                    state(0usize),
                )
                .width(*width)
                .enabled(!disabled)
                .create_element(),
            };
            let constraints = LayoutConstraints::new(*width, *width, 0.0, 160.0);
            element.layout(constraints);
            posture(element.as_mut(), row, column);
            let size = element.layout(constraints);
            let label = format!("{:.0} x {:.0} logical px", size.width, size.height);
            metadata.push_str(&format!("{name} / {kind}: {label}\n"));
            specimens.push(Specimen {
                element,
                origin: Point::new(*x, *y),
                size,
                label,
            });
        }
    }
    // This row uses ScarletUI's HStack allocation, not hand-positioned controls.
    let mut form = HStack::new((
        Text::new("Workspace").font_size(13.0),
        TextField::new(state(String::from("Project Atlas"))),
        Select::new(vec!["Workspace".into(), "Personal".into()], state(0usize)).width(190.0),
        Button::new("Continue"),
    ))
    .spacing(16.0)
    .alignment(Alignment::Center)
    .create_element();
    let form_size = form.layout(LayoutConstraints::loose(904.0, 60.0));
    metadata.push_str(&format!(
        "Native HStack form: {:.0} x {:.0} logical px\n",
        form_size.width, form_size.height
    ));

    for scale in [1000u32, 2000u32] {
        scarlet_ui_core::graphics::set_current_scale_milli(scale);
        let mut ctx = PaintContext::new();
        let ink = Color::rgb_f32(0.12, 0.14, 0.18);
        let muted = Color::rgb_f32(0.39, 0.42, 0.48);
        ctx.draw_text(
            Point::new(48.0, 36.0),
            if baseline {
                "ScarletUI / baseline controls"
            } else {
                "ScarletUI / coherent controls"
            },
            ink,
            26.0,
        );
        ctx.draw_text(
            Point::new(48.0, 76.0),
            "Actual Rust controls. Same font family. Natural heights; fixed comparison widths.",
            muted,
            13.0,
        );
        ctx.draw_text(
            Point::new(48.0, 98.0),
            format!(
                "CPU paint at {}x native resolution. Antialiased glass contours and soft shadows.",
                scale / 1000
            ),
            muted,
            12.0,
        );
        for (kind, x, _) in &columns {
            ctx.draw_text(Point::new(*x, 125.0), *kind, ink, 13.0);
        }
        for (row, (name, y)) in rows.iter().enumerate() {
            ctx.draw_text(Point::new(48.0, *y + 6.0), *name, ink, 13.0);
            if baseline && row >= 3 {
                ctx.draw_text(
                    Point::new(190.0, *y + 6.0),
                    "Omitted from the baseline state comparison",
                    muted,
                    12.0,
                );
            }
        }
        for specimen in &specimens {
            paint_tree(specimen.element.as_ref(), &mut ctx, specimen.origin);
            // Keep metadata away from the open select's popup.
            let y = if specimen.origin.y == 322.0 {
                474.0
            } else {
                specimen.origin.y + specimen.size.height + 12.0
            };
            ctx.draw_text(
                Point::new(specimen.origin.x, y),
                &specimen.label,
                muted,
                11.0,
            );
        }
        ctx.fill_rect(
            Rect::from_xywh(48.0, 684.0, 904.0, 1.0),
            Color::rgb_f32(0.88, 0.89, 0.91),
        );
        ctx.draw_text(
            Point::new(48.0, 704.0),
            "In context / native HStack form",
            ink,
            16.0,
        );
        paint_tree(form.as_ref(), &mut ctx, Point::new(48.0, 744.0));
        ctx.draw_text(
            Point::new(48.0, 812.0),
            "States are delivered through Element::handle_event; no CSS or HTML mock controls.",
            muted,
            11.0,
        );
        let commands = ctx.commands().len();
        let mut renderer = CpuPaintRenderer::new(Size::new(WIDTH, HEIGHT), scale, Color::WHITE);
        renderer.execute(&ctx);
        let path = output.join(format!("controls-{}x.ppm", scale / 1000));
        write_ppm(&path, renderer.buffer())?;
        metadata.push_str(&format!(
            "{}: {} x {} physical px, {} paint commands\n",
            path.display(),
            renderer.buffer().width(),
            renderer.buffer().height(),
            commands
        ));
        println!("{}", path.display());
    }
    fs::write(output.join("metadata.txt"), metadata)?;
    Ok(())
}
