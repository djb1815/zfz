//! Canonical ordering for already-matched directory candidates.

use std::{cmp::Ordering, error::Error, fmt};

use crate::frecency::Record;

/// The historical signal used as the primary ranking input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryMode {
    /// Exponentially decayed score at the current event-clock tick.
    Frecency,
    /// Total visit count.
    Frequency,
    /// Most recent event-clock tick.
    Recency,
}

/// Ranking inputs for one already-matched path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate<'a> {
    path: &'a str,
    record: Record,
    fuzzy_score: i64,
}

impl<'a> Candidate<'a> {
    /// Creates a candidate from its preserved path, history, and match quality.
    #[must_use]
    pub const fn new(path: &'a str, record: Record, fuzzy_score: i64) -> Self {
        Self {
            path,
            record,
            fuzzy_score,
        }
    }

    /// Returns the path exactly as supplied by the shell.
    #[must_use]
    pub const fn path(&self) -> &'a str {
        self.path
    }
}

/// An invalid input to canonical ranking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RankingError {
    /// The event clock is earlier than a candidate's most recent visit.
    CurrentTickPrecedesLastVisit { current_tick: u64, last_tick: u64 },
    /// A candidate's selected frecency score is not finite.
    NonFiniteFrecencyScore { score: f64 },
}

impl fmt::Display for RankingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentTickPrecedesLastVisit {
                current_tick,
                last_tick,
            } => write!(
                formatter,
                "current event-clock tick {current_tick} precedes a candidate's last visit at tick {last_tick}"
            ),
            Self::NonFiniteFrecencyScore { score } => {
                write!(
                    formatter,
                    "candidate frecency score must be finite, got {score}"
                )
            }
        }
    }
}

impl Error for RankingError {}

#[derive(Debug, Clone, Copy)]
enum HistoryKey {
    Frecency(f64),
    Frequency(u64),
    Recency(u64),
}

impl HistoryKey {
    fn descending_cmp(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Frecency(left), Self::Frecency(right)) => right.total_cmp(&left),
            (Self::Frequency(left), Self::Frequency(right))
            | (Self::Recency(left), Self::Recency(right)) => right.cmp(&left),
            _ => unreachable!("all history keys use the selected mode"),
        }
    }
}

