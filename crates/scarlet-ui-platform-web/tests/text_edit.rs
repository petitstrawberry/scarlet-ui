#[path = "../src/text_edit.rs"]
mod text_edit;
use scarlet_ui_core::{
    View,
    event::Event,
    pipeline::RenderingPipeline,
    state::{State, generate_state_id},
    views::TextField,
};
use text_edit::*;

fn field(initial: &str) -> (RenderingPipeline, State<String>) {
    let text = State::new(generate_state_id(), initial.to_owned());
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        TextField::new(text.clone())
            .autofocus(true)
            .create_element(),
    );
    pipeline.layout_initial();
    (pipeline, text)
}
fn apply(pipeline: &mut RenderingPipeline, events: Vec<Event>) {
    for event in events {
        assert!(pipeline.handle_event(&event), "{event:?}");
    }
    pipeline.render_with_damage();
}
#[test]
fn utf16_utf8_offsets_cover_japanese_and_surrogate_pairs() {
    let text = "a😀日本";
    for (units, bytes) in [(0, 0), (1, 1), (3, 5), (4, 8), (5, 11)] {
        assert_eq!(utf16_to_byte(text, units), bytes);
        assert_eq!(byte_to_utf16(text, bytes), units);
    }
    assert_eq!(utf16_to_byte(text, 2), 1); // no split surrogate
    assert_eq!(utf16_to_byte(text, 99), 11);
}
#[test]
fn minimal_field_route_inserts_replaces_and_deletes_without_keydown() {
    let (mut pipeline, text) = field("");
    for expected in ["a", "a😀", "a😀日本", "a語", "語", ""] {
        apply(
            &mut pipeline,
            edit_events(
                &text.get(),
                expected,
                expected.len() as u32,
                expected.len() as u32,
            ),
        );
        assert_eq!(text.get(), expected);
    }
}
#[test]
fn native_selection_is_directional_and_clamped_at_utf8_boundaries() {
    let (mut pipeline, _) = field("a😀日本");
    apply(&mut pipeline, vec![selection(1, 8)]);
    let state = pipeline.focused_text_input_state().unwrap();
    assert_eq!((state.cursor_byte, state.anchor_byte), (1, 8));
    apply(&mut pipeline, vec![selection(3, 1000)]);
    let state = pipeline.focused_text_input_state().unwrap();
    assert_eq!((state.cursor_byte, state.anchor_byte), (1, 11));
}
#[test]
fn composition_is_preedit_until_commit_and_final_duplicate_is_harmless() {
    let (mut pipeline, text) = field("ab");
    apply(
        &mut pipeline,
        vec![
            selection(1, 1),
            preedit("に".into()),
            preedit("日本".into()),
        ],
    );
    assert_eq!(text.get(), "ab");
    apply(&mut pipeline, edit_events("ab", "a日本b", 7, 7));
    assert_eq!(text.get(), "a日本b");
    apply(&mut pipeline, edit_events("a日本b", "a日本b", 7, 7));
    assert_eq!(text.get(), "a日本b");
    assert_eq!(pipeline.focused_text_input_state().unwrap().cursor_byte, 7);
}
#[test]
fn full_autocorrection_is_atomic_and_does_not_trigger_empty_callback() {
    let text = State::new(generate_state_id(), "teh".to_owned());
    let empties = State::new(generate_state_id(), 0);
    let count = empties.clone();
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        TextField::new(text.clone())
            .autofocus(true)
            .blur_on_empty(true)
            .on_empty(move || count.update(|v| *v += 1))
            .create_element(),
    );
    pipeline.layout_initial();
    apply(&mut pipeline, edit_events("teh", "the", 3, 3));
    assert_eq!(text.get(), "the");
    assert_eq!(empties.get(), 0);
    assert!(pipeline.focused_text_input_state().is_some());
}
#[test]
fn diff_never_splits_shared_unicode_bytes_or_overlaps_prefix_suffix() {
    for old in ["", "aaaa", "日本", "😀🙂", "a\u{301}"] {
        for new in ["", "aaa", "日本語", "🙂😀", "e\u{301}"] {
            let (start, end, insert) = replacement(old, new);
            assert_eq!(format!("{}{}{}", &old[..start], insert, &old[end..]), new);
        }
    }
}

#[test]
fn diagonal_scroll_and_boundary_slop_do_not_count_as_taps() {
    assert!(!tap_moved(10, 10, 14, 14));
    assert!(tap_moved(10, 10, 16, 16));
    assert!(tap_moved(10, 10, 18, 10));
    assert!(tap_moved(i32::MIN, i32::MIN, i32::MAX, i32::MAX));
}
#[test]
fn composition_over_repeated_text_keeps_the_original_anchor() {
    assert_eq!(
        composition_replacement("ああ", "あああ", 0, 0),
        (0, 0, "あ")
    );
    assert_eq!(composition_replacement("aaa", "aaaa", 1, 2), (1, 2, "aa"));
    assert_eq!(composition_replacement("abc", "xbc", 3, 3), (0, 1, "x"));
}
