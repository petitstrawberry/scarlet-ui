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

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeasePhase {
    GpuPending,
    Ready,
    HandedOff,
}

// One profile per window/worker, shared with the sink's exact lifecycle events.
// Pending means completion has not been observed, not physically executing GPU
// work. HandedOff means socket publication succeeded, not compositor admission.
#[derive(Default)]
struct LeaseProfile {
    live: Vec<(SgfxCommitToken, LeasePhase)>,
    sequence: u64,
    queued: u64,
    completed: u64,
    handed_off: u64,
    released: u64,
    rejected: u64,
    abandoned: u64,
    unmatched: u64,
    peak_live: usize,
    peak_generation_live: usize,
    peak_gpu_pending: usize,
    peak_handed_off: usize,
    overlap_samples: u64,
    pending_grant: Option<(u64, Instant)>,
    credit_requests: u64,
    legacy_requests: u64,
    grants: u64,
}

impl LeaseProfile {
    fn queued(&mut self, token: SgfxCommitToken) {
        if self.live.iter().any(|(existing, _)| *existing == token) {
            self.unmatched += 1;
            return;
        }
        self.live.push((token, LeasePhase::GpuPending));
        self.queued += 1;
    }

    fn transition(&mut self, token: SgfxCommitToken, phase: LeasePhase) {
        let Some((_, current)) = self.live.iter_mut().find(|(t, _)| *t == token) else {
            self.unmatched += 1;
            return;
        };
        let expected = match phase {
            LeasePhase::Ready => LeasePhase::GpuPending,
            LeasePhase::HandedOff => LeasePhase::Ready,
            LeasePhase::GpuPending => unreachable!(),
        };
        if *current != expected {
            self.unmatched += 1;
            return;
        }
        *current = phase;
        match phase {
            LeasePhase::Ready => self.completed += 1,
            LeasePhase::HandedOff => self.handed_off += 1,
            LeasePhase::GpuPending => unreachable!(),
        }
    }

    fn remove(&mut self, token: SgfxCommitToken, event: &str) {
        let Some(index) = self.live.iter().position(|(t, _)| *t == token) else {
            self.unmatched += 1;
            return;
        };
        self.live.swap_remove(index);
        match event {
            "release" => self.released += 1,
            "reject" => self.rejected += 1,
            _ => self.abandoned += 1,
        }
    }

    fn snapshot(&mut self, window: u32, event: &str) -> String {
        self.sequence += 1;
        let gpu = self
            .live
            .iter()
            .filter(|(_, p)| *p == LeasePhase::GpuPending)
            .count();
        let handed = self
            .live
            .iter()
            .filter(|(_, p)| *p == LeasePhase::HandedOff)
            .count();
        let generation_live = self
            .live
            .iter()
            .map(|(token, _)| {
                self.live
                    .iter()
                    .filter(|(other, _)| {
                        other.identity.generation == token.identity.generation
                            && other.identity.compositor_epoch == token.identity.compositor_epoch
                    })
                    .count()
            })
            .max()
            .unwrap_or(0);
        self.peak_live = self.peak_live.max(self.live.len());
        self.peak_generation_live = self.peak_generation_live.max(generation_live);
        self.peak_gpu_pending = self.peak_gpu_pending.max(gpu);
        self.peak_handed_off = self.peak_handed_off.max(handed);
        self.overlap_samples += u64::from(gpu > 0 && handed > 0);
        format!(
            "[UI_LEASE] window={window} seq={} event={event} live={} gpu_pending={gpu} ready={} sws_handed_off={handed} peak_live={} peak_generation_live={} peak_gpu_pending={} peak_sws_handed_off={} overlap_samples={} queued_total={} completed_total={} handed_off_total={} released_total={} rejected_total={} abandoned_total={} unmatched={} credit_requests={} legacy_requests={} grants={}",
            self.sequence,
            self.live.len(),
            self.live.len() - gpu - handed,
            self.peak_live,
            self.peak_generation_live,
            self.peak_gpu_pending,
            self.peak_handed_off,
            self.overlap_samples,
            self.queued,
            self.completed,
            self.handed_off,
            self.released,
            self.rejected,
            self.abandoned,
            self.unmatched,
            self.credit_requests,
            self.legacy_requests,
            self.grants,
        )
    }
}

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
    profile: Option<Mutex<LeaseProfile>>,
}

impl State {
    fn enqueue_frame(
        &self,
        token: SgfxCommitToken,
        send: impl FnOnce() -> SgfxSinkResult<()>,
    ) -> SgfxSinkResult<()> {
        // Count only successful acceptance, before the worker can record
        // completion. No profile lock/allocation is used when profiling is off.
        let mut profile = self
            .profile
            .as_ref()
            .map(|p| p.lock().unwrap_or_else(|e| e.into_inner()));
        send()?;
        let line = profile.as_mut().map(|p| {
            p.queued(token);
            p.snapshot(token.identity.window_id, "queue")
        });
        drop(profile);
        if let Some(line) = line {
            std::println!("{line}");
        }
        Ok(())
    }

