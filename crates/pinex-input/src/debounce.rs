//! Turning noisy button levels into press events.
//!
//! Pure logic over samples and timestamps, so it is tested without GPIO — which
//! matters, because the failure it exists to prevent is a preset changing on
//! stage when nobody touched anything.
//!
//! # Why not "accept the edge, then debounce"
//!
//! The obvious implementation reports a press the moment a level changes and
//! then ignores changes for a few milliseconds. That is backwards: a *single*
//! noisy sample is enough to fire, and the debounce window only suppresses the
//! bounce that follows. On a floating input it produced phantom presses roughly
//! twice a second.
//!
//! This requires a level to be **stable** for the whole window before believing
//! it. A one-sample glitch never gets through, however often it happens.

use std::time::{Duration, Instant};

/// How long a level must hold before it is believed.
pub const DEFAULT_STABLE_FOR: Duration = Duration::from_millis(25);

/// Debounces a fixed set of buttons.
#[derive(Debug, Clone)]
pub struct Debouncer<const N: usize> {
    /// The last state we believed.
    settled: [bool; N],
    /// The state we are currently waiting to settle.
    candidate: [bool; N],
    /// When `candidate` was first seen.
    candidate_since: Instant,
    stable_for: Duration,
}

impl<const N: usize> Debouncer<N> {
    pub fn new() -> Self {
        Self::with_window(DEFAULT_STABLE_FOR)
    }

    pub fn with_window(stable_for: Duration) -> Self {
        Self {
            settled: [false; N],
            candidate: [false; N],
            candidate_since: Instant::now(),
            stable_for,
        }
    }

    /// Feed a fresh sample, taken at `now`.
    ///
    /// Returns the indices that went from released to pressed, and only once
    /// per press — holding a button does not repeat, which matters when the
    /// action is "load a preset".
    pub fn sample_at(&mut self, levels: [bool; N], now: Instant) -> Vec<usize> {
        if levels != self.candidate {
            // Something moved. Restart the clock; believe nothing yet.
            self.candidate = levels;
            self.candidate_since = now;
            return Vec::new();
        }

        if self.candidate == self.settled {
            return Vec::new();
        }

        if now.duration_since(self.candidate_since) < self.stable_for {
            return Vec::new();
        }

        // Held steady long enough to be real.
        let pressed = (0..N)
            .filter(|&i| self.candidate[i] && !self.settled[i])
            .collect();
        self.settled = self.candidate;
        pressed
    }

    /// Feed a sample taken now.
    pub fn sample(&mut self, levels: [bool; N]) -> Vec<usize> {
        self.sample_at(levels, Instant::now())
    }

    /// Which buttons are currently believed pressed.
    pub fn settled(&self) -> [bool; N] {
        self.settled
    }
}

impl<const N: usize> Default for Debouncer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_millis(25);

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn a_press_held_for_the_window_is_reported_once() {
        let mut d: Debouncer<2> = Debouncer::with_window(WINDOW);
        let t = Instant::now();

        assert_eq!(d.sample_at([false, false], at(t, 0)), Vec::<usize>::new());
        // Pressed, but not yet settled.
        assert_eq!(d.sample_at([true, false], at(t, 1)), Vec::<usize>::new());
        assert_eq!(d.sample_at([true, false], at(t, 20)), Vec::<usize>::new());
        // Now it has held long enough.
        assert_eq!(d.sample_at([true, false], at(t, 30)), vec![0]);
        // Still held: no repeat.
        assert_eq!(d.sample_at([true, false], at(t, 60)), Vec::<usize>::new());
        assert_eq!(d.sample_at([true, false], at(t, 500)), Vec::<usize>::new());
    }

    /// The bug this module exists for. A single noisy sample must never fire.
    #[test]
    fn a_one_sample_glitch_never_fires() {
        let mut d: Debouncer<3> = Debouncer::with_window(WINDOW);
        let t = Instant::now();

        d.sample_at([false, false, false], at(t, 0));
        for tick in 0..200u64 {
            // A pin flickers low for one sample every 10ms, forever.
            let noisy = tick % 10 == 0;
            let fired = d.sample_at([noisy, false, false], at(t, tick * 5));
            assert!(
                fired.is_empty(),
                "a flickering input fired at tick {tick}: {fired:?}"
            );
        }
    }

    /// Bounce on a genuine press: the level chatters, then settles pressed.
    /// Exactly one press should come out.
    #[test]
    fn contact_bounce_produces_exactly_one_press() {
        let mut d: Debouncer<1> = Debouncer::with_window(WINDOW);
        let t = Instant::now();
        let mut total = 0;

        // 5 ms of chatter at 1 ms intervals.
        for ms in 0..5u64 {
            total += d.sample_at([ms % 2 == 0], at(t, ms)).len();
        }
        // Then held down.
        for ms in 5..80u64 {
            total += d.sample_at([true], at(t, ms)).len();
        }

        assert_eq!(total, 1, "bounce should collapse to a single press");
    }

    #[test]
    fn releasing_reports_nothing_and_re_arms_the_button() {
        let mut d: Debouncer<1> = Debouncer::with_window(WINDOW);
        let t = Instant::now();

        d.sample_at([true], at(t, 0));
        assert_eq!(d.sample_at([true], at(t, 30)), vec![0]);

        // Release settles without reporting a press.
        d.sample_at([false], at(t, 40));
        assert_eq!(d.sample_at([false], at(t, 70)), Vec::<usize>::new());
        assert_eq!(d.settled(), [false]);

        // A second press is reported again.
        d.sample_at([true], at(t, 80));
        assert_eq!(d.sample_at([true], at(t, 110)), vec![0]);
    }

    /// Two buttons pressed together must both be reported, not just the first.
    #[test]
    fn simultaneous_presses_are_all_reported() {
        let mut d: Debouncer<3> = Debouncer::with_window(WINDOW);
        let t = Instant::now();

        d.sample_at([true, false, true], at(t, 0));
        assert_eq!(d.sample_at([true, false, true], at(t, 30)), vec![0, 2]);
    }
}
