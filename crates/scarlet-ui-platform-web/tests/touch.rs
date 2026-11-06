#[path = "../src/touch.rs"]
mod touch;
use scarlet_ui_core::event::{Event, MouseEvent, TouchPhase};
use touch::{MenuTouchRouting, TouchContacts};

#[test]
fn all_contacts_have_stable_ids_and_monotonic_frame_metadata() {
    let mut contacts = TouchContacts::default();
    let first = contacts.pointer("pointerdown", 17, 23, 45, 12.5).unwrap();
    let second = contacts.pointer("pointerdown", 91, 70, 80, 12.6).unwrap();
    let moved = contacts.pointer("pointermove", 17, 24, 46, 12.0).unwrap();
    assert_eq!(first.time_ns, 12_500_000);
    assert_eq!(first.changes[0].id, 17);
    assert_eq!(second.changes[0].id, 91);
    assert_eq!(moved.changes[0].id, 17);
    assert_eq!((moved.changes[0].x, moved.changes[0].y), (24, 46));
    assert!(moved.serial > second.serial && moved.time_ns > second.time_ns);
    assert_eq!(moved.changes[0].serial, moved.serial);
    assert_eq!(moved.changes[0].time_ns, moved.time_ns);
    assert_eq!(moved.changes[0].phase, TouchPhase::Move);
}

#[test]
fn end_then_lost_capture_is_not_a_second_end_and_unknown_moves_are_ignored() {
    let mut contacts = TouchContacts::default();
    assert!(contacts.pointer("pointermove", 7, 0, 0, 0.0).is_none());
    contacts.pointer("pointerdown", 7, 1, 2, 0.0).unwrap();
    assert!(contacts.pointer("pointerdown", 7, 1, 2, 0.0).is_none());
    assert!(contacts.pointer("pointerleave", 7, 1, 2, 0.0).is_none());
    assert_eq!(
        contacts.pointer("pointerup", 7, 3, 4, 0.0).unwrap().changes[0].phase,
        TouchPhase::Up
    );
    assert!(
        contacts
            .pointer("lostpointercapture", 7, 3, 4, 0.0)
            .is_none()
    );
    assert!(contacts.cancel_all(0.0).is_none());
}

#[test]
fn lost_capture_and_blur_cancel_every_active_contact_at_last_known_position() {
    let mut contacts = TouchContacts::default();
    contacts.pointer("pointerdown", -1, 1, 2, 0.0).unwrap();
    contacts.pointer("pointerdown", 2, 3, 4, 0.0).unwrap();
    let lost = contacts
        .pointer("lostpointercapture", 2, 0, 0, f64::NAN)
        .unwrap();
    assert_eq!((lost.changes[0].x, lost.changes[0].y), (3, 4));
    let cancelled = contacts.cancel_all(0.0).unwrap();
    assert_eq!(cancelled.changes[0].id, u32::MAX as u64);
    assert!(
        cancelled
            .changes
            .iter()
            .all(|c| c.phase == TouchPhase::Cancel)
    );
    assert!(contacts.cancel_all(0.0).is_none());
    assert!(contacts.pointer("pointerup", -1, 0, 0, 0.0).is_none());
}

#[test]
fn scene_touch_remains_touch_even_when_drag_moves_into_menu() {
    let mut contacts = TouchContacts::default();
    let mut routing = MenuTouchRouting::default();
    let mut calls = 0;
    for (kind, y) in [("pointerdown", 100), ("pointermove", 0), ("pointerup", 0)] {
        let mut frame = contacts.pointer(kind, 1, 10, y, 1.0).unwrap();
        assert!(
            routing
                .route(&mut frame, |_| {
                    calls += 1;
                    (false, None)
                })
                .is_none()
        );
        assert_eq!(frame.changes.len(), 1);
    }
    assert_eq!(calls, 1); // Only menu hit-testing on Down, never scene mouse synthesis.
}

#[test]
fn menu_owned_contact_never_leaks_release_into_scene() {
    let mut contacts = TouchContacts::default();
    let mut routing = MenuTouchRouting::default();
    let mut actions = 0;
    for kind in ["pointerdown", "pointermove", "pointerup"] {
        let mut frame = contacts.pointer(kind, 1, 10, 10, 1.0).unwrap();
        let action = routing.route(&mut frame, |event| {
            (
                true,
                matches!(event, Event::Mouse(MouseEvent::ButtonReleased { .. }))
                    .then(|| "action".into()),
            )
        });
        actions += usize::from(action.is_some());
        assert!(frame.changes.is_empty());
    }
    assert_eq!(actions, 1);
}

#[test]
fn blur_cancels_all_contacts_in_one_frame_and_pointer_ids_can_be_reused() {
    let mut contacts = TouchContacts::default();
    for id in [1, 2, 3] {
        contacts
            .pointer("pointerdown", id, id * 10, id * 20, 2.0)
            .unwrap();
    }
    let frame = contacts.cancel_all(3.0).unwrap();
    assert_eq!(frame.changes.len(), 3);
    assert!(
        frame
            .changes
            .iter()
            .all(|c| c.phase == TouchPhase::Cancel && c.serial == frame.serial)
    );
    assert_eq!(
        contacts
            .pointer("pointerdown", 1, 0, 0, 4.0)
            .unwrap()
            .changes[0]
            .phase,
        TouchPhase::Down
    );
}

#[test]
fn mixed_blur_frame_keeps_scene_cancel_and_removes_menu_cancel() {
    let mut contacts = TouchContacts::default();
    let mut routing = MenuTouchRouting::default();
    for (id, menu) in [(1, true), (2, false)] {
        let mut frame = contacts.pointer("pointerdown", id, 0, 0, 0.0).unwrap();
        routing.route(&mut frame, |_| (menu, None));
    }
    let mut frame = contacts.cancel_all(2.0).unwrap();
    let mut calls = 0;
    routing.route(&mut frame, |event| {
        assert!(matches!(
            event,
            Event::Mouse(MouseEvent::ButtonCancelled { .. })
        ));
        calls += 1;
        (true, None)
    });
    assert_eq!(calls, 1);
    assert_eq!(frame.changes.len(), 1);
    assert_eq!(frame.changes[0].id, 2);
}
