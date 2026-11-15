//! Exercise composed layout, including the local origins inside modifiers.
use crate::{
    element::{Element, ElementRenderObject, LayoutConstraints},
    geometry::{Alignment, Point, Rect, Size},
    state::{State, generate_state_id},
    view::{View, ViewExt},
    views::{Button, HStack, Slider, SliderRenderObject, Text, TextRenderObject},
};
use alloc::vec::Vec;

fn bounds<R: ElementRenderObject + 'static>(root: &dyn Element) -> Vec<Rect> {
    fn walk<R: ElementRenderObject + 'static>(
        node: &dyn Element,
        origin: Point,
        out: &mut Vec<Rect>,
    ) {
        let origin = Point::new(origin.x + node.position().x, origin.y + node.position().y);
        if node.render_object().is_some_and(|r| r.as_any().is::<R>()) {
            out.push(Rect {
                origin,
                size: node.bounds().size,
            });
        }
        for child in node.children() {
            walk::<R>(child.as_ref(), origin, out);
        }
    }
    let mut out = Vec::new();
    walk::<R>(root, Point::ZERO, &mut out);
    out
}

fn slider() -> Slider {
    Slider::new(State::new(generate_state_id(), 0.8))
}

fn assert_centers(root: &mut dyn Element, expected_height: Option<f32>) {
    let size = root.layout(LayoutConstraints::loose(600.0, 100.0));
    // A parent's placement must not replace the frame's child-local origin.
    root.set_position(Point::new(17.0, 23.0));
    let label = bounds::<TextRenderObject>(root)[0];
    let track = bounds::<SliderRenderObject>(root)[0];
    let center = 23.0 + size.height / 2.0;
    assert_eq!(
        label.origin.y + label.size.height / 2.0,
        center,
        "label: {label:?}"
    );
    assert_eq!(
        track.origin.y + track.size.height / 2.0,
        center,
        "slider: {track:?}"
    );
    assert_eq!(
        track.size.height,
        crate::views::style::metrics().slider_height
    );
    if let Some(height) = expected_height {
        assert_eq!(size.height, height);
    }
}

#[test]
fn natural_hstack_centers_label_and_slider() {
    let mut row = HStack::new((
        Text::new("Brightness").font_size(12.0),
        slider().frame_width(270.0),
    ))
    .spacing(12.0)
    .create_element();
    assert_centers(row.as_mut(), None);
}

#[test]
fn row_height_alone_centers_natural_children() {
    for height in [32.0, 44.0] {
        let mut row = HStack::new((
            Text::new("Brightness").font_size(12.0),
            slider().frame_width(270.0),
        ))
        .spacing(12.0)
        .frame_height(height)
        .create_element();
        assert_centers(row.as_mut(), Some(height));
    }
}

#[test]
fn individual_frames_center_intrinsic_children() {
    for height in [32.0, 44.0] {
        let mut row = HStack::new((
            Text::new("Brightness").font_size(12.0).frame(112.0, height),
            slider().frame(270.0, height),
        ))
        .spacing(12.0)
        .create_element();
        assert_centers(row.as_mut(), Some(height));
    }
}

#[test]
fn settings_and_disabled_factory_slider_frames_align_with_labels() {
    for (height, enabled) in [(24.0, true), (26.0, false)] {
        let mut row = HStack::new((
            slider().enabled(enabled).frame(270.0, height),
            Text::new("80%").font_size(12.0).frame_width(36.0),
        ))
        .spacing(12.0)
        .create_element();
        assert_centers(row.as_mut(), Some(height));
    }
}

#[test]
fn frame_repositions_intrinsic_child_after_resize_and_preserves_explicit_alignment() {
    let mut root = Text::new("Label")
        .font_size(12.0)
        .frame(120.0, 32.0)
        .create_element();
    for height in [32.0, 44.0, 32.0] {
        root.update(&Text::new("Label").font_size(12.0).frame(120.0, height));
        root.layout(LayoutConstraints::unconstrained());
        let label = bounds::<TextRenderObject>(root.as_ref())[0];
        assert_eq!(label.origin.x + label.size.width / 2.0, 60.0);
        assert_eq!(label.origin.y + label.size.height / 2.0, height / 2.0);
    }
    let mut leading = Text::new("Label")
        .alignment(Alignment::Leading)
        .frame(120.0, 44.0)
        .create_element();
    leading.layout(LayoutConstraints::unconstrained());
    let label = bounds::<TextRenderObject>(leading.as_ref())[0];
    assert_eq!(label.origin.x, 0.0);
    assert_eq!(label.origin.y + label.size.height / 2.0, 22.0);
}

#[test]
fn frame_preserves_controls_that_accept_tight_constraints() {
    let mut button = Button::new("Button").frame(120.0, 44.0).create_element();
    button.layout(LayoutConstraints::unconstrained());
    let child = &button.children()[0];
    assert_eq!(child.position(), Point::ZERO);
    assert_eq!(child.bounds().size, Size::new(120.0, 44.0));
}

#[test]
fn frame_content_alignment_updates_without_expanding_the_unspecified_axis() {
    let label = Text::new("Label").font_size(12.0);
    let mut root = label.clone().frame_width(120.0).create_element();
    for alignment in [Alignment::Leading, Alignment::Trailing, Alignment::Center] {
        let result = root.update(
            &label
                .clone()
                .frame_width(120.0)
                .content_alignment(alignment),
        );
        assert!(matches!(result, crate::element::UpdateResult::Updated));
        let size = root.layout(LayoutConstraints::loose(600.0, 100.0));
        let text = bounds::<TextRenderObject>(root.as_ref())[0];
        assert_eq!(size.height, text.size.height);
        assert_eq!(text.origin, alignment.apply(size, text.size));
    }
}
