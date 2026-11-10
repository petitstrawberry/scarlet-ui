//! Native clipboard transactions belong to one live window and one retained editor.
use super::RenderingPipeline;
use crate::clipboard::{
    ClipboardError, ClipboardHandle, TextClipboard, TextEditAction, TextEditState,
};
use crate::element::ElementId;
use crate::event::{Event, KeyCode, KeyEvent, MouseEvent};
use crate::geometry::Point;
use crate::os::Mutex;
use crate::views::{Menu, MenuItemContent};
use alloc::{string::String, sync::Arc};

struct Request {
    editor: ElementId,
    snapshot: TextEditState,
    action: TextEditAction,
}
enum Operation {
    Read(ClipboardHandle<Option<String>>),
    Write(ClipboardHandle<()>),
}
struct Pending {
    request: Request,
    operation: Operation,
    valid: bool,
}
pub(super) struct TextEditing {
    clipboard: Option<TextClipboard>,
    pending: Option<Pending>,
    queued: Arc<Mutex<Option<Request>>>,
    error: Option<ClipboardError>,
}

impl Default for TextEditing {
    fn default() -> Self {
        Self {
            clipboard: None,
            pending: None,
            queued: Arc::new(Mutex::new(None)),
            error: None,
        }
    }
}

impl RenderingPipeline {
    /// Connect this window's standard text editing to its real platform clipboard.
    /// ApplicationRunner installs this automatically. Custom runners should also
    /// poll [`Self::poll_text_clipboard`] on idle ticks.
    pub fn set_text_clipboard(&mut self, clipboard: Option<TextClipboard>) {
        self.text_editing.clipboard = clipboard;
        self.text_editing.pending = None;
        self.text_editing.error = None;
        *self.text_editing.queued.lock() = None;
    }

    /// Take an error from the last standard clipboard action. Failed cuts never
    /// remove text; failed pastes leave the document unchanged.
    pub fn take_text_clipboard_error(&mut self) -> Option<ClipboardError> {
        self.text_editing.error.take()
    }

    pub(super) fn invalidate_text_clipboard_for_event(&mut self, event: &Event) {
        if matches!(
            event,
            Event::Keyboard(KeyEvent::Pressed { .. } | KeyEvent::Char { .. })
                | Event::Mouse(MouseEvent::ButtonPressed { .. })
                | Event::Focus(_)
                | Event::TouchFrame(_)
                | Event::Touch(_)
                | Event::TouchGesture(_)
                | Event::TextInputCommit { .. }
                | Event::TextInputPreedit { .. }
                | Event::TextInputSelection { .. }
                | Event::TextInputDeleteSurroundingText { .. }
        ) && let Some(pending) = &mut self.text_editing.pending
        {
            pending.valid = false;
        }
    }

