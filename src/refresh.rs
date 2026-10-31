use std::time::{Duration, Instant};

pub struct IntervalGate {
    last_attempt: Option<Instant>,
}

impl IntervalGate {
    pub const fn new() -> Self {
        Self { last_attempt: None }
    }

    pub fn due(&mut self, interval: Duration) -> bool {
        self.due_at(interval, Instant::now())
    }

    fn due_at(&mut self, interval: Duration, now: Instant) -> bool {
        if self
            .last_attempt
            .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            return false;
        }
        self.last_attempt = Some(now);
        true
    }
}

pub struct AsyncRefreshSnapshot<T> {
    pub snapshot: Option<T>,
    pub refreshed_unix_seconds: Option<f64>,
    pub refreshing: bool,
    pub should_refresh: bool,
    pub report_pending: bool,
}

pub struct AsyncRefreshCache<T> {
    refreshing: bool,
    refreshed: Option<Instant>,
    refreshed_unix_seconds: Option<f64>,
    snapshot: Option<T>,
    report_pending: bool,
}

impl<T> AsyncRefreshCache<T> {
    pub const fn new() -> Self {
        Self {
            refreshing: false,
            refreshed: None,
            refreshed_unix_seconds: None,
            snapshot: None,
            report_pending: false,
        }
    }

    pub fn finish_refresh(&mut self, snapshot: T, refreshed_unix_seconds: f64) {
        self.finish_refresh_at(snapshot, refreshed_unix_seconds, Instant::now());
    }

    fn finish_refresh_at(&mut self, snapshot: T, refreshed_unix_seconds: f64, refreshed: Instant) {
        self.snapshot = Some(snapshot);
        self.refreshed = Some(refreshed);
        self.refreshed_unix_seconds = Some(refreshed_unix_seconds);
        self.report_pending = true;
        self.refreshing = false;
    }
}

impl<T: Clone> AsyncRefreshCache<T> {
    pub fn begin_scrape(&mut self, interval: Duration) -> AsyncRefreshSnapshot<T> {
        self.begin_scrape_at(interval, Instant::now())
    }

    fn begin_scrape_at(&mut self, interval: Duration, now: Instant) -> AsyncRefreshSnapshot<T> {
        let due = self
            .refreshed
            .is_none_or(|refreshed| now.saturating_duration_since(refreshed) >= interval);
        let should_refresh = due && !self.refreshing;
        if should_refresh {
            self.refreshing = true;
        }
        AsyncRefreshSnapshot {
            snapshot: self.snapshot.clone(),
            refreshed_unix_seconds: self.refreshed_unix_seconds,
            refreshing: self.refreshing,
            should_refresh,
            report_pending: std::mem::take(&mut self.report_pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_gate_is_due_at_exact_boundary() {
        let mut gate = IntervalGate::new();
        let start = Instant::now();
        let interval = Duration::from_secs(30);

        assert!(gate.due_at(interval, start));
        assert!(!gate.due_at(interval, start + Duration::from_secs(29)));
        assert!(gate.due_at(interval, start + interval));
    }

    #[test]
    fn async_refresh_cache_coalesces_and_reports_each_refresh_once() {
        let mut cache = AsyncRefreshCache::<u32>::new();
        let start = Instant::now();
        let interval = Duration::from_secs(30);

        let first = cache.begin_scrape_at(interval, start);
        assert!(first.should_refresh);
        assert!(first.refreshing);
        assert!(first.snapshot.is_none());
        assert!(!first.report_pending);

        let while_running = cache.begin_scrape_at(interval, start + Duration::from_secs(1));
        assert!(!while_running.should_refresh);
        assert!(while_running.refreshing);

        cache.finish_refresh_at(7, 123.0, start + Duration::from_secs(2));

        let completed = cache.begin_scrape_at(interval, start + Duration::from_secs(3));
        assert_eq!(completed.snapshot, Some(7));
        assert_eq!(completed.refreshed_unix_seconds, Some(123.0));
        assert!(!completed.refreshing);
        assert!(!completed.should_refresh);
        assert!(completed.report_pending);

        let repeated = cache.begin_scrape_at(interval, start + Duration::from_secs(4));
        assert_eq!(repeated.snapshot, Some(7));
        assert!(!repeated.report_pending);

        let due = cache.begin_scrape_at(interval, start + Duration::from_secs(32));
        assert!(due.should_refresh);
        assert!(due.refreshing);
        assert_eq!(due.snapshot, Some(7));
    }
}
