//! Ranking policies retained for regression-testing the original experiment.
//!
//! None of these types are part of zfz's production API. The scenario and
//! property suites keep the rejected policies here so changes to matching or
//! frecency can still be evaluated against the same alternatives and fixture
//! corpus. Production ranking is intentionally exercised through
//! `zfz::ranking::rank` instead.

use std::cmp::Ordering;

use zfz::{
    frecency::{DEFAULT_LAMBDA, Record},
    ranking::HistoryMode,
};

/// Policies compared while selecting the canonical production ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Use history alone, with preserved path as the deterministic tie-break.
    HistoryOnly,
    /// Use fuzzy quality only to resolve equal-history candidates.
    HistoryThenFuzzy,
    /// Blend result-set-normalised history and fuzzy values at an 80:20 ratio.
    ///
    /// This was rejected because adding an inferior candidate can change the
    /// relative order of existing candidates by changing the normalisation
    /// range.
    MinMaxCombined,
    /// Blend dense history and fuzzy ranks with configurable integer weights.
    ///
    /// This remained competitive in the fixtures, but introduced a weight
    /// policy without improving on history-then-fuzzy.
    RankFusion {
        history_weight: u32,
        fuzzy_weight: u32,
    },
}

/// Experimental handling of query-term alignment order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermOrderPolicy {
    /// Match production behaviour by ignoring term order during ranking.
    Ignore,
    // Each integration-test crate compiles this support module independently;
    // the property suite does not construct this scenario-only variant.
    #[allow(dead_code)]
    /// Prefer in-order term alignments after history and fuzzy ties.
    TieBreak,
}

/// Test-only candidate carrying the term-order signal excluded from production.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate<'a> {
    pub path: &'a str,
    pub record: Record,
    pub fuzzy_score: i64,
    pub terms_in_path_order: bool,
}

/// Applies one experimental policy in place for fixture and property tests.
pub fn rank(
    candidates: &mut [Candidate<'_>],
    mode: HistoryMode,
    current_tick: u64,
    strategy: Strategy,
    term_order: TermOrderPolicy,
) {
    match strategy {
        Strategy::HistoryOnly => candidates.sort_by(|left, right| {
            history_cmp(left, right, mode, current_tick).then_with(|| left.path.cmp(right.path))
        }),
        Strategy::HistoryThenFuzzy => candidates
            .sort_by(|left, right| signal_cmp(left, right, mode, current_tick, term_order)),
        Strategy::MinMaxCombined => rank_min_max(candidates, mode, current_tick, term_order),
        Strategy::RankFusion {
            history_weight,
            fuzzy_weight,
        } => rank_fusion(
            candidates,
            mode,
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
        .then_with(|| match term_order {
            TermOrderPolicy::Ignore => Ordering::Equal,
            TermOrderPolicy::TieBreak => right.terms_in_path_order.cmp(&left.terms_in_path_order),
        })
        .then_with(|| left.path.cmp(right.path))
}

fn rank_min_max(
    candidates: &mut [Candidate<'_>],
    mode: HistoryMode,
    current_tick: u64,
    term_order: TermOrderPolicy,
) {
    // Both inputs are scaled relative to this result set before blending. This
    // dependency on the other candidates is the policy's important weakness.
    let history = normalised_history(candidates, mode, current_tick);
    let fuzzy: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.fuzzy_score as f64)
        .collect();
    let fuzzy_range = value_range(fuzzy.iter().copied());
    let mut scored: Vec<_> = candidates
        .iter()
        .copied()
        .zip(history)
        .zip(fuzzy)
        .map(|((candidate, history), fuzzy)| {
            (
                candidate,
                0.8 * history + 0.2 * normalise(fuzzy, fuzzy_range),
            )
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
        // Equal values carry no distinguishing information, so giving every
        // candidate zero preserves the later tie-breakers.
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
    assert!(history_weight > 0 && fuzzy_weight > 0);
    // Dense ranks give tied signal values equal rank. Converting ranks to
    // points makes larger totals better while avoiding floating-point blends.
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
            let history = count.saturating_sub(history_ranks[index] as u128);
            let fuzzy = count.saturating_sub(fuzzy_ranks[index] as u128);
            (
                candidate,
                history * u128::from(history_weight) + fuzzy * u128::from(fuzzy_weight),
            )
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
    // Rank original indices so the two independently sorted signals can be
    // combined without first rearranging the candidates themselves.
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