    fn publish(&self, action: impl FnOnce() -> Result<(), ()>) -> bool {
        let _guard = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if !self.closed.load(Ordering::Acquire) && !self.failed.load(Ordering::Acquire) {
            if action().is_err() {
                self.failed.store(true, Ordering::Release);
            } else {
                return true;
            }
        }
        false
    }

    fn profile_event(
        &self,
        token: SgfxCommitToken,
        event: &str,
        update: impl FnOnce(&mut LeaseProfile),
    ) {
        if let Some(profile) = &self.profile {
            let mut profile = profile.lock().unwrap_or_else(|e| e.into_inner());
            update(&mut profile);
            let line = profile.snapshot(token.identity.window_id, event);
            drop(profile);
            std::println!("{line}");
        }
    }

    fn close(&self) {
        // Serialize cancellation with socket writes, not with GPU retirement.
        // After close returns no old-epoch publication can start.
        let _guard = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        if !self.closed.swap(true, Ordering::AcqRel) {
            if let Some(profile) = &self.profile {
                let mut p = profile.lock().unwrap_or_else(|e| e.into_inner());
                // Closing invalidates SWS leases; it does not claim GPU work
                // retired. The worker still drains and abandons queued receipts.
                let handed: Vec<_> = p
                    .live
                    .iter()
                    .filter_map(|(t, phase)| (*phase == LeasePhase::HandedOff).then_some(*t))
                    .collect();
                for token in &handed {
                    p.remove(*token, "abandon");
                }
                let line = handed
                    .first()
                    .map(|t| p.snapshot(t.identity.window_id, "close"));
                drop(p);
                if let Some(line) = line {
                    std::println!("{line}");
                }
            }
        }
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
                let mut completed = true;
                // Drain every accepted receipt after failure/close as well.
                // No connection/publication lock is held during GPU waits.
                for submission in &frame.submissions {
                    if !matches!(submission.wait(None), Ok(CompletionStatus::Complete)) {
                        completed = false;
                        state.failed.store(true, Ordering::Release);
                    }
                }
                let retired = Instant::now();
                if completed {
                    state.profile_event(frame.token, "complete", |p| {
                        p.transition(frame.token, LeasePhase::Ready)
                    });
                }
                let published = state.publish(|| {
                    // Serialize profile handoff with release observation. A fast
                    // server can release a token before this socket write returns.
                    let mut profile = state
                        .profile
                        .as_ref()
                        .map(|p| p.lock().unwrap_or_else(|e| e.into_inner()));
                    commit(frame.token, &frame.damage)?;
                    let line = profile.as_mut().map(|p| {
                        p.transition(frame.token, LeasePhase::HandedOff);
                        p.snapshot(frame.token.identity.window_id, "handoff")
                    });
                    drop(profile);
                    if let Some(line) = line {
                        std::println!("{line}");
                    }
                    Ok(())
                });
                if !published {
                    state.profile_event(frame.token, "abandon", |p| {
                        p.remove(frame.token, "abandon")
                    });
                }
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
            Work::Callback { window, id } => {
                state.publish(|| callback(window, id));
            }
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
        let profile = std::env::var("SCARLET_UI_PROFILE").is_ok_and(|value| value == "1");
        let state = Arc::new(State {
            profile: profile.then(|| Mutex::new(LeaseProfile::default())),
            ..State::default()
        });
        let worker_state = state.clone();
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
        self.state.enqueue_frame(token, || {
            self.enqueue(Work::Frame(Frame {
                token,
                damage,
                submissions,
                queued: Instant::now(),
            }))
        })
    }

    pub(super) fn observe_release(&self, token: SgfxCommitToken, rejected: bool) {
        let event = if rejected { "reject" } else { "release" };
        self.state
            .profile_event(token, event, |p| p.remove(token, event));
    }

