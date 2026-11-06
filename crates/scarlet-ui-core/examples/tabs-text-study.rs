//! Native render-command study for tabs and semantic text at 1x and 2x.
//! cargo run -p scarlet-ui-core --example tabs-text-study -- /tmp/tabs-text-study
use scarlet_ui_core::{
    element::{Element, LayoutConstraints},
    event::{Event, MouseButton, MouseEvent, Phase},
    prelude::*,
    renderer::{CpuPaintRenderer, PaintContext},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

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

fn deliver(element: &mut dyn Element, event: &Event) {
    element.handle_event(event, Phase::Target);
    for child in element.children_mut() {
        deliver(child.as_mut(), event);
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
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/tabs-text-study".into());
    fs::create_dir_all(&output)?;
    let palette = ColorPalette::default();
    let mut specimens: Vec<(Box<dyn Element>, Point)> = Vec::new();
    for (row, title) in ["Selected", "Hover", "Held", "Bottom placement"]
        .iter()
        .enumerate()
    {
        let selected = State::new(scarlet_ui_core::generate_state_id(), 0usize);
        let tab = TabView::with_selected_index(
            vec![
                TabItem::new("Overview", Spacer::new),
                TabItem::new("Activity", Spacer::new),
                TabItem::new("Settings", Spacer::new),
            ],
            selected,
        )
        .tab_bar_placement(if row == 3 {
            TabBarPlacement::Bottom
        } else {
            TabBarPlacement::Top
        });
        let mut element = tab.create_element();
        element.layout(LayoutConstraints::tight(620.0, 64.0));
        if row == 1 {
            deliver(
                element.as_mut(),
                &Event::Mouse(MouseEvent::Moved { x: 130, y: 18 }),
            );
        } else if row == 2 {
            deliver(
                element.as_mut(),
                &Event::Mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    x: 20,
                    y: 18,
                    click_count: 1,
                }),
            );
        }
        let y = 76.0 + row as f32 * 100.0;
        let mut label = Text::new(*title)
            .text_style(TextStyle::Caption)
            .tone(TextTone::Secondary)
            .create_element();
        label.layout(LayoutConstraints::unconstrained());
        specimens.push((label, Point::new(30.0, y - 24.0)));
        specimens.push((element, Point::new(30.0, y)));
    }
    for (index, (label, role, tone)) in [
        ("Clear hierarchy", TextStyle::Title, TextTone::Primary),
        (
            "A quiet section heading",
            TextStyle::Headline,
            TextTone::Primary,
        ),
        (
            "Body text keeps the familiar font and rendering.",
            TextStyle::Body,
            TextTone::Primary,
        ),
        (
            "Supporting details use a secondary foreground.",
            TextStyle::Callout,
            TextTone::Secondary,
        ),
        (
            "Caption / metadata",
            TextStyle::Caption,
            TextTone::Secondary,
        ),
    ]
    .iter()
    .enumerate()
    {
        let mut element = Text::new(*label)
            .text_style(*role)
            .tone(*tone)
            .create_element();
        element.layout(LayoutConstraints::unconstrained());
        specimens.push((element, Point::new(30.0, 500.0 + index as f32 * 44.0)));
    }
    let mut paint = PaintContext::new();
    for (element, origin) in &specimens {
        paint_tree(element.as_ref(), &mut paint, *origin);
    }
    for scale in [1000, 2000] {
        let mut renderer =
            CpuPaintRenderer::new(Size::new(680.0, 740.0), scale, palette.background());
        renderer.execute(&paint);
        write_ppm(
            &Path::new(&output).join(format!("tabs-text-{}x.ppm", scale / 1000)),
            renderer.buffer(),
        )?;
    }
    Ok(())
}
