//! Candidate ordering experiments and the canonical ranking policy.
//!
//! Matching establishes eligibility before values reach this module. Ranking
//! keeps records and match quality separate so history modes retain their exact
//! semantics and experimental policies remain explicit.

use std::cmp::Ordering;

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

/// The ranking strategies evaluated for the initial canonical ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Order by history; use the path only for deterministic ties.
    HistoryOnly,
    /// Order by history, then fuzzy quality, then path.
    HistoryThenFuzzy,
    /// Combine result-set-normalised history and fuzzy scores at an 80:20 ratio.
    MinMaxCombined,
    /// Combine dense history and fuzzy ranks using the supplied weights.
    RankFusion {
        history_weight: u32,
        fuzzy_weight: u32,
    },
}

/// Whether term alignment order is considered after history and fuzzy ties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermOrderPolicy {
    Ignore,
    TieBreak,
}

/// The selected policy for normal navigation and ordered result lists.
pub const CANONICAL_STRATEGY: Strategy = Strategy::HistoryThenFuzzy;

/// The provisional term-order policy pending evaluation against real usage.
pub const CANONICAL_TERM_ORDER: TermOrderPolicy = TermOrderPolicy::Ignore;

/// Ranking inputs for one already-matched path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate<'a> {
    /// Path preserved exactly as supplied by the shell.
    pub path: &'a str,
    /// Incremental directory history.
    pub record: Record,
    /// Aggregate quality from matching every query term.
    pub fuzzy_score: i64,
    /// Whether selected term alignments occur in query order.
    pub terms_in_path_order: bool,
}

/// Orders matched candidates from best to worst.
///
/// `current_tick` must not precede a record's last visit when ranking by
/// frecency. Rank-fusion weights must both be nonzero. Paths provide a
/// deterministic final tie-break, so input order never affects the result.
pub fn rank(
    candidates: &mut [Candidate<'_>],
    history_mode: HistoryMode,
    current_tick: u64,
    strategy: Strategy,
    term_order: TermOrderPolicy,
) {
    if history_mode == HistoryMode::Frecency {
        assert!(
            candidates
                .iter()
                .all(|candidate| current_tick >= candidate.record.last_tick),
            "event clock cannot precede a record's last visit"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.record.score.is_finite()),
            "frecency scores must be finite"
        );
    }

    match strategy {
        Strategy::HistoryOnly => candidates.sort_by(|left, right| {
            history_cmp(left, right, history_mode, current_tick)
                .then_with(|| left.path.cmp(right.path))
        }),
        Strategy::HistoryThenFuzzy => candidates
            .sort_by(|left, right| signal_cmp(left, right, history_mode, current_tick, term_order)),
        Strategy::MinMaxCombined => {
            rank_min_max(candidates, history_mode, current_tick, term_order);
        }
        Strategy::RankFusion {
            history_weight,
            fuzzy_weight,
        } => rank_fusion(
            candidates,
            history_mode,
            current_tick,
            term_order,
            history_weight,
            fuzzy_weight,
        ),
    }
}

fn history_cmp(
    left: &Candidate<'_>,
    right: &Candidate<'_>,
    mode: HistoryMode,
    current_tick: u64,
) -> Ordering {
    match mode {
        HistoryMode::Frecency => right
            .record
            .score_at(current_tick, DEFAULT_LAMBDA)
            .total_cmp(&left.record.score_at(current_tick, DEFAULT_LAMBDA)),
        HistoryMode::Frequency => right.record.visits.cmp(&left.record.visits),
        HistoryMode::Recency => right.record.last_tick.cmp(&left.record.last_tick),
    }
}

fn signal_cmp(
    left: &Candidate<'_>,
    right: &Candidate<'_>,
    mode: HistoryMode,
    current_tick: u64,
    term_order: TermOrderPolicy,
) -> Ordering {
    history_cmp(left, right, mode, current_tick)
        .then_with(|| right.fuzzy_score.cmp(&left.fuzzy_score))
        .then_with(|| term_order_cmp(left, right, term_order))
        .then_with(|| left.path.cmp(right.path))
}

fn term_order_cmp(
    left: &Candidate<'_>,
    right: &Candidate<'_>,
    policy: TermOrderPolicy,
) -> Ordering {
    match policy {
        TermOrderPolicy::Ignore => Ordering::Equal,
        TermOrderPolicy::TieBreak => right.terms_in_path_order.cmp(&left.terms_in_path_order),
    }
}