    pub(super) fn profile_pacing(&self, window: u32, id: u64, credit: bool, grant: bool) {
        if let Some(profile) = &self.state.profile {
            let mut p = profile.lock().unwrap_or_else(|e| e.into_inner());
            let wait_us = if grant {
                p.grants += 1;
                p.pending_grant
                    .take()
                    .filter(|(pending, _)| *pending == id)
                    .map(|(_, start)| start.elapsed().as_micros())
                    .unwrap_or(0)
            } else {
                if credit {
                    p.credit_requests += 1;
                } else {
                    p.legacy_requests += 1;
                }
                p.pending_grant = Some((id, Instant::now()));
                0
            };
            let line = p.snapshot(window, if grant { "grant" } else { "request" });
            drop(p);
            std::println!("{line} callback={id} credit={credit} grant_wait_us={wait_us}");
        }
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

    fn token(slot: u32, generation: u32) -> SgfxCommitToken {
        SgfxCommitToken {
            identity: super::super::SgfxBufferIdentity {
                window_id: 1,
                buffer_id: slot,
                generation,
                compositor_epoch: 1,
            },
            commit_serial: u64::from(generation) * 10 + u64::from(slot),
        }
    }

    #[test]
    fn profile_counts_four_accepted_leases_and_completion_handoff_release_overlap() {
        let state = Arc::new(State {
            profile: Some(Mutex::new(LeaseProfile::default())),
            ..State::default()
        });
        let trace = Trace::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        let (entered, waiting) = sync_channel(4);
        let mut gates = Vec::new();
        for slot in 1..=4 {
            let token = token(slot, 1);
            let (gate, retired) = sync_channel(1);
            gates.push(gate);
            let mut receipt = receipt(&trace, &drops);
            receipt.gate = Some(Mutex::new(retired));
            receipt.entered = Some(entered.clone());
            state
                .enqueue_frame(token, || {
                    sender
                        .try_send(Work::Frame(Frame {
                            token,
                            damage: Vec::new(),
                            submissions: vec![receipt],
                            queued: Instant::now(),
                        }))
                        .map_err(|_| SgfxSinkError::Protocol)
                })
                .unwrap();
        }
        {
            let p = state.profile.as_ref().unwrap().lock().unwrap();
            assert_eq!(
                (
                    p.queued,
                    p.peak_live,
                    p.peak_generation_live,
                    p.peak_gpu_pending
                ),
                (4, 4, 4, 4)
            );
            assert_eq!(p.handed_off, 0);
        }
        let (done, flushed) = sync_channel(1);
        sender.send(Work::Flush(done)).unwrap();
        drop(sender);
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            run_worker(receiver, worker_state, |_, _| Ok(()), |_, _| Ok(()), false)
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        gates[0].send(()).unwrap();
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        {
            let p = state.profile.as_ref().unwrap().lock().unwrap();
            assert_eq!((p.completed, p.handed_off, p.live.len()), (1, 1, 4));
            assert!(
                p.overlap_samples > 0,
                "SWS lease coexists with queued GPU work"
            );
        }
        state.profile_event(token(1, 1), "release", |p| p.remove(token(1, 1), "release"));
        for gate in gates.into_iter().skip(1) {
            gate.send(()).unwrap();
        }
        flushed.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        for slot in 2..=4 {
            state.profile_event(token(slot, 1), "release", |p| {
                p.remove(token(slot, 1), "release")
            });
        }
        let p = state.profile.as_ref().unwrap().lock().unwrap();
        assert_eq!(
            (p.queued, p.completed, p.handed_off, p.released),
            (4, 4, 4, 4)
        );
        assert!(p.live.is_empty());
        assert_eq!(p.unmatched, 0);
        assert_eq!(drops.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn profile_separates_generations_rejections_and_failed_enqueue() {
        let state = State {
            profile: Some(Mutex::new(LeaseProfile::default())),
            ..State::default()
        };
        assert!(
            state
                .enqueue_frame(token(1, 1), || Err(SgfxSinkError::Protocol))
                .is_err()
        );
        let mut p = state.profile.as_ref().unwrap().lock().unwrap();
        assert_eq!((p.queued, p.peak_live), (0, 0));
        for generation in 1..=2 {
            for slot in 1..=4 {
                p.queued(token(slot, generation));
            }
            p.snapshot(1, "queue");
        }
        assert_eq!((p.peak_live, p.peak_generation_live), (8, 4));
        p.remove(token(1, 1), "reject");
        p.remove(token(1, 1), "release"); // Late/duplicate must not free generation 2.
        assert_eq!(
            (p.live.len(), p.rejected, p.released, p.unmatched),
            (7, 1, 0, 1)
        );
        assert!(p.live.iter().any(|(t, _)| *t == token(1, 2)));
        assert_eq!(
            LeaseProfile::default().peak_live,
            0,
            "separate window has no shared peak"
        );
    }

    #[test]
    fn profile_close_invalidates_handoff_but_does_not_claim_gpu_retirement_or_release() {
        let state = State {
            profile: Some(Mutex::new(LeaseProfile::default())),
            ..State::default()
        };
        {
            let mut p = state.profile.as_ref().unwrap().lock().unwrap();
            p.queued(token(1, 1));
            p.queued(token(2, 1));
            p.transition(token(1, 1), LeasePhase::Ready);
            p.transition(token(1, 1), LeasePhase::HandedOff);
        }
        state.close();
        let p = state.profile.as_ref().unwrap().lock().unwrap();
        assert_eq!(
            (p.live.len(), p.completed, p.released, p.abandoned),
            (1, 1, 0, 1)
        );
        assert!(p.live[0].1 == LeasePhase::GpuPending);
        assert!(State::default().profile.is_none());
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
