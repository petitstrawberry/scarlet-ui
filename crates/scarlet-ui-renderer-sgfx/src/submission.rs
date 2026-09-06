//! Bounded frame-level completion tracking for the existing paint encoder.

use alloc::vec::Vec;
use core::cell::Cell;
use core::fmt;

use sgfx::backend::{CommandExecutor, CommandSubmitter, Completion, CompletionStatus, SubmitError};
use sgfx::ir::CommandBuffer;

const MAX_IN_FLIGHT: usize = 16;

/// Failure while asynchronously encoding a frame.
///
/// Earlier command buffers may already have been accepted. Never retry the
/// entire frame as if this error certified side-effect-free rejection.
#[derive(Debug)]
pub enum FrameSubmissionError<E, S> {
    /// The current logical stream was rejected or partially accepted.
    Submit(SubmitError<E, S>),
    /// An earlier accepted stream failed completion observation.
    Completion(E),
    /// A completion could not establish retirement at the admission boundary.
    Pending,
    /// Receipt storage could not be reserved before submitting more work.
    OutOfMemory,
    /// Earlier admission or observation failed; this frame cannot be continued.
    Invalidated,
}

impl<E: fmt::Display, S> fmt::Display for FrameSubmissionError<E, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Submit(SubmitError::Busy) => formatter.write_str("GPU admission is busy"),
            Self::Submit(SubmitError::Rejected(error) | SubmitError::Failed { error, .. }) => {
                write!(formatter, "GPU submission failed: {error}")
            }
            Self::Submit(_) => formatter.write_str("unknown GPU submission failure"),
            Self::Completion(error) => write!(formatter, "GPU completion failed: {error}"),
            Self::Pending => formatter.write_str("GPU work has not retired"),
            Self::OutOfMemory => formatter.write_str("GPU receipt storage exhausted"),
            Self::Invalidated => {
                formatter.write_str("GPU frame was invalidated by an earlier failure")
            }
        }
    }
}

/// Adapt a tracked submitter to the paint encoder without waiting after each submit.
///
/// All accepted receipts are retained until the frame's handoff boundary. Only
/// bounded receipt-capacity pressure may wait earlier. Dropping this owner does
/// not cancel accepted work or authorize shared-image reuse.
pub struct FrameExecutor<E: CommandSubmitter> {
    executor: E,
    submissions: Vec<E::Submission>,
    failed: Cell<bool>,
    retirement_failed: Cell<bool>,
}

impl<E: CommandSubmitter> FrameExecutor<E> {
    /// Begin tracking one frame using a backend-owned executor.
    ///
    /// # Arguments
    ///
    /// * `executor` - Executor bound to the encoder's logical resource table.
    /// Native packet splitting and transport retries belong to the backend
    /// queue. On logical admission pressure this frame may retire its oldest
    /// accepted submission; accepted/failed streams are never replayed.
    ///
    /// # Returns
    ///
    /// A frame-scoped executor with no submitted work yet.
    pub fn new(executor: E) -> Self {
        Self {
            executor,
            submissions: Vec::new(),
            failed: Cell::new(false),
            retirement_failed: Cell::new(false),
        }
    }

    /// Observe every accepted stream without waiting for GPU work.
    ///
    /// # Returns
    ///
    /// Complete only when every stream retired successfully, pending, or an
    /// error. This does not acknowledge SWS release or presentation.
    pub fn poll(&self) -> Result<CompletionStatus, FrameSubmissionError<E::Error, E::Submission>> {
        if self.failed.get() {
            return Err(FrameSubmissionError::Invalidated);
        }
        let mut status = CompletionStatus::Complete;
        for submission in &self.submissions {
            if submission.poll().map_err(|error| {
                self.failed.set(true);
                self.retirement_failed.set(true);
                FrameSubmissionError::Completion(error)
            })? != CompletionStatus::Complete
            {
                status = CompletionStatus::Pending;
            }
        }
        Ok(status)
    }

    /// Retire the frame before handing its images to another queue or process.
    ///
    /// # Returns
    ///
    /// Complete only after all accepted command buffers retired, or an error.
    /// No caller deadline is imposed. Failed/pending observation never permits
    /// image publication or reuse. SWS's separate release is still required.
    pub fn wait(&self) -> Result<CompletionStatus, FrameSubmissionError<E::Error, E::Submission>> {
        if self.failed.get() {
            return Err(FrameSubmissionError::Invalidated);
        }
        for submission in &self.submissions {
            if submission.wait(None).map_err(|error| {
                self.failed.set(true);
                self.retirement_failed.set(true);
                FrameSubmissionError::Completion(error)
            })? != CompletionStatus::Complete
            {
                return Ok(CompletionStatus::Pending);
            }
        }
        Ok(CompletionStatus::Complete)
    }