fn rank_min_max(
    candidates: &mut [Candidate<'_>],
    mode: HistoryMode,
    current_tick: u64,
    term_order: TermOrderPolicy,
) {
    let normalised_history = normalised_history(candidates, mode, current_tick);
    let fuzzy_values: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.fuzzy_score as f64)
        .collect();
    let fuzzy_range = value_range(fuzzy_values.iter().copied());

    let mut scored: Vec<_> = candidates
        .iter()
        .copied()
        .zip(normalised_history)
        .zip(fuzzy_values)
        .map(|((candidate, history), fuzzy)| {
            let combined = 0.8 * history + 0.2 * normalise(fuzzy, fuzzy_range);
            (candidate, combined)
        })
        .collect();

    scored.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| signal_cmp(left, right, mode, current_tick, term_order))
    });
    for (target, (candidate, _)) in candidates.iter_mut().zip(scored) {
        *target = candidate;
    }
}

fn normalised_history(
    candidates: &[Candidate<'_>],
    mode: HistoryMode,
    current_tick: u64,
) -> Vec<f64> {
    match mode {
        HistoryMode::Frecency => {
            let values: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.record.score_at(current_tick, DEFAULT_LAMBDA))
                .collect();
            let range = value_range(values.iter().copied());
            values
                .into_iter()
                .map(|value| normalise(value, range))
                .collect()
        }
        HistoryMode::Frequency => normalise_u64(
            candidates
                .iter()
                .map(|candidate| candidate.record.visits)
                .collect(),
        ),
        HistoryMode::Recency => normalise_u64(
            candidates
                .iter()
                .map(|candidate| candidate.record.last_tick)
                .collect(),
        ),
    }
}

fn normalise_u64(values: Vec<u64>) -> Vec<f64> {
    let Some(min) = values.iter().copied().min() else {
        return Vec::new();
    };
    let max = values.iter().copied().max().unwrap();
    if min == max {
        return vec![0.0; values.len()];
    }
    let range = (max - min) as f64;
    values
        .into_iter()
        .map(|value| (value - min) as f64 / range)
        .collect()
}

fn value_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
        (min.min(value), max.max(value))
    })
}

fn normalise(value: f64, (min, max): (f64, f64)) -> f64 {
    if min == max {
        0.0
    } else {
        (value - min) / (max - min)
    }
}

fn rank_fusion(
    candidates: &mut [Candidate<'_>],
    mode: HistoryMode,
    current_tick: u64,
    term_order: TermOrderPolicy,
    history_weight: u32,
    fuzzy_weight: u32,
) {
    assert!(
        history_weight > 0 && fuzzy_weight > 0,
        "rank-fusion weights must be nonzero"
    );

    let history_ranks = dense_ranks(candidates, |left, right| {
        history_cmp(left, right, mode, current_tick)
    });
    let fuzzy_ranks = dense_ranks(candidates, |left, right| {
        right.fuzzy_score.cmp(&left.fuzzy_score)
    });
    let count = candidates.len() as u128;
    let mut scored: Vec<_> = candidates
        .iter()
        .copied()
        .enumerate()
        .map(|(index, candidate)| {
            let history_points = count.saturating_sub(history_ranks[index] as u128);
            let fuzzy_points = count.saturating_sub(fuzzy_ranks[index] as u128);
            let combined = history_points * u128::from(history_weight)
                + fuzzy_points * u128::from(fuzzy_weight);
            (candidate, combined)
        })
        .collect();

    scored.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| signal_cmp(left, right, mode, current_tick, term_order))
    });
    for (target, (candidate, _)) in candidates.iter_mut().zip(scored) {
        *target = candidate;
    }
}

