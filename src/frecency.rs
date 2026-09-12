//! Event-clock-based exponentially decaying frecency.
//!
//! This module independently implements the scoring model used by ze. Each
//! recorded visit advances a global event clock by one tick. Scores therefore
//! decay only as navigation events occur, rather than while the shell is idle.

/// Default exponential decay constant, in inverse navigation events. Same as ze.
pub(crate) const DEFAULT_LAMBDA: f64 = 8e-3;

/// Per-directory state sufficient to update and rank an exponential moving sum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Record {
    /// Total number of recorded visits, used by frequency-only ranking.
    visits: u64,
    /// Global event-clock tick of the most recent visit, used by recency ranking.
    last_tick: u64,
    /// Exponential moving sum as of `last_tick`.
    score: f64,
}

/// Values that cannot represent a directory's frecency history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    /// A history must contain at least one visit.
    ZeroVisits,
    /// A recorded visit cannot occur before the event clock starts.
    ZeroLastTick,
    /// A directory cannot have more visits than global navigation events.
    VisitsExceedLastTick { visits: u64, last_tick: u64 },
    /// The stored score must be a finite, nonnegative number.
    InvalidScore,
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroVisits => formatter.write_str("record visits must be nonzero"),
            Self::ZeroLastTick => formatter.write_str("record last tick must be nonzero"),
            Self::VisitsExceedLastTick { visits, last_tick } => write!(
                formatter,
                "record visits {visits} exceed its last event-clock tick {last_tick}"
            ),
            Self::InvalidScore => {
                formatter.write_str("record score must be finite and nonnegative")
            }
        }
    }
}

impl std::error::Error for RecordError {}

impl Record {
    /// Creates validated state from a persisted or fixture record.
    pub fn new(visits: u64, last_tick: u64, score: f64) -> Result<Self, RecordError> {
        if visits == 0 {
            return Err(RecordError::ZeroVisits);
        }
        if last_tick == 0 {
            return Err(RecordError::ZeroLastTick);
        }
        if visits > last_tick {
            return Err(RecordError::VisitsExceedLastTick { visits, last_tick });
        }
        if !score.is_finite() || score < 0.0 {
            return Err(RecordError::InvalidScore);
        }
        Ok(Self {
            visits,
            last_tick,
            score,
        })
    }

    /// Returns the total number of recorded visits.
    #[must_use]
    pub const fn visits(self) -> u64 {
        self.visits
    }

    /// Returns the event-clock tick of the most recent visit.
    #[must_use]
    pub const fn last_tick(self) -> u64 {
        self.last_tick
    }

    /// Returns this record's decayed score at `current_tick`.
    #[must_use]
    pub fn score_at(self, current_tick: u64) -> f64 {
        self.score_at_with_lambda(current_tick, DEFAULT_LAMBDA)
    }

    /// Returns the stored score as of [`Self::last_tick`].
    #[must_use]
    pub(crate) const fn stored_score(self) -> f64 {
        self.score
    }

    /// Returns this record's decayed score at `current_tick`.
    #[must_use]
    fn score_at_with_lambda(self, current_tick: u64, lambda: f64) -> f64 {
        assert!(
            current_tick >= self.last_tick,
            "event clock cannot move backwards"
        );
        assert!(
            lambda.is_finite() && lambda > 0.0,
            "lambda must be positive and finite"
        );
        self.score * (-lambda * (current_tick - self.last_tick) as f64).exp()
    }

    /// Records a visit at `tick` and returns the updated state.
    ///
    /// The caller must advance the global event clock before calling this
    /// method, so `tick` is strictly later than every prior event.
    #[must_use]
    pub(crate) fn visit(self, tick: u64) -> Self {
        self.visit_with_lambda(tick, DEFAULT_LAMBDA)
    }

