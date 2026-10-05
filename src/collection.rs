#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectionReport {
    errors: u64,
}

impl CollectionReport {
    pub const fn success() -> Self {
        Self { errors: 0 }
    }

    pub const fn error() -> Self {
        Self { errors: 1 }
    }

    pub const fn record_error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }

    pub const fn merge(&mut self, other: Self) {
        self.errors = self.errors.saturating_add(other.errors);
    }

    pub const fn error_count(self) -> u64 {
        self.errors
    }

    pub const fn is_success(self) -> bool {
        self.errors == 0
    }
}

#[cfg(test)]
mod tests {
    use super::CollectionReport;

    #[test]
    fn collection_report_tracks_partial_failures() {
        let mut report = CollectionReport::success();
        assert!(report.is_success());
        assert_eq!(report.error_count(), 0);

        report.record_error();
        report.record_error();
        assert!(!report.is_success());
        assert_eq!(report.error_count(), 2);
    }
}