    pub(super) fn handle_text_edit_shortcut(&mut self, event: &Event) -> bool {
        if self.text_editing.clipboard.is_none() {
            return false;
        }
        let Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Char(key),
            modifiers,
        }) = event
        else {
            return false;
        };
        if !modifiers.primary() || modifiers.alt || modifiers.shift {
            return false;
        }
        let action = match key.to_ascii_lowercase() {
            'c' => TextEditAction::Copy,
            'x' => TextEditAction::Cut,
            'v' => TextEditAction::Paste,
            _ => return false,
        };
        self.event_dispatcher
            .reconcile_keyboard_focus(&mut self.element_tree);
        let Some(input) = self.focused_text_input_state() else {
            return false;
        };
        let Some(editor) = input.editor_id else {
            return false;
        };
        let Some(snapshot) = self
            .element_tree
            .find_element(editor)
            .and_then(|element| element.text_edit_state())
        else {
            return false;
        };
        // Preserve existing custom event/callback routing.
        if matches!(action, TextEditAction::Copy | TextEditAction::Cut) && snapshot.custom_copy
            || action == TextEditAction::Paste && snapshot.custom_paste
        {
            return false;
        }
        // Standard editing is a target handler: capture observers/overrides
        // retain exactly the same precedence as other text editing keys.
        if let Some(path) = self.element_tree.find_path_ids(editor) {
            for id in path.iter().take(path.len().saturating_sub(1)) {
                if self
                    .element_tree
                    .find_element_mut(*id)
                    .is_some_and(|element| {
                        element.handle_event(event, crate::event::Phase::Capture)
                    })
                {
                    return true;
                }
            }
        }
        self.start_text_edit(Request {
            editor,
            snapshot,
            action,
        });
        true
    }

    fn request_is_current(&self, request: &Request) -> bool {
        self.element_tree
            .find_element(request.editor)
            .is_some_and(|element| {
                element.wants_keyboard_focus()
                    && element
                        .text_edit_state()
                        .is_some_and(|state| request.snapshot.matches(&state))
            })
    }

    fn apply_text_edit_request(&mut self, request: &Request, text: Option<&str>) -> bool {
        if !self.request_is_current(request) {
            return false;
        }
        self.element_tree
            .find_element_mut(request.editor)
            .is_some_and(|element| element.apply_text_edit(request.action, text))
    }

    fn start_text_edit(&mut self, request: Request) {
        if !self.request_is_current(&request) {
            return;
        }
        let custom = match request.action {
            TextEditAction::Copy | TextEditAction::Cut => request.snapshot.custom_copy,
            TextEditAction::Paste => request.snapshot.custom_paste,
            TextEditAction::SelectAll => true,
        };
        if custom {
            self.apply_text_edit_request(&request, None);
            return;
        }
        if self.text_editing.pending.is_some() {
            self.text_editing.error = Some(ClipboardError::Busy);
            return;
        }
        let Some(clipboard) = &self.text_editing.clipboard else {
            return;
        };
        let operation = match request.action {
            TextEditAction::Copy | TextEditAction::Cut => {
                let Some(range) = request.snapshot.selection.normalized_range() else {
                    return;
                };
                let document = request.snapshot.document.as_str();
                let Some(text) = document.get(range) else {
                    return;
                };
                Operation::Write(clipboard.write_text(String::from(text)))
            }
            TextEditAction::Paste => Operation::Read(clipboard.read_text()),
            TextEditAction::SelectAll => unreachable!(),
        };
        self.text_editing.pending = Some(Pending {
            request,
            operation,
            valid: true,
        });
    }

    /// Complete nonblocking clipboard operations. Cut deletes only after a
    /// successful write. Intervening input, focus changes, document/selection
    /// updates, disabled controls, and replaced editors discard stale edits.
    pub fn poll_text_clipboard(&mut self) -> bool {
        let queued = self.text_editing.queued.lock().take();
        if let Some(request) = queued {
            self.start_text_edit(request);
        }
        let Some(pending) = &self.text_editing.pending else {
            return false;
        };
        let result = match &pending.operation {
            Operation::Read(handle) => handle.try_take(),
            Operation::Write(handle) => handle.try_take().map(|result| result.map(|_| None)),
        };
        let Some(result) = result else {
            return false;
        };
        let pending = self.text_editing.pending.take().unwrap();
        match result {
            Err(error) => {
                self.text_editing.error = Some(error);
                false
            }
            Ok(text) if pending.valid => match pending.request.action {
                TextEditAction::Cut => self.apply_text_edit_request(&pending.request, Some("")),
                TextEditAction::Paste => text
                    .as_deref()
                    .is_some_and(|text| self.apply_text_edit_request(&pending.request, Some(text))),
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn text_edit_context_menu(&mut self, point: Point) -> Option<Menu> {
        // Browser textarea editing keeps the browser's native context menu.
        self.text_editing.clipboard.as_ref()?;
        let hit = self
            .event_dispatcher
            .hit_test_with_path(&self.element_tree, point)?;
        let (editor, snapshot) = hit.path.iter().rev().find_map(|element| {
            element
                .text_edit_state()
                .map(|snapshot| (element.id(), snapshot))
        })?;
        let path = self.element_tree.find_path_ids(editor)?;
        self.event_dispatcher
            .set_focused_element(&mut self.element_tree, editor, &path);
        let selected = snapshot.selection.normalized_range().is_some();
        let row = |label: &str, key: &str, action, enabled| {
            let queued = self.text_editing.queued.clone();
            let snapshot = snapshot.clone();
            MenuItemContent::new(label)
                .shortcut(key)
                .enabled(enabled)
                .callback(move || {
                    *queued.lock() = Some(Request {
                        editor,
                        snapshot: snapshot.clone(),
                        action,
                    });
                })
        };
        let primary = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        Some(Menu::new(alloc::vec![
            row(
                "Cut",
                &alloc::format!("{primary}+X"),
                TextEditAction::Cut,
                selected && !snapshot.composing
            ),
            row(
                "Copy",
                &alloc::format!("{primary}+C"),
                TextEditAction::Copy,
                selected && !snapshot.composing
            ),
            row(
                "Paste",
                &alloc::format!("{primary}+V"),
                TextEditAction::Paste,
                !snapshot.composing
            ),
            MenuItemContent::separator(),
            row(
                "Select All",
                &alloc::format!("{primary}+A"),
                TextEditAction::SelectAll,
                !snapshot.document.is_empty() && !snapshot.composing
            ),
        ]))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::clipboard::ClipboardProvider;
    use crate::element::LayoutConstraints;
    use crate::event::{FocusEvent, KeyModifiers, MouseButton};
    use crate::state::{State, generate_state_id};
    use crate::view::View;
    use crate::views::{TextField, TextSelection, TextView};
    use std::sync::Barrier;
    use std::time::{Duration, Instant};

    struct Provider {
        text: Arc<Mutex<Option<String>>>,
        fail: bool,
        gate: Option<Arc<Barrier>>,
    }
    impl ClipboardProvider for Provider {
        fn read_text(&self) -> Result<Option<String>, ClipboardError> {
            if let Some(gate) = &self.gate {
                gate.wait();
            }
            if self.fail {
                Err(ClipboardError::AccessDenied)
            } else {
                Ok(self.text.lock().clone())
            }
        }
        fn write_text(&self, value: &str) -> Result<(), ClipboardError> {
            if let Some(gate) = &self.gate {
                gate.wait();
            }
            if self.fail {
                return Err(ClipboardError::AccessDenied);
            }
            *self.text.lock() = Some(value.into());
            Ok(())
        }
    }
    fn setup(
        initial: &str,
        clipboard: Option<&str>,
        fail: bool,
        gate: Option<Arc<Barrier>>,
    ) -> (RenderingPipeline, State<String>, Arc<Mutex<Option<String>>>) {
        let text = State::new(generate_state_id(), initial.into());
        let stored = Arc::new(Mutex::new(clipboard.map(String::from)));
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            TextField::new(text.clone())
                .autofocus(true)
                .create_element(),
        );
        pipeline
            .element_tree
            .layout(LayoutConstraints::tight(300.0, 40.0));
        pipeline.set_text_clipboard(Some(TextClipboard::new(Provider {
            text: stored.clone(),
            fail,
            gate,
        })));
        (pipeline, text, stored)
    }
    fn key(keycode: KeyCode, primary: bool) -> Event {
        Event::Keyboard(KeyEvent::Pressed {
            keycode,
            modifiers: KeyModifiers {
                control: primary && !cfg!(target_os = "macos"),
                super_key: primary && cfg!(target_os = "macos"),
                ..Default::default()
            },
        })
    }
    fn shortcut(pipeline: &mut RenderingPipeline, keycode: char) {
        assert!(pipeline.handle_event(&key(KeyCode::Char(keycode), true)));
    }
    fn finish(pipeline: &mut RenderingPipeline) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while pipeline.text_editing.pending.is_some()
            || pipeline.text_editing.queued.lock().is_some()
        {
            pipeline.poll_text_clipboard();
            assert!(
                Instant::now() < deadline,
                "clipboard operation did not complete"
            );
            std::thread::yield_now();
        }
    }
    #[test]
    fn text_field_unicode_copy_cut_paste_is_automatic_and_single_line() {
        let (mut pipeline, text, stored) = setup("日本 👩‍💻", None, false, None);
        shortcut(&mut pipeline, 'a');
        shortcut(&mut pipeline, 'c');
        finish(&mut pipeline);
        assert_eq!(stored.lock().as_deref(), Some("日本 👩‍💻"));
        assert_eq!(text.get(), "日本 👩‍💻");
        shortcut(&mut pipeline, 'x');
        finish(&mut pipeline);
        assert_eq!(text.get(), "");
        *stored.lock() = Some("hello\r\nworld\t!".into());
        shortcut(&mut pipeline, 'v');
        finish(&mut pipeline);
        assert_eq!(text.get(), "helloworld\t!");
    }
    #[test]
    fn failed_cut_and_paste_never_modify_text_and_report_error() {
        for action in ['x', 'v'] {
            let (mut pipeline, text, _) = setup("keep", Some("replace"), true, None);
            shortcut(&mut pipeline, 'a');
            shortcut(&mut pipeline, action);
            finish(&mut pipeline);
            assert_eq!(text.get(), "keep");
            assert_eq!(
                pipeline.take_text_clipboard_error(),
                Some(ClipboardError::AccessDenied)
            );
            assert_eq!(pipeline.take_text_clipboard_error(), None);
        }
    }
    #[test]
    fn empty_clipboard_does_not_delete_selection() {
        let (mut pipeline, text, _) = setup("keep", None, false, None);
        shortcut(&mut pipeline, 'a');
        shortcut(&mut pipeline, 'v');
        finish(&mut pipeline);
        assert_eq!(text.get(), "keep");
    }
    #[test]
    fn in_flight_edits_are_discarded_after_typing_selection_or_focus_changes() {
        for action in ['x', 'v'] {
            for interruption in 0..3 {
                let gate = Arc::new(Barrier::new(2));
                let (mut pipeline, text, _) =
                    setup("keep", Some("replacement"), false, Some(gate.clone()));
                shortcut(&mut pipeline, 'a');
                shortcut(&mut pipeline, action);
                assert_eq!(text.get(), "keep");
                match interruption {
                    0 => {
                        pipeline.handle_event(&Event::Keyboard(KeyEvent::Char { c: '!' }));
                    }
                    1 => {
                        pipeline.handle_event(&key(KeyCode::Right, false));
                        shortcut(&mut pipeline, 'a');
                    }
                    _ => {
                        pipeline.handle_event(&Event::Focus(FocusEvent::Lost));
                        pipeline.handle_event(&Event::Focus(FocusEvent::Gained));
                    }
                }
                gate.wait();
                finish(&mut pipeline);
                assert_eq!(text.get(), if interruption == 0 { "!" } else { "keep" });
            }
        }
    }
    #[test]
    fn external_state_change_and_replaced_editor_reject_pending_paste() {
        for replace in [false, true] {
            let gate = Arc::new(Barrier::new(2));
            let (mut pipeline, text, _) =
                setup("old", Some("clipboard"), false, Some(gate.clone()));
            shortcut(&mut pipeline, 'v');
            text.set("new".into());
            if replace {
                pipeline.set_root(
                    TextField::new(text.clone())
                        .autofocus(true)
                        .create_element(),
                );
            }
            gate.wait();
            finish(&mut pipeline);
            assert_eq!(text.get(), "new");
        }
    }
    #[test]
    fn unrelated_field_rebuild_preserves_pending_transaction() {
        let gate = Arc::new(Barrier::new(2));
        let (mut pipeline, text, _) = setup("a", Some("b"), false, Some(gate.clone()));
        shortcut(&mut pipeline, 'v');
        pipeline.element_tree.root_mut().unwrap().update(
            &TextField::new(text.clone())
                .autofocus(true)
                .placeholder("changed"),
        );
        gate.wait();
        finish(&mut pipeline);
        assert_eq!(text.get(), "ab");
    }
    #[test]
    fn text_view_default_clipboard_preserves_multiline_and_custom_callbacks_win() {
        for custom in [false, true] {
            let (mut pipeline, _, stored) = setup("", Some("platform\ntext"), false, None);
            let text = State::new(generate_state_id(), String::from("日本"));
            let selection = State::new(generate_state_id(), TextSelection::collapsed(6));
            let mut view = TextView::new(text.clone(), selection).autofocus(true);
            if custom {
                view = view.on_paste(|| Some(alloc::borrow::Cow::Borrowed("custom")));
            }
            pipeline.set_root(view.create_element());
            pipeline
                .element_tree
                .layout(LayoutConstraints::tight(300.0, 100.0));
            shortcut(&mut pipeline, 'a');
            shortcut(&mut pipeline, 'v');
            finish(&mut pipeline);
            assert_eq!(text.get(), if custom { "custom" } else { "platform\ntext" });
            shortcut(&mut pipeline, 'a');
            shortcut(&mut pipeline, 'x');
            finish(&mut pipeline);
            assert_eq!(text.get(), "");
            assert_eq!(
                stored.lock().as_deref(),
                Some(if custom { "custom" } else { "platform\ntext" })
            );
        }
    }
    #[test]
    fn right_click_preserves_selection_and_menu_uses_same_async_cut() {
        let (mut pipeline, text, stored) = setup("selected", None, false, None);
        shortcut(&mut pipeline, 'a');
        let before = pipeline.focused_text_input_state().unwrap();
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Right,
                x: 10,
                y: 10,
                click_count: 1
            }))
        );
        let after = pipeline.focused_text_input_state().unwrap();
        assert_eq!(
            (before.cursor_byte, before.anchor_byte),
            (after.cursor_byte, after.anchor_byte)
        );
        pipeline.handle_event(&key(KeyCode::Down, false));
        pipeline.handle_event(&key(KeyCode::Enter, false));
        finish(&mut pipeline);
        assert_eq!(text.get(), "");
        assert_eq!(stored.lock().as_deref(), Some("selected"));
    }
    #[test]
    fn composition_and_disabled_fields_do_not_start_clipboard_operations() {
        let (mut pipeline, text, stored) = setup("keep", None, false, None);
        pipeline.handle_event(&Event::TextInputPreedit {
            context_id: 1,
            serial: 1,
            cursor_byte: 0,
            anchor_byte: 0,
            text: "かな".into(),
            spans: alloc::vec![],
        });
        shortcut(&mut pipeline, 'x');
        finish(&mut pipeline);
        assert_eq!(text.get(), "keep");
        assert!(stored.lock().is_none());
        pipeline
            .element_tree
            .root_mut()
            .unwrap()
            .update(&TextField::new(text.clone()).enabled(false));
        assert!(!pipeline.handle_text_edit_shortcut(&key(KeyCode::Char('v'), true)));
        assert!(
            pipeline
                .text_edit_context_menu(Point::new(10.0, 10.0))
                .is_none()
        );
    }
    #[test]
    fn capture_overrides_precede_native_clipboard_and_observers_run_once() {
        use crate::view::ViewExt;
        use alloc::rc::Rc;
        use core::cell::Cell;
        for consume in [false, true] {
            let (mut pipeline, text, stored) = setup("kept", None, false, None);
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            pipeline.set_root(
                TextField::new(text.clone())
                    .autofocus(true)
                    .on_event_capture(move |event| {
                        if matches!(
                            event,
                            Event::Keyboard(KeyEvent::Pressed {
                                keycode: KeyCode::Char('x'),
                                ..
                            })
                        ) {
                            observed.set(observed.get() + 1);
                            consume
                        } else {
                            false
                        }
                    })
                    .create_element(),
            );
            pipeline
                .element_tree
                .layout(LayoutConstraints::tight(300.0, 40.0));
            shortcut(&mut pipeline, 'a');
            shortcut(&mut pipeline, 'x');
            finish(&mut pipeline);
            assert_eq!(calls.get(), 1);
            assert_eq!(text.get(), if consume { "kept" } else { "" });
            assert_eq!(
                stored.lock().as_deref(),
                if consume { None } else { Some("kept") }
            );
        }
    }
    #[test]
    fn disabling_editor_discards_pending_cut_and_paste() {
        for action in ['x', 'v'] {
            let gate = Arc::new(Barrier::new(2));
            let (mut pipeline, text, _) =
                setup("keep", Some("replacement"), false, Some(gate.clone()));
            shortcut(&mut pipeline, 'a');
            shortcut(&mut pipeline, action);
            pipeline
                .element_tree
                .root_mut()
                .unwrap()
                .update(&TextField::new(text.clone()).enabled(false));
            gate.wait();
            finish(&mut pipeline);
            assert_eq!(text.get(), "keep");
        }
    }
    #[test]
    fn repeated_command_reports_busy_without_destructive_edit() {
        let gate = Arc::new(Barrier::new(2));
        let (mut pipeline, text, stored) = setup("keep", Some("old"), false, Some(gate.clone()));
        shortcut(&mut pipeline, 'a');
        shortcut(&mut pipeline, 'x');
        shortcut(&mut pipeline, 'v');
        assert_eq!(
            pipeline.take_text_clipboard_error(),
            Some(ClipboardError::Busy)
        );
        gate.wait();
        finish(&mut pipeline);
        assert_eq!(text.get(), "keep");
        // The already submitted copy may complete; it never authorizes stale deletion.
        assert_eq!(stored.lock().as_deref(), Some("keep"));
    }
}
