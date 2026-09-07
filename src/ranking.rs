//! Canonical ordering for already-matched directory candidates.

use std::{cmp::Ordering, error::Error, fmt};

use crate::frecency::{DEFAULT_LAMBDA, Record};

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

    /// Returns the candidate's incremental directory history.
    #[must_use]
    pub const fn record(&self) -> Record {
        self.record
    }

    /// Returns the aggregate quality from matching every query term.
    #[must_use]
    pub const fn fuzzy_score(&self) -> i64 {
        self.fuzzy_score
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
        record.score_at(tick, DEFAULT_LAMBDA)
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
            if current_tick < candidate.record.last_tick {
                return Err(RankingError::CurrentTickPrecedesLastVisit {
                    current_tick,
                    last_tick: candidate.record.last_tick,
                });
            }
        }
    }

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
            HistoryMode::Frequency => HistoryKey::Frequency(candidate.record.visits),
            HistoryMode::Recency => HistoryKey::Recency(candidate.record.last_tick),
        };
        decorated.push((candidate, history_key));
    }

    decorated.sort_by(|(left, left_history), (right, right_history)| {
        left_history
            .descending_cmp(*right_history)
            .then_with(|| right.fuzzy_score.cmp(&left.fuzzy_score))
            .then_with(|| left.path.cmp(right.path))
    });
    for (target, (candidate, _)) in candidates.iter_mut().zip(decorated) {
        *target = candidate;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

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
            Record {
                visits,
                last_tick,
                score,
            },
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
            record.score
        })
        .unwrap();

        assert_eq!(computations, candidates.len());
    }

    #[test]
    fn frequency_and_recency_compare_u64_values_exactly() {
        let large = (1_u64 << 53) + 1;
        let mut frequency = [
            candidate("/lower", 1.0, large, 90, 10),
            candidate("/higher", 1.0, large + 1, 90, 10),
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

    #[test]
    fn every_non_finite_frecency_does_not_reorder_input() {
        for score in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut candidates = [
                candidate("/z-valid", 1.0, 1, TICK, 1),
                candidate("/invalid", score, 1, TICK, 1),
                candidate("/a-valid", 1.0, 1, TICK, 1),
            ];
            let original_paths: Vec<_> = candidates.iter().map(Candidate::path).collect();
            let error = rank(&mut candidates, HistoryMode::Frecency, TICK).unwrap_err();
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
    }

    #[test]
    fn non_finite_stored_scores_do_not_affect_integer_history_modes() {
        for mode in [HistoryMode::Frequency, HistoryMode::Recency] {
            let mut candidates = [
                candidate("/lower", f64::NAN, 1, 1, 100),
                candidate("/higher", f64::INFINITY, 2, 2, 0),
            ];
            rank(&mut candidates, mode, 0).unwrap();
            assert_eq!(candidates[0].path(), "/higher");
        }
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
