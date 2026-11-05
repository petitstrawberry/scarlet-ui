#[path = "../src/safe_area.rs"]
mod safe_area;

use safe_area::canvas_insets;
use scarlet_ui_core::geometry::{EdgeInsets, Rect, Size};

#[test]
fn full_screen_canvas_receives_browser_insets_without_device_constants() {
    let viewport = Size::new(417., 893.);
    let insets = EdgeInsets::new(7., 53., 11., 31.);
    assert_eq!(
        canvas_insets(
            viewport,
            Rect::from_xywh(0., 0., 417., 893.),
            viewport,
            insets
        ),
        insets
    );
    assert_eq!(
        canvas_insets(
            Size::new(893., 417.),
            Rect::from_xywh(0., 0., 893., 417.),
            Size::new(893., 417.),
            EdgeInsets::new(53., 0., 53., 21.)
        ),
        EdgeInsets::new(53., 0., 53., 21.)
    );
}

#[test]
fn host_insetting_and_embedded_specimens_do_not_receive_padding_twice() {
    let viewport = Size::new(417., 893.);
    let insets = EdgeInsets::new(7., 53., 11., 31.);
    for rect in [
        Rect::from_xywh(7., 53., 399., 809.),
        Rect::from_xywh(40., 200., 300., 240.),
        Rect::from_xywh(40., -80., 300., 240.),
    ] {
        assert_eq!(
            canvas_insets(viewport, rect, rect.size, insets),
            EdgeInsets::ZERO
        );
    }
}

#[test]
fn css_scaling_projects_insets_into_logical_canvas_coordinates() {
    let viewport = Size::new(400., 800.);
    assert_eq!(
        canvas_insets(
            viewport,
            Rect::from_xywh(0., 0., 400., 800.),
            Size::new(800., 1600.),
            EdgeInsets::new(0., 40., 0., 20.)
        ),
        EdgeInsets::new(0., 80., 0., 40.)
    );
}
