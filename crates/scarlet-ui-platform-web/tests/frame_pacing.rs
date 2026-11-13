#[path = "../src/frame_pacing.rs"]
mod frame_pacing;
use frame_pacing::FramePacing;

#[test]
fn every_requested_120hz_frame_is_granted_once() {
    let pacing = FramePacing::default();
    for timestamp_ms in [0.0, 8.25, 16.5, 24.75] {
        pacing.request();
        assert_eq!(pacing.take_ready(), None, "a request is not a grant");
        pacing.on_animation_frame(timestamp_ms);
        assert_eq!(
            pacing.take_ready(),
            Some((timestamp_ms * 1_000_000.0) as u64)
        );
        assert_eq!(pacing.take_ready(), None, "each rAF grants at most once");
    }
}

#[test]
fn synchronous_polls_do_not_grant_or_consume_a_pending_request() {
    let pacing = FramePacing::default();
    pacing.request();
    for _ in 0..64 {
        assert_eq!(pacing.take_ready(), None);
    }
    pacing.on_animation_frame(8.0);
    assert_eq!(pacing.take_ready(), Some(8_000_000));
}

#[test]
fn idle_frames_and_duplicate_requests_do_not_accumulate_grants() {
    let pacing = FramePacing::default();
    pacing.on_animation_frame(0.0);
    assert_eq!(pacing.take_ready(), None);
    pacing.request();
    pacing.request();
    pacing.on_animation_frame(8.0);
    pacing.request(); // The requested grant is already waiting to be delivered.
    pacing.on_animation_frame(16.0);
    assert_eq!(pacing.take_ready(), Some(8_000_000));
    pacing.on_animation_frame(24.0);
    assert_eq!(pacing.take_ready(), None);
    pacing.request();
    pacing.on_animation_frame(32.0);
    assert_eq!(pacing.take_ready(), Some(32_000_000));
}

#[test]
fn pause_preserves_demand_and_resume_waits_for_a_fresh_frame() {
    for grant_before_pause in [false, true] {
        let pacing = FramePacing::default();
        pacing.request();
        if grant_before_pause {
            pacing.on_animation_frame(8.0);
        }
        pacing.pause();
        pacing.pause();
        assert!(pacing.is_paused());
        pacing.on_animation_frame(16.0);
        assert_eq!(pacing.take_ready(), None);
        pacing.resume();
        pacing.resume();
        assert!(!pacing.is_paused());
        assert_eq!(pacing.take_ready(), None);
        pacing.on_animation_frame(24.0);
        assert_eq!(pacing.take_ready(), Some(24_000_000));
        assert_eq!(pacing.take_ready(), None);
    }
}

#[test]
fn pause_resume_without_demand_does_not_create_a_grant() {
    let pacing = FramePacing::default();
    pacing.pause();
    pacing.resume();
    pacing.on_animation_frame(8.0);
    assert_eq!(pacing.take_ready(), None);
    pacing.pause();
    pacing.request();
    pacing.on_animation_frame(16.0);
    assert_eq!(pacing.take_ready(), None);
    pacing.resume();
    pacing.on_animation_frame(24.0);
    assert_eq!(pacing.take_ready(), Some(24_000_000));
}
