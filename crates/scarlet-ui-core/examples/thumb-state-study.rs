//! CPU render-command diagnostics using the real Toggle/Slider render objects.
//! These images are not native window screenshots or event-routing verification.
//! cargo run -p scarlet-ui-core --example thumb-state-study -- /tmp/thumb-state-study
use scarlet_ui_core::{
    element::{ElementRenderObject, LayoutConstraints},
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
    views::{SliderRenderObject, ToggleRenderObject},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 490.0;

fn write_ppm(path: &Path, buffer: &scarlet_ui_core::Buffer) -> io::Result<()> {
    let mut file = io::BufWriter::new(fs::File::create(path)?);
    write!(file, "P6\n{} {}\n255\n", buffer.width(), buffer.height())?;
    for pixel in buffer.as_slice() {
        file.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])?;
    }
    file.flush()
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/thumb-state-study".into());
    let output = Path::new(&output);
    fs::create_dir_all(output)?;
    let palette = ColorPalette::default();
    let rows = [
        "Toggle / OFF",
        "Toggle / ON",
        "Slider / 50%",
        "Slider / minimum",
        "Slider / maximum",
    ];
    let mut controls: Vec<(Box<dyn ElementRenderObject>, Point)> = Vec::new();
    for row in 0..rows.len() {
        for (column, held) in [false, true].into_iter().enumerate() {
            let control: Box<dyn ElementRenderObject> = if row < 2 {
                let mut toggle = ToggleRenderObject::new(row == 1);
                toggle.set_pressed(held);
                Box::new(toggle)
            } else {
                let value = [0.5, 0.0, 1.0][row - 2];
                let mut slider = SliderRenderObject::new(value, 0.0, 1.0, false);
                slider.layout(LayoutConstraints::tight(200.0, 20.0));
                slider.set_dragging(held);
                Box::new(slider)
            };
            controls.push((
                control,
                Point::new(216.0 + column as f32 * 264.0, 142.0 + row as f32 * 58.0),
            ));
        }
    }
    let mut metadata = String::from(
        "CPU render-command diagnostics, NOT native screenshots.\n\
         Real ToggleRenderObject and SliderRenderObject paint methods.\n\
         Held states set through set_pressed / set_dragging; event routing is not exercised.\n\
         Fixed 36x20 Toggle and 200x20 Slider layout boxes; full visual outsets retained.\n\
         No diagnostic control geometry, gradients, outlines, or shadows are recreated here.\n\
         Alternating row backgrounds are diagnostic context only.\n",
    );
    for scale in [1000, 2000] {
        let mut ctx = PaintContext::new();
        ctx.draw_text(
            Point::new(28.0, 24.0),
            "CPU render-command diagnostics",
            palette.text_primary(),
            22.0,
        );
        ctx.draw_text(
            Point::new(28.0, 58.0),
            "Real Toggle / Slider paint output. Not a native window screenshot.",
            palette.text_secondary(),
            13.0,
        );
        ctx.draw_text(
            Point::new(216.0, 103.0),
            "Resting / glass",
            palette.text_primary(),
            14.0,
        );
        ctx.draw_text(
            Point::new(480.0, 103.0),
            "Held / translucent",
            palette.text_primary(),
            14.0,
        );
        for (row, label) in rows.iter().enumerate() {
            let y = 130.0 + row as f32 * 58.0;
            if row % 2 == 1 {
                ctx.fill_rect(
                    Rect::from_xywh(20.0, y, 720.0, 46.0),
                    palette.background_tertiary(),
                );
            }
            ctx.draw_text(
                Point::new(32.0, y + 14.0),
                *label,
                palette.text_secondary(),
                13.0,
            );
        }
        for (control, origin) in &controls {
            assert!(control.paint(&mut ctx, *origin));
        }
        ctx.draw_text(
            Point::new(28.0, 452.0),
            format!(
                "{}x raster | shared backdrop material | CPU command raster",
                scale / 1000
            ),
            palette.text_secondary(),
            12.0,
        );
        let mut renderer =
            CpuPaintRenderer::new(Size::new(WIDTH, HEIGHT), scale, palette.background());
        renderer.execute(&ctx);
        let path = output.join(format!("thumb-states-{}x.ppm", scale / 1000));
        write_ppm(&path, renderer.buffer())?;
        metadata.push_str(&format!(
            "{}: {}x{} physical px, {} paint commands\n",
            path.display(),
            renderer.buffer().width(),
            renderer.buffer().height(),
            ctx.commands().len()
        ));
        println!("{}", path.display());
    }
    fs::write(output.join("metadata.txt"), metadata)?;
    Ok(())
}
