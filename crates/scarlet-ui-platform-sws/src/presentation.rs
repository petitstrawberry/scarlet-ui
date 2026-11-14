//! Ordered, bounded GPU completion and SWS handoff off the UI event thread.
//!
//! GPU resources remain owned by accepted submissions. SWS retention separately
//! prevents reuse of presented targets. Legacy frame callbacks remain ordered
//! after commits; independently negotiated draw credits bypass this worker.

use super::{SgfxCommitToken, SgfxSinkError, SgfxSinkResult, SwsSgfxFrameSink, sws};
use sgfx::backend::{Completion, CompletionStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// Four image leases, one legacy callback and one resize flush. Resize
// flushes the old generation before registering/admitting any new-generation work.
const QUEUE_CAPACITY: usize = 6;

struct Frame<R> {
    token: SgfxCommitToken,
    damage: Vec<sws::SgfxDamageRect>,
    submissions: Vec<R>,
    queued: Instant,
}

enum Work<R> {
    Frame(Frame<R>),
    Callback { window: u32, id: u64 },
    Flush(SyncSender<()>),
}

#[derive(Default)]
struct State {
    closed: AtomicBool,
    failed: AtomicBool,
    publication: Mutex<()>,
}

impl State {
    fn publish(&self, action: impl FnOnce() -> Result<(), ()>) {
        let _guard = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if !self.closed.load(Ordering::Acquire) && !self.failed.load(Ordering::Acquire) {
            if action().is_err() {
                self.failed.store(true, Ordering::Release);
            }
        }
    }

    fn close(&self) {
        // Serialize cancellation with socket writes, not with GPU retirement.
        // After close returns no old-epoch publication can start.
        let _guard = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        self.closed.store(true, Ordering::Release);
    }

    fn check(&self) -> SgfxSinkResult<()> {
        if self.failed.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire) {
            Err(SgfxSinkError::Protocol)
        } else {
            Ok(())
        }
    }
}

// A panicking worker must wake the sink's polling path instead of leaving all
// four retained slots waiting forever for commits that can no longer arrive.
struct WorkerExit(Arc<State>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.failed.store(true, Ordering::Release);
    }
}

