use super::*;
use crate::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, Phase, TouchChange,
    TouchGesture, TouchPhase,
};
use crate::state::{State, generate_state_id};
use crate::view::ViewExt;
use crate::views::{Text, TextField};

fn present(element: &mut dyn Element, active: bool) {
    assert!(element.handle_event(&Event::TabletPresentationChanged { active }, Phase::Capture));
}

fn render(element: &dyn Element) -> &WindowRenderObject {
    element
        .render_object()
        .unwrap()
        .as_any()
        .downcast_ref()
        .unwrap()
}

#[test]
fn tablet_presentation_suppresses_and_restores_declared_chrome_and_geometry() {
    for decoration in [
        WindowDecoration::CUSTOM,
        WindowDecoration::SYSTEM,
        WindowDecoration::NONE,
        WindowDecoration::new(WindowFrame::System, WindowTitleBar::Custom),
    ] {
        let window = Window::new(
            "Tablet",
            Text::new("Content").frame(f32::INFINITY, f32::INFINITY),
        )
        .size(Size::new(400.0, 300.0))
        .min_size(Size::new(200.0, 100.0))
        .decoration(decoration)
        .corner_radius(17.0)
        .window_geometry_insets(EdgeInsets::new(10.0, 12.0, 14.0, 16.0))
        .shadow(true);
        let declared = window.window_info();
        let mut element = window.create_element();
        let content_id = element.children().last().unwrap().id();
        let limits = element.get_window_size_limits();
        let surface = Size::new(640.0, 480.0);
        element.layout(LayoutConstraints::tight(surface.width, surface.height));
        let desktop_bounds = element.children().last().unwrap().bounds();

        for _ in 0..2 {
            present(element.as_mut(), true);
            present(element.as_mut(), true); // Replayed state after a pipeline rebuild.
            assert_eq!(element.bounds().size, surface);
            assert_eq!(element.children().len(), 1);
            assert_eq!(element.children()[0].id(), content_id);
            let info = element.get_window_info().unwrap();
            assert_eq!(info.decoration, WindowDecoration::NONE);
            assert_eq!(info.window_geometry_insets, EdgeInsets::ZERO);
            assert_eq!(info.shadow_elevation, ElevationRole::Flat);
            assert_eq!(info.effective_corner_radius(), 0.0);
            assert!(info.platform_surface_is_opaque());
            assert_eq!(info.size, declared.size);
            assert_eq!(element.get_window_size_limits(), limits);
            assert_eq!(render(element.as_ref()).window_geometry_size(), surface);
            assert!(render(element.as_ref()).deferred_backdrop().is_none());
            element.layout(LayoutConstraints::tight(surface.width, surface.height));
            assert_eq!(
                element.children()[0].bounds(),
                Rect::new(Point::ZERO, surface)
            );

            present(element.as_mut(), false);
            assert_eq!(element.bounds().size, surface);
            element.layout(LayoutConstraints::tight(surface.width, surface.height));
            let restored = element.get_window_info().unwrap();
            assert_eq!(restored.decoration, declared.decoration);
            assert_eq!(
                restored.window_geometry_insets,
                declared.window_geometry_insets
            );
            assert_eq!(restored.shadow_elevation, declared.shadow_elevation);
            assert_eq!(restored.corner_radius, declared.corner_radius);
            assert_eq!(element.children().last().unwrap().id(), content_id);
            assert_eq!(element.children().last().unwrap().bounds(), desktop_bounds);
        }
    }
}

