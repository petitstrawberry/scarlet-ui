use super::*;
use crate::{renderer::PaintCommand, state::generate_state_id};

#[test]
fn retained_grid_paint_preserves_explicit_fonts_for_plain_and_bold_cells() {
    let font = FontStack::new(include_bytes!("../testing/fonts/metrics-primary.ttf")).unwrap();
    let mut grid = TextGridBuffer::new(2, 1, TextGridCell::new('W', Color::WHITE, Color::BLACK));
    let mut bold = grid.cell(1, 0).unwrap();
    bold.bold = true;
    grid.set_cell(1, 0, bold);
    let view = TextGrid::new(State::new(generate_state_id(), grid.clone()))
        .cell_size(10.0, 20.0)
        .font_size(14.0)
        .cursor_color(Color::WHITE)
        .font_stack(font.clone());
    let mut object = TextGridRenderObject::new(
        grid,
        10.0,
        20.0,
        14.0,
        Some(font.clone()),
        Color::BLACK,
        None,
        Color::WHITE,
    );
    object.layout(LayoutConstraints::tight(20.0, 20.0));
    assert!(matches!(object.update(&view), UpdateResult::NoChange));
    object.render();
    let mut paint = PaintContext::new();
    object.paint(&mut paint, Point::ZERO);
    let fonts: Vec<_> = paint
        .commands()
        .iter()
        .filter_map(|command| {
            if let PaintCommand::DrawText { font_stack, .. } = command {
                Some(font_stack.as_deref().map(FontStack::cache_id))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(fonts, vec![Some(font.cache_id()); 3]);

    // The retained commands used by accelerated renderers must have the same
    // font and baseline as the direct CPU path, including synthetic bold.
    let mut renderer =
        crate::renderer::CpuPaintRenderer::new(Size::new(20.0, 20.0), 1000, Color::BLACK);
    renderer.execute(&paint);
    assert_eq!(
        object.get_buffer().unwrap().as_slice(),
        renderer.buffer().as_slice()
    );
}