fn dense_ranks(
    candidates: &[Candidate<'_>],
    compare: impl Fn(&Candidate<'_>, &Candidate<'_>) -> Ordering,
) -> Vec<usize> {
    let mut indices: Vec<_> = (0..candidates.len()).collect();
    indices.sort_by(|left, right| compare(&candidates[*left], &candidates[*right]));

    let mut ranks = vec![0; candidates.len()];
    let mut rank = 0;
    for pair in indices.windows(2) {
        ranks[pair[0]] = rank;
        if compare(&candidates[pair[0]], &candidates[pair[1]]) != Ordering::Equal {
            rank += 1;
        }
    }
    if let Some(last) = indices.last() {
        ranks[*last] = rank;
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::{
        CANONICAL_STRATEGY, CANONICAL_TERM_ORDER, Candidate, HistoryMode, Strategy,
        TermOrderPolicy, rank,
    };
    use crate::frecency::Record;

    const TICK: u64 = 100;

    fn candidate<'a>(
        path: &'a str,
        score: f64,
        visits: u64,
        last_tick: u64,
        fuzzy_score: i64,
    ) -> Candidate<'a> {
        Candidate {
            path,
            record: Record {
                visits,
                last_tick,
                score,
            },
            fuzzy_score,
            terms_in_path_order: true,
        }
    }

    fn canonical(candidates: &mut [Candidate<'_>], mode: HistoryMode) {
        rank(
            candidates,
            mode,
            TICK,
            CANONICAL_STRATEGY,
            CANONICAL_TERM_ORDER,
        );
    }

    #[test]
    fn canonical_uses_fuzzy_quality_only_when_history_ties() {
        let mut candidates = [
            candidate("/work/doccache-service", 5.0, 5, 90, 41),
            candidate("/work/docs", 5.0, 5, 90, 112),
        ];

        canonical(&mut candidates, HistoryMode::Frecency);

        assert_eq!(candidates[0].path, "/work/docs");
    }

    #[test]
    fn frequency_and_recency_compare_u64_values_exactly() {
        let large = (1_u64 << 53) + 1;
        let mut frequency = [
            candidate("/lower", 1.0, large, 90, 10),
            candidate("/higher", 1.0, large + 1, 90, 10),
        ];
        canonical(&mut frequency, HistoryMode::Frequency);
        assert_eq!(frequency[0].path, "/higher");

        let mut recency = [
            candidate("/lower", 1.0, 1, large, 10),
            candidate("/higher", 1.0, 1, large + 1, 10),
        ];
        rank(
            &mut recency,
            HistoryMode::Recency,
            0,
            CANONICAL_STRATEGY,
            CANONICAL_TERM_ORDER,
        );
        assert_eq!(recency[0].path, "/higher");
    }

    #[test]
    fn term_order_can_be_tested_as_a_final_tie_break() {
        let mut ignored = [
            Candidate {
                terms_in_path_order: false,
                ..candidate("/a/reverse", 5.0, 5, 90, 50)
            },
            candidate("/z/forward", 5.0, 5, 90, 50),
        ];
        let mut preferred = ignored;

        canonical(&mut ignored, HistoryMode::Frecency);
        rank(
            &mut preferred,
            HistoryMode::Frecency,
            TICK,
            CANONICAL_STRATEGY,
            TermOrderPolicy::TieBreak,
        );

        assert_eq!(ignored[0].path, "/a/reverse");
        assert_eq!(preferred[0].path, "/z/forward");
    }

    #[test]
    fn min_max_combination_can_change_when_an_inferior_candidate_is_added() {
        let mut pair = [
            candidate("/work/docs", 9.0, 9, 90, 100),
            candidate("/work/distant", 10.0, 10, 90, 10),
        ];
        rank(
            &mut pair,
            HistoryMode::Frecency,
            TICK,
            Strategy::MinMaxCombined,
            CANONICAL_TERM_ORDER,
        );
        assert_eq!(pair[0].path, "/work/distant");

        let mut with_inferior = [
            candidate("/work/docs", 9.0, 9, 90, 100),
            candidate("/work/distant", 10.0, 10, 90, 10),
            candidate("/work/dormant", 0.0, 0, 0, 10),
        ];
        rank(
            &mut with_inferior,
            HistoryMode::Frecency,
            TICK,
            Strategy::MinMaxCombined,
            CANONICAL_TERM_ORDER,
        );
        assert_eq!(with_inferior[0].path, "/work/docs");
    }

    #[test]
    fn empty_and_singleton_inputs_are_supported() {
        let mut empty = [];
        canonical(&mut empty, HistoryMode::Frecency);

        let mut singleton = [candidate("/only", 1.0, 1, 1, 1)];
        canonical(&mut singleton, HistoryMode::Frecency);
        assert_eq!(singleton[0].path, "/only");
    }

    #[test]
    #[should_panic(expected = "event clock cannot precede")]
    fn frecency_rejects_a_current_tick_before_the_record() {
        let mut candidates = [candidate("/future", 1.0, 1, TICK + 1, 1)];
        canonical(&mut candidates, HistoryMode::Frecency);
    }

    #[test]
    #[should_panic(expected = "frecency scores must be finite")]
    fn frecency_rejects_non_finite_scores() {
        let mut candidates = [candidate("/invalid", f64::NAN, 1, TICK, 1)];
        canonical(&mut candidates, HistoryMode::Frecency);
    }

    #[test]
    #[should_panic(expected = "rank-fusion weights must be nonzero")]
    fn rank_fusion_rejects_zero_weights() {
        let mut candidates = [candidate("/only", 1.0, 1, 1, 1)];
        rank(
            &mut candidates,
            HistoryMode::Frecency,
            TICK,
            Strategy::RankFusion {
                history_weight: 0,
                fuzzy_weight: 1,
            },
            CANONICAL_TERM_ORDER,
        );
    }
}
