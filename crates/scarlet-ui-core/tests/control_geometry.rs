use scarlet_ui_core::{
    element::LayoutConstraints,
    prelude::*,
    renderer::{PaintCommand, PaintContext},
};

fn state<T: Clone + 'static>(value: T) -> State<T> {
    State::new(scarlet_ui_core::generate_state_id(), value)
}

#[test]
fn default_controls_share_height_type_size_and_corner_radius() {
    let controls: Vec<Box<dyn Element>> = vec![
        Button::new("Continue").create_element(),
        TextField::new(state(String::from("Project Atlas"))).create_element(),
        Select::new(vec![String::from("Workspace")], state(0usize)).create_element(),
    ];
    for mut element in controls {
        let size = element.layout(LayoutConstraints::unconstrained());
        assert_eq!(size.height, 28.0);
        let mut ctx = PaintContext::new();
        element
            .render_object()
            .unwrap()
            .paint(&mut ctx, Point::ZERO);
        assert!(ctx.commands().iter().any(|command| matches!(command,
            PaintCommand::FillVerticalGradientRoundedRect { corner_radius, .. }
                | PaintCommand::BackdropFilterRoundedRect { corner_radius, .. } if *corner_radius == 12.0)));
        assert!(ctx.commands().iter().any(|command| matches!(command,
            PaintCommand::DrawText { font_size_px, .. } if *font_size_px == 14.0)));
    }
}

#[test]
fn button_measures_unicode_labels_instead_of_utf8_byte_length() {
    for label in ["Continue", "保存する", "Ångström", "WWW", "iii"] {
        let mut element = Button::new(label).create_element();
        let size = element.layout(LayoutConstraints::unconstrained());
        let (width, _) = scarlet_ui_core::graphics::measure_text_sized(label, 14.0);
        assert_eq!(size.width, width as f32 + 16.0);
    }
}

#[test]
fn explicit_small_button_frames_and_compact_header_remain_supported() {
    let mut button = Button::new("Go").create_element();
    assert_eq!(
        button.layout(LayoutConstraints::tight(60.0, 24.0)),
        Size::new(60.0, 24.0)
    );
    for mut control in [
        TextField::new(state(String::new())).create_element(),
        Select::new(vec![String::from("Small")], state(0usize)).create_element(),
    ] {
        assert_eq!(
            control.layout(LayoutConstraints::tight(80.0, 22.0)),
            Size::new(80.0, 22.0)
        );
    }
    let mut select = Select::new(vec![String::from("Small")], state(0usize))
        .width(90.0)
        .row_height(22.0)
        .create_element();
    assert_eq!(
        select.layout(LayoutConstraints::unconstrained()),
        Size::new(90.0, 22.0)
    );
    let mut header = Button::icon_only(Icon::Settings)
        .header_style()
        .create_element();
    assert!(header.layout(LayoutConstraints::unconstrained()).height < 36.0);
}