    #[must_use]
    fn visit_with_lambda(self, tick: u64, lambda: f64) -> Self {
        assert!(
            tick > self.last_tick,
            "each visit must advance the event clock"
        );

        Self {
            visits: self.visits + 1,
            last_tick: tick,
            score: self.score_at_with_lambda(tick, lambda) + 1.0,
        }
    }
}

/// Creates state for a directory's first visit at `tick`.
#[must_use]
pub(crate) fn first_visit(tick: u64) -> Record {
    assert!(tick > 0, "event-clock tick must be nonzero");
    Record {
        visits: 1,
        last_tick: tick,
        score: 1.0,
    }
}

/// Returns the number of navigation events needed for a score to halve.
#[cfg(test)]
#[must_use]
fn half_life(lambda: f64) -> f64 {
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
        let record = first_visit(1);
        assert_eq!(record.visits(), 1);
        assert_eq!(record.last_tick(), 1);
        assert_eq!(record.stored_score(), 1.0);
    }

    #[test]
    fn visit_decays_the_previous_score_then_adds_one() {
        let record = Record::new(3, 4, 2.5).unwrap();

        let updated = record.visit(9);

        assert_eq!(updated.visits(), 4);
        assert_eq!(updated.last_tick(), 9);
        assert_close(
            updated.stored_score(),
            2.5 * (-DEFAULT_LAMBDA * 5.0).exp() + 1.0,
        );
    }

    #[test]
    fn consecutive_visits_apply_decay_plus_visit_incrementally() {
        let lambda = 0.5;
        let after_second_visit = first_visit(1).visit_with_lambda(2, lambda);
        let after_third_visit = after_second_visit.visit_with_lambda(3, lambda);
        let after_fourth_visit = after_third_visit.visit_with_lambda(4, lambda);

        let expected_after_second_visit = (-lambda).exp() + 1.0;
        let expected_after_third_visit = expected_after_second_visit * (-lambda).exp() + 1.0;
        let expected_after_fourth_visit = expected_after_third_visit * (-lambda).exp() + 1.0;

        assert_eq!(after_fourth_visit.visits(), 4);
        assert_eq!(after_fourth_visit.last_tick(), 4);
        assert_close(
            after_second_visit.stored_score(),
            expected_after_second_visit,
        );
        assert_close(after_third_visit.stored_score(), expected_after_third_visit);
        assert_close(
            after_fourth_visit.stored_score(),
            expected_after_fourth_visit,
        );
    }

    #[test]
    fn visits_elsewhere_advance_the_clock_and_decay_a_record() {
        let lambda = 0.5;
        let first_directory = first_visit(1);
        let second_directory = first_visit(2)
            .visit_with_lambda(3, lambda)
            .visit_with_lambda(4, lambda);

        assert_eq!(second_directory.last_tick(), 4);
        assert_close(
            first_directory.score_at_with_lambda(second_directory.last_tick(), lambda),
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
            record.score_at(current_tick),
            (-DEFAULT_LAMBDA * elapsed_ticks as f64).exp(),
        );
    }

    #[test]
    fn frequency_and_recency_are_available_without_reconstructing_history() {
        let record = Record::new(42, 99, 3.5).unwrap();

        assert_eq!(record.visits(), 42);
        assert_eq!(record.last_tick(), 99);
    }

    #[test]
    fn construction_rejects_invalid_history_state() {
        assert_eq!(Record::new(0, 1, 1.0), Err(super::RecordError::ZeroVisits));
        assert_eq!(
            Record::new(1, 0, 1.0),
            Err(super::RecordError::ZeroLastTick)
        );
        assert_eq!(
            Record::new(2, 1, 1.0),
            Err(super::RecordError::VisitsExceedLastTick {
                visits: 2,
                last_tick: 1,
            })
        );
        for score in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(
                Record::new(1, 1, score),
                Err(super::RecordError::InvalidScore)
            );
        }
    }

    #[test]
    fn default_lambda_has_expected_half_life() {
        assert_close(half_life(DEFAULT_LAMBDA), 86.64339756999316);
    }
}
