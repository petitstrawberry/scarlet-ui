//! Real control paint diagnostics for focus and button visibility.
//! Not native-window screenshots. Uses actual elements and target input events.
//! cargo run -p scarlet-ui-core --example focus-button-study -- /tmp/focus-button-study
use scarlet_ui_core::{
    element::{Element, LayoutConstraints},
    event::{Event, FocusEvent, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use std::{fs, io::Write, path::PathBuf};

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

fn posture(element: &mut dyn Element, column: usize) {
    let mut send = |event| {
        element.handle_event(&event, Phase::Target);
    };
    if column == 1 || column == 2 {
        send(Event::Mouse(MouseEvent::Entered { x: 8, y: 8 }));
        send(Event::Mouse(MouseEvent::Moved { x: 8, y: 8 }));
    }
    if column == 2 {
        send(Event::Mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: 8,
            y: 8,
            click_count: 1,
        }));
    }
    if column >= 3 {
        send(Event::Focus(FocusEvent::Gained));
    }
    if column == 4 {
        send(Event::Focus(FocusEvent::Lost));
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/focus-button-study".into()),
    );
    fs::create_dir_all(&output)?;
    scarlet_ui_core::graphics::set_default_font(include_bytes!(
        "../../../examples/web-counter/assets/DejaVuSans.ttf"
    ))
    .unwrap();
    let mut elements = Vec::new();
    for group in 0..3 {
        let palette = if group == 2 {
            ColorPalette::dark()
        } else {
            ColorPalette::light()
        };
        for row in 0..2 {
            for column in 0..5 {
                let mut element = if row == 0 {
                    let mut button = Button::new("Continue").text_color(palette.text_primary());
                    if group == 2 {
                        button = button.material(GlassMaterial::new(
                            18.0,
                            palette
                                .button_background()
                                .lighten(0.035)
                                .with_opacity(0.78),
                        ));
                    }
                    button.create_element()
                } else {
                    let mut field = TextField::new(State::new(
                        scarlet_ui_core::generate_state_id(),
                        String::from("Project Atlas"),
                    ))
                    .text_color(palette.text_primary());
                    if group == 2 {
                        field = field.material(GlassMaterial::new(
                            18.0,
                            palette.button_background().with_opacity(0.78),
                        ));
                    }
                    field.create_element()
                };
                element.layout(LayoutConstraints::tight(154.0, 36.0));
                posture(element.as_mut(), column);
                elements.push((
                    element,
                    Point::new(
                        146.0 + 174.0 * column as f32,
                        151.0 + group as f32 * 200.0 + row as f32 * 61.0,
                    ),
                ));
            }
        }
    }
    for scale in [1000, 2000] {
        scarlet_ui_core::graphics::set_current_scale_milli(scale);
        let mut ctx = PaintContext::new();
        let light = ColorPalette::light();
        ctx.draw_text(
            Point::new(24.0, 20.0),
            "Focus and button visibility / actual control paint",
            light.text_primary(),
            23.0,
        );
        ctx.draw_text(Point::new(24.0, 58.0), "CPU render-command diagnostics, not native screenshots. Light defaults and explicit dark materials.", light.text_secondary(), 12.0);
        for (i, title) in [
            "Rest",
            "Pointer hover",
            "Pointer held",
            "Keyboard focus",
            "Focus lost",
        ]
        .into_iter()
        .enumerate()
        {
            ctx.draw_text(
                Point::new(146.0 + i as f32 * 174.0, 97.0),
                title,
                light.text_secondary(),
                12.0,
            );
        }
        for group in 0..3 {
            let palette = if group == 2 {
                ColorPalette::dark()
            } else {
                light.clone()
            };
            let y = 124.0 + group as f32 * 200.0;
            ctx.fill_rounded_rect(
                Rect::from_xywh(16.0, y, 1000.0, 146.0),
                14.0,
                palette.background(),
            );
            if group == 1 {
                ctx.push_clip(Rect::from_xywh(16.0, y, 1000.0, 146.0));
                for i in 0..32 {
                    let colors = [
                        Color::rgb(188u8, 215u8, 227u8),
                        Color::rgb(225u8, 207u8, 230u8),
                        Color::rgb(205u8, 224u8, 205u8),
                    ];
                    ctx.fill_rect(
                        Rect::from_xywh(130.0 + i as f32 * 30.0, y, 30.0, 146.0),
                        colors[i % 3],
                    );
                }
                ctx.pop_clip();
            }
            ctx.draw_text(
                Point::new(30.0, y + 28.0),
                "Button",
                palette.text_primary(),
                12.0,
            );
            ctx.draw_text(
                Point::new(30.0, y + 90.0),
                "TextField",
                palette.text_primary(),
                12.0,
            );
            let caption = [
                "White canvas",
                "Colored backdrop to verify blur and translucency",
                "Explicit dark glass and foreground colors (not a global theme switch)",
            ][group];
            ctx.draw_text(
                Point::new(24.0, y + 159.0),
                caption,
                light.text_secondary(),
                12.0,
            );
        }
        for (element, origin) in &elements {
            paint_tree(element.as_ref(), &mut ctx, *origin);
        }
        let mut renderer = CpuPaintRenderer::new(
            Size::new(1032.0, 732.0),
            scale,
            Color::rgb(248u8, 249u8, 251u8),
        );
        renderer.execute(&ctx);
        let buffer = renderer.buffer();
        let path = output.join(format!("focus-button-{}x.ppm", scale / 1000));
        let mut file = std::io::BufWriter::new(fs::File::create(&path)?);
        write!(file, "P6\n{} {}\n255\n", buffer.width(), buffer.height())?;
        for pixel in buffer.as_slice() {
            file.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])?;
        }
        file.flush()?;
        println!("{}", path.display());
    }
    Ok(())
}
