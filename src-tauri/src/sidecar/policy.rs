//! Restart policy for the sidecar.
//!
//! Pure logic, no processes — restart behaviour is the part most likely to be
//! subtly wrong (thundering restarts, giving up too early, never giving up) and
//! the hardest to observe once it's tangled up with real process handling.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPolicy {
    /// Consecutive failures tolerated before giving up.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RestartPolicy {
    /// Delay before restart attempt `attempt` (1-based), or `None` once the
    /// budget is spent.
    ///
    /// Exponential with a ceiling: a sidecar that crashes instantly on start
    /// would otherwise spin the CPU restarting it, while a genuinely transient
    /// failure still recovers quickly.
    pub fn delay_for(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }
        let factor = 2u32.saturating_pow(attempt - 1);
        let delay = self.base_delay.saturating_mul(factor);
        Some(delay.min(self.max_delay))
    }
}

/// Tracks consecutive failures. A run that stays up long enough to be
/// considered healthy resets the budget, so a sidecar that crashes once an hour
/// isn't eventually refused for having crashed five times over a week.
#[derive(Debug)]
pub struct RestartTracker {
    policy: RestartPolicy,
    consecutive_failures: u32,
    healthy_after: Duration,
}

impl RestartTracker {
    pub fn new(policy: RestartPolicy) -> Self {
        Self {
            policy,
            consecutive_failures: 0,
            healthy_after: Duration::from_secs(10),
        }
    }

    /// Records an exit and returns how long to wait before respawning, or
    /// `None` to give up.
    pub fn record_exit(&mut self, uptime: Duration) -> Option<Duration> {
        if uptime >= self.healthy_after {
            self.consecutive_failures = 0;
        }
        self.consecutive_failures += 1;
        self.policy.delay_for(self.consecutive_failures)
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially() {
        let policy = RestartPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
        };
        assert_eq!(policy.delay_for(1), Some(Duration::from_millis(100)));
        assert_eq!(policy.delay_for(2), Some(Duration::from_millis(200)));
        assert_eq!(policy.delay_for(3), Some(Duration::from_millis(400)));
        assert_eq!(policy.delay_for(4), Some(Duration::from_millis(800)));
    }

    #[test]
    fn backoff_is_capped() {
        let policy = RestartPolicy {
            max_attempts: 20,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
        };
        // Without a ceiling, attempt 20 would be ~14 hours.
        assert_eq!(policy.delay_for(20), Some(Duration::from_secs(1)));
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let policy = RestartPolicy {
            max_attempts: 3,
            ..Default::default()
        };
        assert!(policy.delay_for(3).is_some());
        assert!(policy.delay_for(4).is_none());
    }

    #[test]
    fn attempt_zero_is_not_a_restart() {
        assert!(RestartPolicy::default().delay_for(0).is_none());
    }

    #[test]
    fn crash_looping_sidecar_eventually_gives_up() {
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_attempts: 3,
            ..Default::default()
        });
        // Instant crashes, over and over.
        assert!(tracker.record_exit(Duration::from_millis(5)).is_some());
        assert!(tracker.record_exit(Duration::from_millis(5)).is_some());
        assert!(tracker.record_exit(Duration::from_millis(5)).is_some());
        assert!(
            tracker.record_exit(Duration::from_millis(5)).is_none(),
            "a crash-looping sidecar must not be restarted forever"
        );
    }

    #[test]
    fn a_long_healthy_run_resets_the_budget() {
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_attempts: 2,
            ..Default::default()
        });
        tracker.record_exit(Duration::from_millis(5));
        tracker.record_exit(Duration::from_millis(5));
        assert!(tracker.record_exit(Duration::from_millis(5)).is_none());

        // Same tracker, but this time the sidecar had been up for a while.
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_attempts: 2,
            ..Default::default()
        });
        tracker.record_exit(Duration::from_millis(5));
        assert_eq!(tracker.consecutive_failures(), 1);
        tracker.record_exit(Duration::from_secs(30));
        assert_eq!(
            tracker.consecutive_failures(),
            1,
            "a healthy run should have reset the counter before incrementing"
        );
    }

    #[test]
    fn a_handshake_does_not_refill_the_budget() {
        // A sidecar can reach `Ready` in milliseconds and then die on the very
        // next command. If the handshake reset the budget, that would restart
        // forever. Only sustained uptime counts as healthy.
        let mut tracker = RestartTracker::new(RestartPolicy {
            max_attempts: 2,
            ..Default::default()
        });
        assert!(tracker.record_exit(Duration::from_millis(20)).is_some());
        assert!(tracker.record_exit(Duration::from_millis(20)).is_some());
        assert!(tracker.record_exit(Duration::from_millis(20)).is_none());
    }
}
