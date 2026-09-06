//! Event-clock-based exponentially decaying frecency.
//!
//! This module independently implements the scoring model used by ze. Each
//! recorded visit advances a global event clock by one tick. Scores therefore
//! decay only as navigation events occur, rather than while the shell is idle.

/// default exponential decay constant, in inverse navigation events. Same as ze.
pub const DEFAULT_LAMBDA: f64 = 8e-3;

/// Per-directory state sufficient to update and rank an exponential moving sum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Record {
    /// Total number of recorded visits, used by frequency-only ranking.
    pub visits: u64,
    /// Global event-clock tick of the most recent visit, used by recency ranking.
    pub last_tick: u64,
    /// Exponential moving sum as of `last_tick`.
    pub score: f64,
}

impl Record {
    /// Returns this record's decayed score at `current_tick`.
    ///
    /// `lambda` is expected to be positive and finite.
    #[must_use]
    pub fn score_at(self, current_tick: u64, lambda: f64) -> f64 {
        assert!(
            current_tick >= self.last_tick,
            "event clock cannot move backwards"
        );
        self.score * (-lambda * (current_tick - self.last_tick) as f64).exp()
    }

    /// Records a visit at `tick` and returns the updated state.
    ///
    /// The caller must advance the global event clock before calling this
    /// method, so `tick` is strictly later than every prior event.
    /// `lambda` is expected to be positive and finite.
    #[must_use]
    pub fn visit(self, tick: u64, lambda: f64) -> Self {
        assert!(
            tick > self.last_tick,
            "each visit must advance the event clock"
        );

        Self {
            visits: self.visits + 1,
            last_tick: tick,
            score: self.score_at(tick, lambda) + 1.0,
        }
    }
}

/// Creates state for a directory's first visit at `tick`.
#[must_use]
pub fn first_visit(tick: u64) -> Record {
    assert!(tick > 0, "event-clock tick must be nonzero");
    Record {
        visits: 1,
        last_tick: tick,
        score: 1.0,
    }
}

/// Returns the number of navigation events needed for a score to halve.
#[must_use]
pub fn half_life(lambda: f64) -> f64 {
    assert!(
        lambda.is_finite() && lambda > 0.0,
        "lambda must be positive and finite"
    );
    std::f64::consts::LN_2 / lambda
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::{DEFAULT_LAMBDA, Record, first_visit, half_life};

    const EPSILON: f64 = 1e-12;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn first_visit_has_one_visit_and_a_unit_score() {
        assert_eq!(
            first_visit(1),
            Record {
                visits: 1,
                last_tick: 1,
                score: 1.0,
            }
        );
    }

    #[test]
    fn visit_decays_the_previous_score_then_adds_one() {
        let record = Record {
            visits: 3,
            last_tick: 4,
            score: 2.5,
        };

        let updated = record.visit(9, DEFAULT_LAMBDA);

        assert_eq!(updated.visits, 4);
        assert_eq!(updated.last_tick, 9);
        assert_close(updated.score, 2.5 * (-DEFAULT_LAMBDA * 5.0).exp() + 1.0);
    }

    #[test]
    fn consecutive_visits_apply_decay_plus_visit_incrementally() {
        let lambda = 0.5;
        let after_second_visit = first_visit(1).visit(2, lambda);
        let after_third_visit = after_second_visit.visit(3, lambda);
        let after_fourth_visit = after_third_visit.visit(4, lambda);

        let expected_after_second_visit = (-lambda).exp() + 1.0;
        let expected_after_third_visit = expected_after_second_visit * (-lambda).exp() + 1.0;
        let expected_after_fourth_visit = expected_after_third_visit * (-lambda).exp() + 1.0;

        assert_eq!(after_fourth_visit.visits, 4);
        assert_eq!(after_fourth_visit.last_tick, 4);
        assert_close(after_second_visit.score, expected_after_second_visit);
        assert_close(after_third_visit.score, expected_after_third_visit);
        assert_close(after_fourth_visit.score, expected_after_fourth_visit);
    }

    #[test]
    fn visits_elsewhere_advance_the_clock_and_decay_a_record() {
        let lambda = 0.5;
        let first_directory = first_visit(1);
        let second_directory = first_visit(2).visit(3, lambda).visit(4, lambda);

        assert_eq!(second_directory.last_tick, 4);
        assert_close(
            first_directory.score_at(second_directory.last_tick, lambda),
            (-1.5_f64).exp(),
        );
    }

    #[rstest]
    #[case(10, 0)]
    #[case(20, 10)]
    #[case(100, 90)]
    fn current_score_reflects_elapsed_navigation_events(
        #[case] current_tick: u64,
        #[case] elapsed_ticks: u64,
    ) {
        let record = first_visit(10);

        assert_close(
            record.score_at(current_tick, DEFAULT_LAMBDA),
            (-DEFAULT_LAMBDA * elapsed_ticks as f64).exp(),
        );
    }

    #[test]
    fn frequency_and_recency_are_available_without_reconstructing_history() {
        let record = Record {
            visits: 42,
            last_tick: 99,
            score: 3.5,
        };

        assert_eq!(record.visits, 42);
        assert_eq!(record.last_tick, 99);
    }

    #[test]
    fn default_lambda_has_expected_half_life() {
        assert_close(half_life(DEFAULT_LAMBDA), 86.64339756999316);
    }
}