/// Orders matched candidates from best to worst using the canonical policy.
///
/// History is compared descending in the selected mode, followed by fuzzy
/// score descending and preserved path ascending. Frecency inputs are fully
/// validated before the candidates are reordered.
pub fn rank(
    candidates: &mut [Candidate<'_>],
    history_mode: HistoryMode,
    current_tick: u64,
) -> Result<(), RankingError> {
    rank_with_frecency_scorer(candidates, history_mode, current_tick, |record, tick| {
        record.score_at(tick)
    })
}

fn rank_with_frecency_scorer(
    candidates: &mut [Candidate<'_>],
    history_mode: HistoryMode,
    current_tick: u64,
    mut frecency_score: impl FnMut(Record, u64) -> f64,
) -> Result<(), RankingError> {
    if history_mode == HistoryMode::Frecency {
        for candidate in candidates.iter() {
            if current_tick < candidate.record.last_tick() {
                return Err(RankingError::CurrentTickPrecedesLastVisit {
                    current_tick,
                    last_tick: candidate.record.last_tick(),
                });
            }
        }
    }

    // Decorate: compute each candidate's history key exactly once rather than
    // repeating frecency calculations inside the sort comparator.
    let mut decorated = Vec::with_capacity(candidates.len());
    for candidate in candidates.iter().copied() {
        let history_key = match history_mode {
            HistoryMode::Frecency => {
                let score = frecency_score(candidate.record, current_tick);
                if !score.is_finite() {
                    return Err(RankingError::NonFiniteFrecencyScore { score });
                }
                HistoryKey::Frecency(score)
            }
            HistoryMode::Frequency => HistoryKey::Frequency(candidate.record.visits()),
            HistoryMode::Recency => HistoryKey::Recency(candidate.record.last_tick()),
        };
        decorated.push((candidate, history_key));
    }

    // Sort: compare the cached history keys, then the canonical tie-breakers.
    decorated.sort_by(|(left, left_history), (right, right_history)| {
        left_history
            .descending_cmp(*right_history)
            .then_with(|| right.fuzzy_score.cmp(&left.fuzzy_score))
            .then_with(|| left.path.cmp(right.path))
    });

    // Undecorate: copy the ordered candidates back and discard the cached keys.
    for (target, (candidate, _)) in candidates.iter_mut().zip(decorated) {
        *target = candidate;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::{Candidate, HistoryMode, RankingError, rank, rank_with_frecency_scorer};
    use crate::frecency::Record;

    const TICK: u64 = 100;

    fn candidate(
        path: &str,
        score: f64,
        visits: u64,
        last_tick: u64,
        fuzzy_score: i64,
    ) -> Candidate<'_> {
        Candidate::new(
            path,
            Record::new(visits, last_tick, score).unwrap(),
            fuzzy_score,
        )
    }

    #[test]
    fn fuzzy_quality_is_used_only_when_history_ties() {
        let mut candidates = [
            candidate("/work/doccache-service", 5.0, 5, 90, 41),
            candidate("/work/docs", 5.0, 5, 90, 112),
        ];
        rank(&mut candidates, HistoryMode::Frecency, TICK).unwrap();
        assert_eq!(candidates[0].path(), "/work/docs");
    }

    #[test]
    fn frecency_is_computed_once_per_candidate() {
        let mut candidates = [
            candidate("/one", 1.0, 1, TICK, 1),
            candidate("/two", 2.0, 2, TICK, 2),
            candidate("/three", 3.0, 3, TICK, 3),
        ];
        let mut computations = 0;

        rank_with_frecency_scorer(&mut candidates, HistoryMode::Frecency, TICK, |record, _| {
            computations += 1;
            record.stored_score()
        })
        .unwrap();

        assert_eq!(computations, candidates.len());
    }

    #[test]
    fn frequency_and_recency_compare_u64_values_exactly() {
        let large = (1_u64 << 53) + 1;
        let mut frequency = [
            candidate("/lower", 1.0, large, large + 1, 10),
            candidate("/higher", 1.0, large + 1, large + 1, 10),
        ];
        rank(&mut frequency, HistoryMode::Frequency, TICK).unwrap();
        assert_eq!(frequency[0].path(), "/higher");

        let mut recency = [
            candidate("/lower", 1.0, 1, large, 10),
            candidate("/higher", 1.0, 1, large + 1, 10),
        ];
        rank(&mut recency, HistoryMode::Recency, 0).unwrap();
        assert_eq!(recency[0].path(), "/higher");
    }

    #[test]
    fn empty_singleton_and_equal_tick_inputs_are_supported() {
        let mut empty = [];
        rank(&mut empty, HistoryMode::Frecency, TICK).unwrap();

        let mut singleton = [candidate("/only", 1.0, 1, TICK, 1)];
        rank(&mut singleton, HistoryMode::Frecency, TICK).unwrap();
        assert_eq!(singleton[0].path(), "/only");
    }

    #[test]
    fn invalid_clock_does_not_reorder_input() {
        let mut candidates = [
            candidate("/z-valid", 2.0, 2, TICK, 2),
            candidate("/future", 3.0, 3, TICK + 1, 3),
            candidate("/a-valid", 1.0, 1, TICK, 1),
        ];
        let original = candidates;
        assert_eq!(
            rank(&mut candidates, HistoryMode::Frecency, TICK),
            Err(RankingError::CurrentTickPrecedesLastVisit {
                current_tick: TICK,
                last_tick: TICK + 1,
            })
        );
        assert_eq!(candidates, original);
    }

    #[rstest]
    #[case::nan(f64::NAN)]
    #[case::positive_infinity(f64::INFINITY)]
    #[case::negative_infinity(f64::NEG_INFINITY)]
    fn non_finite_frecency_does_not_reorder_input(#[case] score: f64) {
        let mut candidates = [
            candidate("/z-valid", 1.0, 1, TICK, 1),
            candidate("/invalid", 2.0, 1, TICK, 1),
            candidate("/a-valid", 1.0, 1, TICK, 1),
        ];
        let original_paths: Vec<_> = candidates.iter().map(Candidate::path).collect();
        let error =
            rank_with_frecency_scorer(&mut candidates, HistoryMode::Frecency, TICK, |record, _| {
                if record.stored_score() == 2.0 {
                    score
                } else {
                    record.stored_score()
                }
            })
            .unwrap_err();
        match error {
            RankingError::NonFiniteFrecencyScore { score: actual } => {
                assert!(actual == score || actual.is_nan() && score.is_nan());
            }
            other => panic!("unexpected error: {other}"),
        }
        assert_eq!(
            candidates.iter().map(Candidate::path).collect::<Vec<_>>(),
            original_paths
        );
    }

    #[rstest]
    #[case::frequency(HistoryMode::Frequency)]
    #[case::recency(HistoryMode::Recency)]
    fn integer_history_modes_do_not_invoke_the_frecency_scorer(#[case] mode: HistoryMode) {
        let mut candidates = [
            candidate("/lower", 1.0, 1, 1, 100),
            candidate("/higher", 2.0, 2, 2, 0),
        ];
        rank_with_frecency_scorer(&mut candidates, mode, 0, |_, _| {
            panic!("integer history modes must not score frecency")
        })
        .unwrap();
        assert_eq!(candidates[0].path(), "/higher");
    }

    #[test]
    fn errors_are_useful_standard_errors() {
        fn accepts_error(_: &dyn Error) {}

        let clock = RankingError::CurrentTickPrecedesLastVisit {
            current_tick: 10,
            last_tick: 11,
        };
        let score = RankingError::NonFiniteFrecencyScore {
            score: f64::INFINITY,
        };
        accepts_error(&clock);
        accepts_error(&score);
        assert_eq!(
            clock.to_string(),
            "current event-clock tick 10 precedes a candidate's last visit at tick 11"
        );
        assert_eq!(
            score.to_string(),
            "candidate frecency score must be finite, got inf"
        );
    }
}