    /// Discard this frame and retire its accepted prefix without presenting it.
    ///
    /// # Returns
    ///
    /// Complete only after every accepted stream retired successfully, Pending
    /// if retirement is still outstanding, or an error. Busy/rejection and CPU
    /// lowering failure do not prevent this cleanup. A failed-prefix or observed
    /// GPU failure does: no error proves quiescence. The caller must additionally
    /// classify the immediate rejection as recoverable before reusing its backend.
    /// This never replays commands, rolls back GPU writes, or makes the discarded
    /// frame presentable. Subsequent execute/poll/wait calls stay invalidated.
    pub fn discard(
        &self,
    ) -> Result<CompletionStatus, FrameSubmissionError<E::Error, E::Submission>> {
        self.failed.set(true);
        if self.retirement_failed.get() {
            return Err(FrameSubmissionError::Invalidated);
        }
        for submission in &self.submissions {
            if submission.wait(None).map_err(|error| {
                self.retirement_failed.set(true);
                FrameSubmissionError::Completion(error)
            })? != CompletionStatus::Complete
            {
                return Ok(CompletionStatus::Pending);
            }
        }
        Ok(CompletionStatus::Complete)
    }
}

impl<E: CommandSubmitter> CommandExecutor for FrameExecutor<E> {
    type Error = FrameSubmissionError<E::Error, E::Submission>;

    /// Submit one logical stream and retain its completion for the whole frame.
    ///
    /// # Arguments
    ///
    /// * `commands` - Finished commands, including borrowed upload bytes.
    ///
    /// # Returns
    ///
    /// Success after acceptance, not GPU completion. Only proven Busy retries
    /// the current stream. Failure receipts and earlier accepted receipts remain
    /// owned by the error and this executor respectively.
    fn execute<'r, 'data>(
        &mut self,
        commands: &CommandBuffer<'r, 'data>,
    ) -> Result<(), Self::Error> {
        if self.failed.get() {
            return Err(FrameSubmissionError::Invalidated);
        }
        let result = self.submit_commands(commands);
        if result.is_err() {
            self.failed.set(true);
        }
        if !matches!(
            &result,
            Ok(())
                | Err(FrameSubmissionError::OutOfMemory | FrameSubmissionError::Pending)
                | Err(FrameSubmissionError::Submit(
                    SubmitError::Busy | SubmitError::Rejected(_)
                ))
        ) {
            self.retirement_failed.set(true);
        }
        result
    }
}

impl<E: CommandSubmitter> FrameExecutor<E> {
    fn retire_oldest(&mut self) -> Result<(), FrameSubmissionError<E::Error, E::Submission>> {
        if self.submissions[0]
            .wait(None)
            .map_err(FrameSubmissionError::Completion)?
            != CompletionStatus::Complete
        {
            return Err(FrameSubmissionError::Pending);
        }
        self.submissions.remove(0);
        Ok(())
    }