#[test]
fn tablet_presentation_survives_rebuild_with_mounted_text_selection_and_focus() {
    let text = State::new(generate_state_id(), String::from("abcdef"));
    let field = TextField::new(text.clone()).autofocus(true);
    let window = Window::new("Before", field.clone()).decoration(WindowDecoration::CUSTOM);
    let mut element = window.create_element();
    let pipeline = PipelineId::generate();
    element.mount(&MountContext::new(pipeline));
    element.layout(LayoutConstraints::tight(640.0, 480.0));
    let root_id = element.id();
    let content = element.children_mut().last_mut().unwrap();
    let content_id = content.id();
    assert!(content.handle_event(
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Left,
            modifiers: KeyModifiers {
                shift: true,
                ..KeyModifiers::empty()
            },
        }),
        Phase::Target,
    ));
    let before = content.text_input_state().unwrap();
    assert_ne!(before.cursor_byte, before.anchor_byte);

    present(element.as_mut(), true);
    // Even changed desktop declarations must not replace the managed Window.
    let updated = Window::new("After", field)
        .decoration(WindowDecoration::new(
            WindowFrame::None,
            WindowTitleBar::Custom,
        ))
        .corner_radius(23.0)
        .window_geometry_insets(EdgeInsets::all(9.0))
        .shadow(true);
    let mut previous_active = true;
    for active in [true, true, false, true, false] {
        assert!(!matches!(element.update(&updated), UpdateResult::Replaced));
        assert!(!matches!(element.rebuild(), UpdateResult::Replaced));
        assert_eq!(
            element.get_window_info().unwrap().decoration,
            if previous_active {
                WindowDecoration::NONE
            } else {
                updated.get_decoration()
            }
        );
        assert_eq!(
            element.children().len(),
            if previous_active { 1 } else { 2 }
        );
        present(element.as_mut(), active);
        previous_active = active;
        element.layout(LayoutConstraints::tight(640.0, 480.0));
        assert_eq!(element.id(), root_id);
        let content = element.children().last().unwrap();
        assert_eq!(content.id(), content_id);
        let after = content.text_input_state().expect("focus must be retained");
        assert_eq!(after.cursor_byte, before.cursor_byte);
        assert_eq!(after.anchor_byte, before.anchor_byte);
        assert_eq!(after.surrounding_text, before.surrounding_text);
        let info = element.get_window_info().unwrap();
        assert_eq!(info.title, "After");
        assert_eq!(
            info.decoration,
            if active {
                WindowDecoration::NONE
            } else {
                updated.get_decoration()
            }
        );
        assert_eq!(
            info.window_geometry_insets,
            if active {
                EdgeInsets::ZERO
            } else {
                EdgeInsets::all(9.0)
            }
        );
    }
    // Editing must still replace the retained selection rather than append.
    assert!(
        element
            .children_mut()
            .last_mut()
            .unwrap()
            .handle_event(&Event::Keyboard(KeyEvent::Char { c: 'X' }), Phase::Target,)
    );
    assert_eq!(text.get(), "abcdeX");
    element.unmount();
    crate::pipeline::clear_global_dirty(pipeline);
}

#[test]
fn hidden_titlebar_cannot_receive_mouse_touch_or_a_stale_release() {
    let window = Window::new("Controls", Text::new("Content"))
        .decoration(WindowDecoration::CUSTOM)
        .shadow(false);
    let mut element = window.create_element();
    element.layout(LayoutConstraints::tight(400.0, 300.0));
    let close = render(element.as_ref()).close_button_rect(400).center();
    let (x, y) = (close.x as i32, close.y as i32);
    let press = Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x,
        y,
        click_count: 1,
    });
    let release = Event::Mouse(MouseEvent::ButtonReleased {
        button: MouseButton::Left,
        x,
        y,
        click_count: 1,
    });
    assert!(element.handle_event(&press, Phase::Capture));
    let touch = Event::Touch(TouchChange {
        seat_id: 1,
        serial: 1,
        time_ns: 1,
        id: 9,
        phase: TouchPhase::Down,
        x,
        y,
        pressure: None,
        touch_major: None,
    });
    let tap = Event::TouchGesture(TouchGesture::Tap { id: 9, x, y });
    assert!(element.handle_event(&touch, Phase::Capture));
    present(element.as_mut(), true);
    for phase in [Phase::Capture, Phase::Target, Phase::Bubble] {
        assert!(!element.handle_event(&press, phase));
        assert!(!element.handle_event(&release, phase));
        assert!(!element.handle_event(&touch, phase));
        assert!(!element.handle_event(&tap, phase));
        assert!(!element.handle_event(
            &Event::TouchGesture(crate::event::TouchGesture::Drag {
                id: 1,
                phase: crate::event::GesturePhase::Started,
                x: 100,
                y: 16,
                delta_x: 10,
                delta_y: 0,
            }),
            phase
        ));
        assert_eq!(element.take_window_action(), None);
    }
    present(element.as_mut(), false);
    assert!(!element.handle_event(&release, Phase::Capture));
    assert!(!element.handle_event(&tap, Phase::Capture));
    assert_eq!(element.take_window_action(), None);
    assert!(element.handle_event(&press, Phase::Capture));
    assert!(element.handle_event(&release, Phase::Capture));
    assert_eq!(
        element.take_window_action(),
        Some(crate::event::WindowEvent::CloseRequested)
    );
}

