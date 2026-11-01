//! The work queue behind [`crate::run`]: one entry per name, with backoff.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

/// The first retry delay after a failure; it doubles with each failure.
const BACKOFF_BASE: Duration = Duration::from_millis(100);
/// The longest retry delay.
const BACKOFF_MAX: Duration = Duration::from_secs(30);

// shortcut: linear scans over all queued names, fine for the hundreds of
// resources a homelab has; switch to a heap if a kind grows to thousands.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    due: HashMap<String, Instant>,
    failures: HashMap<String, u32>,
}

impl Queue {
    /// Queues `name` at `at`, or earlier if it is already queued earlier.
    pub(crate) fn add(&mut self, name: String, at: Instant) {
        self.due
            .entry(name)
            .and_modify(|due| *due = (*due).min(at))
            .or_insert(at);
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.due.values().min().copied()
    }

    /// Removes and returns the name that has been due longest, if any is due.
    pub(crate) fn pop_due(&mut self, now: Instant) -> Option<String> {
        let name = self
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .min_by_key(|(_, at)| **at)?
            .0
            .clone();
        self.due.remove(&name);
        Some(name)
    }

    pub(crate) fn succeeded(&mut self, name: &str) {
        self.failures.remove(name);
    }

    /// Records a failure and returns how long to wait before retrying.
    pub(crate) fn failed(&mut self, name: &str) -> Duration {
        let failures = self.failures.entry(name.to_owned()).or_insert(0);
        *failures = failures.saturating_add(1);
        BACKOFF_BASE
            .saturating_mul(2u32.saturating_pow(*failures - 1))
            .min(BACKOFF_MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_time_wins_and_names_are_deduplicated() {
        let now = Instant::now();
        let mut q = Queue::default();
        q.add("a".into(), now + Duration::from_secs(10));
        q.add("a".into(), now + Duration::from_secs(1));
        q.add("a".into(), now + Duration::from_secs(5));
        assert_eq!(q.next_deadline(), Some(now + Duration::from_secs(1)));
        assert_eq!(q.pop_due(now), None);
        assert_eq!(q.pop_due(now + Duration::from_secs(1)), Some("a".into()));
        assert_eq!(q.pop_due(now + Duration::from_secs(60)), None);
    }

    #[test]
    fn pops_in_due_order() {
        let now = Instant::now();
        let mut q = Queue::default();
        q.add("late".into(), now + Duration::from_millis(2));
        q.add("early".into(), now + Duration::from_millis(1));
        let later = now + Duration::from_secs(1);
        assert_eq!(q.pop_due(later), Some("early".into()));
        assert_eq!(q.pop_due(later), Some("late".into()));
    }

    #[test]
    fn backoff_doubles_caps_and_resets() {
        let mut q = Queue::default();
        assert_eq!(q.failed("a"), Duration::from_millis(100));
        assert_eq!(q.failed("a"), Duration::from_millis(200));
        assert_eq!(q.failed("a"), Duration::from_millis(400));
        for _ in 0..40 {
            q.failed("a");
        }
        assert_eq!(q.failed("a"), BACKOFF_MAX);
        q.succeeded("a");
        assert_eq!(q.failed("a"), Duration::from_millis(100));
    }
}
