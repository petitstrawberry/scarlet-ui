//! Deduplicate successfully committed text-input protocol state.

use alloc::string::String;

#[derive(Clone, Copy)]
pub(super) struct State<'a> {
    pub rect: (i32, i32, u32, u32),
    pub text: &'a str,
    pub cursor: u32,
    pub anchor: u32,
    pub hint: u32,
    pub purpose: u32,
}

#[derive(Debug, PartialEq)]
pub(super) enum Request<'a> {
    CursorRect(u32, (i32, i32, u32, u32)),
    SurroundingText(u32, u32, u32, &'a str),
    ContentType(u32, u32, u32),
    Commit(u32, u32),
}

#[derive(Debug, PartialEq)]
pub(super) enum SyncResult {
    Unchanged,
    Committed,
    Failed,
}

struct CommittedState {
    context: u32,
    rect: (i32, i32, u32, u32),
    text: String,
    cursor: u32,
    anchor: u32,
    hint: u32,
    purpose: u32,
}

#[derive(Default)]
pub(super) struct TextInputSync {
    committed: Option<CommittedState>,
}

impl TextInputSync {
    pub fn invalidate(&mut self) {
        self.committed = None;
    }

    pub fn synchronize<'a>(
        &mut self,
        context: u32,
        serial: u32,
        state: State<'a>,
        mut send: impl FnMut(Request<'a>) -> bool,
    ) -> SyncResult {
        if self.committed.as_ref().is_some_and(|old| {
            old.context == context
                && old.rect == state.rect
                && old.text == state.text
                && old.cursor == state.cursor
                && old.anchor == state.anchor
                && old.hint == state.hint
                && old.purpose == state.purpose
        }) {
            return SyncResult::Unchanged;
        }
        // Only a complete, successfully published transaction may suppress a
        // future update. A partial send must be retried even for identical text.
        self.invalidate();
        if !send(Request::CursorRect(context, state.rect))
            || !send(Request::SurroundingText(
                context,
                state.cursor,
                state.anchor,
                state.text,
            ))
            || !send(Request::ContentType(context, state.hint, state.purpose))
            || !send(Request::Commit(context, serial))
        {
            return SyncResult::Failed;
        }
        self.committed = Some(CommittedState {
            context,
            rect: state.rect,
            text: String::from(state.text),
            cursor: state.cursor,
            anchor: state.anchor,
            hint: state.hint,
            purpose: state.purpose,
        });
        SyncResult::Committed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn state() -> State<'static> {
        State {
            rect: (4, 8, 1, 20),
            text: "日本語",
            cursor: 9,
            anchor: 3,
            hint: 0,
            purpose: 0,
        }
    }

    fn record<'a>(
        sync: &mut TextInputSync,
        context: u32,
        serial: u32,
        state: State<'a>,
    ) -> Vec<Request<'a>> {
        let mut sent = Vec::new();
        sync.synchronize(context, serial, state, |request| {
            sent.push(request);
            true
        });
        sent
    }

    #[test]
    fn unchanged_editor_sends_no_protocol_messages_or_serial_commit() {
        let mut sync = TextInputSync::default();
        assert_eq!(
            record(&mut sync, 7, 11, state()),
            [
                Request::CursorRect(7, (4, 8, 1, 20)),
                Request::SurroundingText(7, 9, 3, "日本語"),
                Request::ContentType(7, 0, 0),
                Request::Commit(7, 11),
            ]
        );
        for _ in 0..100 {
            assert!(record(&mut sync, 7, 12, state()).is_empty());
        }
    }

    #[test]
    fn each_wire_field_and_new_context_publish_a_fresh_transaction() {
        let mut sync = TextInputSync::default();
        let original = state();
        let mut cases = [original; 9];
        cases[0].rect.0 += 1;
        cases[1].rect.1 += 1;
        cases[2].rect.2 *= 2; // Includes output-scale / caret-size changes.
        cases[3].rect.3 *= 2;
        cases[4].text = "日本語x";
        cases[5].cursor = 6;
        cases[6].anchor = 0;
        cases[7].hint = 1;
        cases[8].purpose = 2;
        for changed in cases {
            sync.invalidate();
            record(&mut sync, 7, 11, original);
            let sent = record(&mut sync, 7, 12, changed);
            assert_eq!(sent.len(), 4);
            assert_eq!(sent.last(), Some(&Request::Commit(7, 12)));
            assert!(record(&mut sync, 7, 13, changed).is_empty());
        }
        assert_eq!(record(&mut sync, 8, 1, original).len(), 4);
    }

    #[test]
    fn focus_release_reacquire_and_reset_republish_identical_state() {
        let mut sync = TextInputSync::default();
        record(&mut sync, 7, 11, state());
        sync.invalidate(); // Production focus loss/None, scale, or transport reset.
        assert_eq!(record(&mut sync, 7, 12, state()).len(), 4);
        assert!(record(&mut sync, 7, 13, state()).is_empty());
    }

    #[test]
    fn partial_write_or_failed_commit_never_caches_unsent_state() {
        for fail_at in 0..4 {
            let mut sync = TextInputSync::default();
            record(&mut sync, 7, 10, state());
            let mut changed = state();
            changed.text = "変更";
            let mut attempts = 0;
            assert_eq!(
                sync.synchronize(7, 11, changed, |_| {
                    let success = attempts != fail_at;
                    attempts += 1;
                    success
                }),
                SyncResult::Failed
            );
            assert_eq!(attempts, fail_at + 1);
            assert_eq!(record(&mut sync, 7, 11, state()).len(), 4);
            assert!(record(&mut sync, 7, 12, state()).is_empty());
        }
    }
}