#[test]
fn tablet_occlusion_pans_content_without_resizing_and_restores_it() {
    let window = Window::new(
        "Form",
        Text::new("Content").frame(f32::INFINITY, f32::INFINITY),
    )
    .decoration(WindowDecoration::CUSTOM)
    .shadow(false);
    let mut element = window.create_element();
    let constraints = LayoutConstraints::tight(400.0, 600.0);
    element.layout(constraints);
    let occlusion = Some(Rect::from_xywh(0.0, 400.0, 400.0, 200.0));
    let caret = Some(Rect::from_xywh(20.0, 490.0, 2.0, 20.0));
    assert!(!element.avoid_input_occlusion(occlusion, caret, true));
    present(element.as_mut(), true);
    element.layout(constraints);
    let content_id = element.children()[0].id();
    let content_size = element.children()[0].bounds().size;
    assert!(element.avoid_input_occlusion(occlusion, caret, true));
    assert_eq!(element.children()[0].position(), Point::new(0.0, -110.0));
    let moved_caret = Some(Rect::from_xywh(20.0, 380.0, 2.0, 20.0));
    assert!(!element.avoid_input_occlusion(occlusion, moved_caret, true));
    assert!(!element.avoid_input_occlusion(occlusion, caret, false));
    assert!(!matches!(element.update(&window), UpdateResult::Replaced));
    present(element.as_mut(), true);
    element.layout(constraints);
    assert_eq!(element.children()[0].position(), Point::new(0.0, -110.0));
    assert_eq!(element.children()[0].bounds().size, content_size);
    assert_eq!(element.bounds().size, Size::new(400.0, 600.0));
    assert_eq!(element.children()[0].id(), content_id);
    // Hit testing uses the same translated element position as painting.
    assert!(element.children()[0].hit_test(Point::new(20.0, 480.0)));
    assert!(!element.children()[0].hit_test(Point::new(20.0, 590.0)));
    assert!(element.avoid_input_occlusion(occlusion, None, false));
    assert_eq!(element.children()[0].position(), Point::ZERO);
    assert!(element.avoid_input_occlusion(occlusion, caret, true));
    assert!(element.avoid_input_occlusion(None, caret, false));
    assert_eq!(element.children()[0].position(), Point::ZERO);
    assert!(!element.avoid_input_occlusion(
        occlusion,
        Some(Rect::from_xywh(20.0, 100.0, 2.0, 20.0)),
        true
    ));
    assert!(!element.avoid_input_occlusion(
        Some(Rect::from_xywh(200.0, 400.0, 200.0, 200.0)),
        caret,
        true
    ));
    assert!(element.avoid_input_occlusion(occlusion, caret, true));
    present(element.as_mut(), false);
    element.layout(constraints);
    assert_eq!(
        element.children().last().unwrap().position(),
        WindowContentLayout::new(true).offset()
    );
}