    fn submit_commands(
        &mut self,
        commands: &CommandBuffer<'_, '_>,
    ) -> Result<(), FrameSubmissionError<E::Error, E::Submission>> {
        if self.submissions.len() == MAX_IN_FLIGHT {
            self.retire_oldest()?;
        }
        self.submissions
            .try_reserve(1)
            .map_err(|_| FrameSubmissionError::OutOfMemory)?;
        loop {
            match self.executor.submit(commands) {
                Ok(submission) => {
                    self.submissions.push(submission);
                    return Ok(());
                }
                Err(SubmitError::Busy) if !self.submissions.is_empty() => self.retire_oldest()?,
                Err(error) => return Err(FrameSubmissionError::Submit(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use core::cell::Cell;
    use core::time::Duration;
    use sgfx::ir::{CommandEncoder, ResourceTable};

    #[derive(Debug)]
    struct Receipt {
        waits: Rc<Cell<usize>>,
        fail: Cell<bool>,
        pending: Cell<bool>,
    }
    impl Completion for Receipt {
        type Error = &'static str;
        fn poll(&self) -> Result<CompletionStatus, Self::Error> {
            if self.fail.get() {
                return Err("observation failed");
            }
            Ok(CompletionStatus::Pending)
        }
        fn wait(&self, _: Option<Duration>) -> Result<CompletionStatus, Self::Error> {
            self.waits.set(self.waits.get() + 1);
            if self.fail.get() {
                return Err("observation failed");
            }
            if self.pending.get() {
                return Ok(CompletionStatus::Pending);
            }
            Ok(CompletionStatus::Complete)
        }
    }
    struct Submitter {
        calls: usize,
        waits: Rc<Cell<usize>>,
        busy: bool,
        fail: bool,
    }
    impl CommandExecutor for Submitter {
        type Error = &'static str;
        fn execute<'r, 'data>(&mut self, _: &CommandBuffer<'r, 'data>) -> Result<(), Self::Error> {
            panic!("must not use synchronous execution")
        }
    }
    impl CommandSubmitter for Submitter {
        type Submission = Receipt;
        fn submit<'r, 'data>(
            &mut self,
            _: &CommandBuffer<'r, 'data>,
        ) -> Result<Receipt, SubmitError<Self::Error, Receipt>> {
            self.calls += 1;
            if core::mem::take(&mut self.busy) {
                return Err(SubmitError::Busy);
            }
            let receipt = Receipt {
                waits: Rc::clone(&self.waits),
                fail: Cell::new(false),
                pending: Cell::new(false),
            };
            if self.fail {
                return Err(SubmitError::Failed {
                    error: "partial",
                    completion: receipt,
                });
            }
            Ok(receipt)
        }
    }
    #[test]
    fn submits_multiple_streams_before_waiting_at_the_frame_boundary() {
        let waits = Rc::new(Cell::new(0));
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let mut frame = FrameExecutor::new(Submitter {
            calls: 0,
            waits: waits.clone(),
            busy: false,
            fail: false,
        });
        for _ in 0..3 {
            frame.execute(&commands).unwrap();
        }
        assert_eq!(waits.get(), 0);
        assert_eq!(frame.poll().unwrap(), CompletionStatus::Pending);
        assert_eq!(frame.wait().unwrap(), CompletionStatus::Complete);
        assert_eq!(waits.get(), 3);
    }
    #[test]
    fn retries_only_busy_and_never_replays_partial_acceptance() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let waits = Rc::new(Cell::new(0));
        let mut frame = FrameExecutor::new(Submitter {
            calls: 0,
            waits: Rc::clone(&waits),
            busy: false,
            fail: false,
        });
        frame.execute(&commands).unwrap();
        frame.executor.busy = true;
        frame.executor.fail = true;
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Submit(SubmitError::Failed { .. }))
        ));
        assert_eq!(frame.executor.calls, 3);
        assert!(matches!(
            frame.wait(),
            Err(FrameSubmissionError::Invalidated)
        ));
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Invalidated)
        ));
        assert_eq!(frame.executor.calls, 3);
        assert_eq!(waits.get(), 1);
    }
    #[test]
    fn busy_without_owned_work_does_not_start_a_transport_retry_loop() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let waits = Rc::new(Cell::new(0));
        let mut frame = FrameExecutor::new(Submitter {
            calls: 0,
            waits: Rc::clone(&waits),
            busy: true,
            fail: false,
        });
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Submit(SubmitError::Busy))
        ));
        assert_eq!(frame.executor.calls, 1);
        assert_eq!(waits.get(), 0);
    }

    #[test]
    fn receipt_capacity_waits_only_when_the_bounded_pool_is_full() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let waits = Rc::new(Cell::new(0));
        let mut frame = FrameExecutor::new(Submitter {
            calls: 0,
            waits: waits.clone(),
            busy: false,
            fail: false,
        });
        for _ in 0..MAX_IN_FLIGHT {
            frame.execute(&commands).unwrap();
        }
        assert_eq!(waits.get(), 0);
        frame.execute(&commands).unwrap();
        assert_eq!(waits.get(), 1);
        assert_eq!(frame.submissions.len(), MAX_IN_FLIGHT);
    }

    #[test]
    fn failed_poll_or_wait_permanently_invalidates_the_frame() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        for use_wait in [false, true] {
            let mut frame = FrameExecutor::new(Submitter {
                calls: 0,
                waits: Rc::new(Cell::new(0)),
                busy: false,
                fail: false,
            });
            frame.execute(&commands).unwrap();
            frame.submissions[0].fail.set(true);
            let result = if use_wait { frame.wait() } else { frame.poll() };
            assert!(matches!(
                result,
                Err(FrameSubmissionError::Completion("observation failed"))
            ));
            frame.submissions[0].fail.set(false);
            assert!(matches!(
                frame.poll(),
                Err(FrameSubmissionError::Invalidated)
            ));
            assert!(matches!(
                frame.wait(),
                Err(FrameSubmissionError::Invalidated)
            ));
            assert!(matches!(
                frame.execute(&commands),
                Err(FrameSubmissionError::Invalidated)
            ));
            assert_eq!(frame.executor.calls, 1);
        }
    }

    #[test]
    fn capacity_retirement_failure_never_submits_more_work() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let mut frame = FrameExecutor::new(Submitter {
            calls: 0,
            waits: Rc::new(Cell::new(0)),
            busy: false,
            fail: false,
        });
        for _ in 0..MAX_IN_FLIGHT {
            frame.execute(&commands).unwrap();
        }
        frame.submissions[0].fail.set(true);
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Completion("observation failed"))
        ));
        assert_eq!(frame.executor.calls, MAX_IN_FLIGHT);
        assert_eq!(frame.submissions.len(), MAX_IN_FLIGHT);
        assert!(matches!(
            frame.wait(),
            Err(FrameSubmissionError::Invalidated)
        ));
    }

    struct RejectAfterOne(Submitter);

    impl CommandExecutor for RejectAfterOne {
        type Error = &'static str;
        fn execute<'r, 'data>(&mut self, _: &CommandBuffer<'r, 'data>) -> Result<(), Self::Error> {
            panic!("tracked path required")
        }
    }

    impl CommandSubmitter for RejectAfterOne {
        type Submission = Receipt;
        fn submit<'r, 'data>(
            &mut self,
            commands: &CommandBuffer<'r, 'data>,
        ) -> Result<Receipt, SubmitError<Self::Error, Receipt>> {
            if self.0.calls == 1 {
                self.0.calls += 1;
                return Err(SubmitError::Rejected("submission too large"));
            }
            self.0.submit(commands)
        }
    }

    #[test]
    fn rejection_discards_and_retires_the_prefix_without_replay_then_allows_a_new_frame() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let waits = Rc::new(Cell::new(0));
        let mut frame = FrameExecutor::new(RejectAfterOne(Submitter {
            calls: 0,
            waits: waits.clone(),
            busy: false,
            fail: false,
        }));
        frame.execute(&commands).unwrap();
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Submit(SubmitError::Rejected(
                "submission too large"
            )))
        ));
        assert!(matches!(
            frame.wait(),
            Err(FrameSubmissionError::Invalidated)
        ));
        assert_eq!(frame.discard().unwrap(), CompletionStatus::Complete);
        assert_eq!(waits.get(), 1);
        assert!(matches!(
            frame.execute(&commands),
            Err(FrameSubmissionError::Invalidated)
        ));
        assert_eq!(frame.executor.0.calls, 2);
        let mut next = FrameExecutor::new(frame.executor);
        next.execute(&commands).unwrap();
        assert_eq!(next.wait().unwrap(), CompletionStatus::Complete);
        assert_eq!(next.executor.0.calls, 3);
    }

    #[test]
    fn discard_requires_successful_retirement_even_after_side_effect_free_rejection() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        let mut frame = FrameExecutor::new(RejectAfterOne(Submitter {
            calls: 0,
            waits: Rc::new(Cell::new(0)),
            busy: false,
            fail: false,
        }));
        frame.execute(&commands).unwrap();
        assert!(frame.execute(&commands).is_err());
        frame.submissions[0].pending.set(true);
        assert_eq!(frame.discard().unwrap(), CompletionStatus::Pending);
        frame.submissions[0].pending.set(false);
        frame.submissions[0].fail.set(true);
        assert!(matches!(
            frame.discard(),
            Err(FrameSubmissionError::Completion(_))
        ));
        frame.submissions[0].fail.set(false);
        assert!(matches!(
            frame.discard(),
            Err(FrameSubmissionError::Invalidated)
        ));
    }

    #[test]
    fn discarded_lowering_failure_retires_work_but_failed_prefix_never_certifies_recovery() {
        let table = ResourceTable::new();
        let commands = CommandEncoder::new(&table).finish().unwrap();
        for partial in [false, true] {
            let mut frame = FrameExecutor::new(Submitter {
                calls: 0,
                waits: Rc::new(Cell::new(0)),
                busy: false,
                fail: false,
            });
            frame.execute(&commands).unwrap();
            if partial {
                frame.executor.fail = true;
                assert!(frame.execute(&commands).is_err());
                assert!(matches!(
                    frame.discard(),
                    Err(FrameSubmissionError::Invalidated)
                ));
            } else {
                assert_eq!(frame.discard().unwrap(), CompletionStatus::Complete);
            }
            assert!(matches!(
                frame.wait(),
                Err(FrameSubmissionError::Invalidated)
            ));
        }
    }
}