fn run_worker<R: Completion>(
    receiver: Receiver<Work<R>>,
    state: Arc<State>,
    mut commit: impl FnMut(SgfxCommitToken, &[sws::SgfxDamageRect]) -> Result<(), ()>,
    mut callback: impl FnMut(u32, u64) -> Result<(), ()>,
    profile: bool,
) {
    let _exit = WorkerExit(state.clone());
    while let Ok(work) = receiver.recv() {
        match work {
            Work::Frame(frame) => {
                let started = Instant::now();
                // Drain every accepted receipt after failure/close as well.
                // No connection/publication lock is held during GPU waits.
                for submission in &frame.submissions {
                    if !matches!(submission.wait(None), Ok(CompletionStatus::Complete)) {
                        state.failed.store(true, Ordering::Release);
                    }
                }
                let retired = Instant::now();
                state.publish(|| commit(frame.token, &frame.damage));
                if profile && state.check().is_ok() {
                    std::println!(
                        "[UI_PRESENT] window={} serial={} queue_us={} completion_wait_us={} handoff_us={}",
                        frame.token.identity.window_id,
                        frame.token.commit_serial,
                        started.duration_since(frame.queued).as_micros(),
                        retired.duration_since(started).as_micros(),
                        retired.elapsed().as_micros(),
                    );
                }
            }
            Work::Callback { window, id } => state.publish(|| callback(window, id)),
            Work::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

pub(super) struct FrameHandoff {
    sender: SyncSender<Work<sgfx::Submission>>,
    state: Arc<State>,
}

impl FrameHandoff {
    pub(super) fn new(conn: sws::Connection) -> SgfxSinkResult<Self> {
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        let state = Arc::new(State::default());
        let worker_state = state.clone();
        let profile = std::env::var("SCARLET_UI_PROFILE").is_ok_and(|value| value == "1");
        std::thread::Builder::new()
            .name("sgfx-present".into())
            .spawn(move || {
                run_worker(
                    receiver,
                    worker_state,
                    |token, damage| {
                        conn.commit_sgfx_frame(
                            SwsSgfxFrameSink::client_identity(token.identity),
                            token.commit_serial,
                            damage,
                        )
                        .map_err(|_| ())
                    },
                    |window, id| conn.request_frame(window, id).map_err(|_| ()),
                    profile,
                )
            })
            .map_err(|_| SgfxSinkError::Protocol)?;
        Ok(Self { sender, state })
    }

    fn enqueue(&self, work: Work<sgfx::Submission>) -> SgfxSinkResult<()> {
        self.state.check()?;
        self.sender.try_send(work).map_err(|_| {
            // A failed handoff may follow accepted GPU writes. Poison the
            // owner; never advertise this target as reusable for another frame.
            self.state.failed.store(true, Ordering::Release);
            SgfxSinkError::Protocol
        })
    }

    pub(super) fn send(
        &self,
        token: SgfxCommitToken,
        damage: Vec<sws::SgfxDamageRect>,
        submissions: Vec<sgfx::Submission>,
    ) -> SgfxSinkResult<()> {
        self.enqueue(Work::Frame(Frame {
            token,
            damage,
            submissions,
            queued: Instant::now(),
        }))
    }

    pub(super) fn request_frame(&self, window: u32, id: u64) -> SgfxSinkResult<()> {
        self.enqueue(Work::Callback { window, id })
    }

    // Only generation changes need a UI-thread barrier. This is GPU retirement
    // and socket publication, not SWS release (which needs the replacement frame).
    pub(super) fn flush(&self) -> SgfxSinkResult<()> {
        let (sender, receiver) = sync_channel(1);
        self.enqueue(Work::Flush(sender))?;
        receiver.recv().map_err(|_| SgfxSinkError::Protocol)?;
        self.state.check()
    }

    pub(super) fn failed(&self) -> bool {
        self.state.failed.load(Ordering::Acquire)
    }
    pub(super) fn close(&self) {
        self.state.close();
    }
}

impl Drop for FrameHandoff {
    fn drop(&mut self) {
        self.close();
        // Dropping the sole sender drains accepted GPU work and exits. The
        // platform window holds only a Weak, so it cannot keep a dead worker alive.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    type Trace = Arc<Mutex<Vec<&'static str>>>;
    struct Receipt {
        trace: Trace,
        gate: Option<Mutex<Receiver<()>>>,
        entered: Option<SyncSender<()>>,
        failed: bool,
        drops: Arc<AtomicUsize>,
    }
    impl Completion for Receipt {
        type Error = ();
        fn poll(&self) -> Result<CompletionStatus, ()> {
            Ok(CompletionStatus::Pending)
        }
        fn wait(&self, _: Option<Duration>) -> Result<CompletionStatus, ()> {
            self.trace.lock().unwrap().push("wait");
            if let Some(entered) = &self.entered {
                entered.send(()).unwrap();
            }
            if let Some(gate) = &self.gate {
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
            }
            if self.failed {
                Err(())
            } else {
                Ok(CompletionStatus::Complete)
            }
        }
    }
    impl Drop for Receipt {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn receipt(trace: &Trace, drops: &Arc<AtomicUsize>) -> Receipt {
        Receipt {
            trace: trace.clone(),
            gate: None,
            entered: None,
            failed: false,
            drops: drops.clone(),
        }
    }
    fn frame(submissions: Vec<Receipt>) -> Work<Receipt> {
        Work::Frame(Frame {
            token: SgfxCommitToken {
                identity: super::super::SgfxBufferIdentity {
                    window_id: 1,
                    buffer_id: 1,
                    generation: 1,
                    compositor_epoch: 1,
                },
                commit_serial: 1,
            },
            damage: Vec::new(),
            submissions,
            queued: Instant::now(),
        })
    }

    #[test]
    fn completion_commit_callback_and_resize_flush_are_ordered() {
        let trace = Trace::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let (gate, retired) = sync_channel(1);
        let (entered, waiting) = sync_channel(1);
        let mut first = receipt(&trace, &drops);
        first.gate = Some(Mutex::new(retired));
        first.entered = Some(entered);
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        sender
            .send(frame(vec![first, receipt(&trace, &drops)]))
            .unwrap();
        sender.send(Work::Callback { window: 1, id: 2 }).unwrap();
        let (done, flushed) = sync_channel(1);
        sender.send(Work::Flush(done)).unwrap();
        let state = Arc::new(State::default());
        let commits = trace.clone();
        let callbacks = trace.clone();
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            run_worker(
                receiver,
                worker_state,
                |_, _| {
                    commits.lock().unwrap().push("commit");
                    Ok(())
                },
                |_, _| {
                    callbacks.lock().unwrap().push("callback");
                    Ok(())
                },
                false,
            )
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(*trace.lock().unwrap(), ["wait"]);
        assert_eq!(
            drops.load(Ordering::Relaxed),
            0,
            "pending receipts remain owned"
        );
        assert!(
            flushed.try_recv().is_err(),
            "new-generation registration must wait"
        );
        gate.send(()).unwrap();
        flushed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            *trace.lock().unwrap(),
            ["wait", "wait", "commit", "callback"]
        );
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        assert!(state.check().is_ok());
        drop(sender);
        worker.join().unwrap();
    }

    #[test]
    fn close_or_completion_failure_drains_tail_without_publication() {
        for close in [false, true] {
            let trace = Trace::default();
            let drops = Arc::new(AtomicUsize::new(0));
            let state = Arc::new(State::default());
            if close {
                state.close();
            }
            let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
            let mut first = receipt(&trace, &drops);
            first.failed = !close;
            sender
                .send(frame(vec![first, receipt(&trace, &drops)]))
                .unwrap();
            sender.send(Work::Callback { window: 1, id: 2 }).unwrap();
            sender.send(frame(vec![receipt(&trace, &drops)])).unwrap();
            let (done, flushed) = sync_channel(1);
            sender.send(Work::Flush(done)).unwrap();
            drop(sender);
            run_worker(
                receiver,
                state,
                |_, _| panic!("must not commit"),
                |_, _| panic!("must not request a presentation callback"),
                false,
            );
            flushed.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(*trace.lock().unwrap(), ["wait", "wait", "wait"]);
            assert_eq!(drops.load(Ordering::Relaxed), 3);
        }
    }

    #[test]
    fn socket_failure_suppresses_later_commits_but_retires_their_receipts() {
        let trace = Trace::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(State::default());
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        for _ in 0..2 {
            sender.send(frame(vec![receipt(&trace, &drops)])).unwrap();
        }
        sender.send(Work::Callback { window: 1, id: 2 }).unwrap();
        drop(sender);
        run_worker(
            receiver,
            state.clone(),
            |_, _| {
                trace.lock().unwrap().push("failed commit");
                Err(())
            },
            |_, _| panic!("callback after failure"),
            false,
        );
        assert_eq!(*trace.lock().unwrap(), ["wait", "failed commit", "wait"]);
        assert!(state.failed.load(Ordering::Acquire));
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn worker_panic_is_observable_and_disconnects_resize_flush() {
        let state = Arc::new(State::default());
        let worker_state = state.clone();
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        sender.send(frame(vec![])).unwrap();
        let (done, flushed) = sync_channel(1);
        sender.send(Work::Flush(done)).unwrap();
        let worker = std::thread::spawn(move || {
            run_worker(
                receiver,
                worker_state,
                |_, _| panic!("simulated worker failure"),
                |_, _| Ok(()),
                false,
            )
        });
        assert!(worker.join().is_err());
        assert!(state.failed.load(Ordering::Acquire));
        assert!(flushed.recv_timeout(Duration::from_secs(2)).is_err());
    }

    #[test]
    fn epoch_close_serializes_with_publication_without_waiting_on_gpu_work() {
        let state = Arc::new(State::default());
        let (started, publishing) = sync_channel(1);
        let (finish, finishing) = sync_channel(1);
        let publish_state = state.clone();
        let publisher = std::thread::spawn(move || {
            publish_state.publish(|| {
                started.send(()).unwrap();
                finishing.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(())
            })
        });
        publishing.recv_timeout(Duration::from_secs(2)).unwrap();
        let (done, closed) = sync_channel(1);
        let close_state = state.clone();
        let closer = std::thread::spawn(move || {
            close_state.close();
            done.send(()).unwrap();
        });
        assert!(closed.recv_timeout(Duration::from_millis(10)).is_err());
        finish.send(()).unwrap();
        closed.recv_timeout(Duration::from_secs(2)).unwrap();
        state.publish(|| panic!("old epoch published after close returned"));
        publisher.join().unwrap();
        closer.join().unwrap();
    }
}
