#[path = "../src/safe_area.rs"]
mod safe_area;

use safe_area::{SafeAreaMode, canvas_insets};
use scarlet_ui_core::geometry::{EdgeInsets, Size};

#[test]
fn viewport_mode_preserves_api_values_without_window_size_heuristics() {
    let insets = EdgeInsets::new(7., 53., 11., 31.);
    for css in [
        Size::new(417., 893.),
        Size::new(417., 1100.),
        Size::new(893., 417.),
    ] {
        assert_eq!(
            canvas_insets(SafeAreaMode::Viewport, css, css, insets),
            insets
        );
    }
}

#[test]
fn contained_mode_does_not_apply_browser_insets_twice() {
    assert_eq!(
        canvas_insets(
            SafeAreaMode::Contained,
            Size::new(300., 240.),
            Size::new(300., 240.),
            EdgeInsets::new(7., 53., 11., 31.)
        ),
        EdgeInsets::ZERO
    );
}

#[test]
fn css_scaling_projects_api_values_into_canvas_logical_coordinates() {
    assert_eq!(
        canvas_insets(
            SafeAreaMode::Viewport,
            Size::new(400., 800.),
            Size::new(800., 1600.),
            EdgeInsets::new(0., 40., 0., 20.)
        ),
        EdgeInsets::new(0., 80., 0., 40.)
    );
}
