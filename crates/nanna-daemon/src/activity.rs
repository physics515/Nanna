//! Daemon activity clock — the "is the system in use right now?" signal that
//! gates the scheduled dream cycle.
//!
//! Dreaming (memory consolidation) competes with the live agent for the
//! summarizer model and rewrites the memory store mid-conversation, so it must
//! only run during a genuine lull. The scheduler reads [`ActivityClock::idle`]
//! and feeds it to the pure `dream_trigger` policy in `nanna-memory`; the
//! control plane stamps [`ActivityClock::record`] on every chat/agent request
//! (and nothing else — a status/log poll must **not** count as activity, or the
//! gate would never open on a GUI that polls once a second).
//!
//! The clock is monotonic (backed by [`Instant`], immune to wall-clock jumps)
//! and lock-free on both the stamp and the read path: a single `AtomicU64`
//! holding milliseconds elapsed from a fixed base instant.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Shared, lock-free record of when the daemon last did real work.
///
/// Construct once at daemon boot, share the `Arc` into both the control plane
/// (which calls [`Self::record`]) and the scheduler (which calls
/// [`Self::idle`]). Cloning the `Arc` shares the same underlying clock.
#[derive(Debug)]
pub struct ActivityClock {
    /// Fixed reference point; all timestamps are milliseconds from here.
    base: Instant,
    /// Milliseconds from `base` at the last [`Self::record`]. Seeded to boot
    /// time so a freshly-booted, never-touched daemon reads as *just active*
    /// (idle ~0), not as idle-forever — dreaming waits for a real lull after
    /// startup rather than firing immediately on boot.
    last_activity_ms: AtomicU64,
}

impl ActivityClock {
    /// Create a clock anchored to now, seeded as active-at-boot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            last_activity_ms: AtomicU64::new(0),
        }
    }

    /// Milliseconds from `base` to now, saturating (never wraps; `base` is in
    /// the past so `elapsed()` only grows).
    #[must_use]
    fn now_ms(&self) -> u64 {
        // u128 → u64: the daemon would have to run ~584 million years for the
        // millisecond count to overflow u64, so the cast is lossless in
        // practice; `min` makes it total regardless.
        self.base.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    /// Stamp "activity happened now". Call from the chat/agent chokepoint only.
    pub fn record(&self) {
        let now = self.now_ms();
        // Monotonic store: never move the timestamp backwards (two racing
        // recorders both see a fresh `now`; keeping the max is stable).
        self.last_activity_ms.fetch_max(now, Ordering::Relaxed);
    }

    /// How long since the last [`Self::record`] (saturating at zero — a clock
    /// stamped "in the future" by a race reads as idle 0, never as a huge
    /// wrapped duration).
    #[must_use]
    pub fn idle(&self) -> Duration {
        let now = self.now_ms();
        let last = self.last_activity_ms.load(Ordering::Relaxed);
        Duration::from_millis(now.saturating_sub(last))
    }
}

impl Default for ActivityClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_clock_reads_near_zero_idle() {
        // A just-constructed clock is "active at boot", not idle-forever.
        let clock = ActivityClock::new();
        assert!(
            clock.idle() < Duration::from_secs(1),
            "a fresh clock must not read as long-idle"
        );
    }

    #[test]
    fn idle_grows_after_a_pause() {
        let clock = ActivityClock::new();
        clock.record();
        std::thread::sleep(Duration::from_millis(30));
        let idle = clock.idle();
        assert!(
            idle >= Duration::from_millis(20),
            "idle should reflect elapsed time since record(); got {idle:?}"
        );
    }

    #[test]
    fn record_resets_idle_to_near_zero() {
        let clock = ActivityClock::new();
        std::thread::sleep(Duration::from_millis(30));
        assert!(clock.idle() >= Duration::from_millis(20));
        clock.record();
        assert!(
            clock.idle() < Duration::from_millis(20),
            "record() must bring idle back near zero"
        );
    }

    #[test]
    fn record_is_monotonic_under_shared_clock() {
        // Cloning the Arc shares one clock; the latest record wins and idle
        // never jumps backwards across shared handles.
        let clock = std::sync::Arc::new(ActivityClock::new());
        let a = clock.clone();
        let b = clock.clone();
        a.record();
        b.record();
        assert!(clock.idle() < Duration::from_millis(20));
    }
}
