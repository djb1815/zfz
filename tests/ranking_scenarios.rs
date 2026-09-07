use std::collections::BTreeMap;

mod support;

use pretty_assertions::assert_eq;
use support::ranking::{
    Candidate as ExperimentalCandidate, Strategy, TermOrderPolicy, rank as rank_experimental,
};
use zfz::{
    frecency::Record,
    matcher::match_terms,
    ranking::{Candidate, HistoryMode, rank},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expectation {
    Winner,
    Other,
    Neutral,
}

#[derive(Debug)]
struct FixtureRecord {
    mode: HistoryMode,
    current_tick: u64,
    path: String,
    record: Record,
    terms: String,
    expectation: Expectation,
}

fn scenarios() -> BTreeMap<String, Vec<FixtureRecord>> {
    let contents = include_str!("fixtures/ranking_histories.tsv");
    let mut scenarios: BTreeMap<String, Vec<FixtureRecord>> = BTreeMap::new();

    for line in contents.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 9, "invalid fixture line: {line}");
        let mode = match fields[1] {
            "frecency" => HistoryMode::Frecency,
            "frequency" => HistoryMode::Frequency,
            "recency" => HistoryMode::Recency,
            mode => panic!("invalid history mode: {mode}"),
        };
        let expectation = match fields[8] {
            "winner" => Expectation::Winner,
            "other" => Expectation::Other,
            "neutral" => Expectation::Neutral,
            expectation => panic!("invalid expectation: {expectation}"),
        };
        scenarios
            .entry(fields[0].to_owned())
            .or_default()
            .push(FixtureRecord {
                mode,
                current_tick: fields[2].parse().unwrap(),
                path: fields[3].to_owned(),
                record: Record {
                    visits: fields[4].parse().unwrap(),
                    last_tick: fields[5].parse().unwrap(),
                    score: fields[6].parse().unwrap(),
                },
                terms: fields[7].to_owned(),
                expectation,
            });
    }

    scenarios
}

fn experimental_candidates(records: &[FixtureRecord]) -> Vec<ExperimentalCandidate<'_>> {
    records
        .iter()
        .map(|fixture| {
            let matched = match_terms(&fixture.path, fixture.terms.split_whitespace())
                .unwrap_or_else(|| panic!("fixture path must match: {}", fixture.path));
            ExperimentalCandidate {
                path: &fixture.path,
                record: fixture.record,
                fuzzy_score: matched.fuzzy_score,
                terms_in_path_order: matched.terms_in_path_order,
            }
        })
        .collect()
}

fn top_path(records: &[FixtureRecord], strategy: Strategy, term_order: TermOrderPolicy) -> &str {
    let mut candidates = experimental_candidates(records);
    rank_experimental(
        &mut candidates,
        records[0].mode,
        records[0].current_tick,
        strategy,
        term_order,
    );
    candidates[0].path
}

fn canonical_top_path(records: &[FixtureRecord]) -> &str {
    let mut candidates: Vec<_> = experimental_candidates(records)
        .into_iter()
        .map(|candidate| Candidate::new(candidate.path, candidate.record, candidate.fuzzy_score))
        .collect();
    rank(&mut candidates, records[0].mode, records[0].current_tick).unwrap();
    candidates[0].path()
}

fn expected_path(records: &[FixtureRecord]) -> Option<&str> {
    let winners: Vec<_> = records
        .iter()
        .filter(|record| record.expectation == Expectation::Winner)
        .collect();
    assert!(winners.len() <= 1, "scenario has multiple expected winners");
    winners.first().map(|record| record.path.as_str())
}

fn wins(strategy: Strategy) -> usize {
    scenarios()
        .values()
        .filter_map(|records| {
            expected_path(records).map(|expected| {
                usize::from(top_path(records, strategy, TermOrderPolicy::Ignore) == expected)
            })
        })
        .sum()
}

#[test]
fn fixture_covers_all_history_modes_and_twenty_scored_scenarios() {
    let scenarios = scenarios();
    let scored = scenarios
        .values()
        .filter(|records| expected_path(records).is_some())
        .count();

    assert_eq!(scored, 20);
    assert_eq!(scenarios.len(), 21);
    for records in scenarios.values() {
        assert!(records.len() >= 2);
        assert!(records.iter().all(|record| {
            record.mode == records[0].mode
                && record.current_tick == records[0].current_tick
                && record.terms == records[0].terms
        }));
        let winners = records
            .iter()
            .filter(|record| record.expectation == Expectation::Winner)
            .count();
        assert!(
            winners == 1
                || records
                    .iter()
                    .all(|record| record.expectation == Expectation::Neutral)
        );
    }
    for mode in [
        HistoryMode::Frecency,
        HistoryMode::Frequency,
        HistoryMode::Recency,
    ] {
        assert!(scenarios.values().any(|records| records[0].mode == mode));
    }
}

#[test]
fn canonical_mismatches_are_explicitly_recorded() {
    let mut mismatches = Vec::new();
    for (name, records) in scenarios() {
        let Some(expected) = expected_path(&records) else {
            continue;
        };
        let actual = canonical_top_path(&records);
        if actual != expected {
            mismatches.push(name);
        }
    }

    assert_eq!(
        mismatches,
        ["deep_path_tie", "long_query_tie", "smart_case_tie"]
    );
}

#[test]
fn selection_rule_prefers_history_then_fuzzy() {
    let history_only = wins(Strategy::HistoryOnly);
    let history_then_fuzzy = wins(Strategy::HistoryThenFuzzy);
    let min_max = wins(Strategy::MinMaxCombined);
    let rank_fusion = wins(Strategy::RankFusion {
        history_weight: 4,
        fuzzy_weight: 1,
    });

    assert_eq!(
        (history_only, history_then_fuzzy, min_max, rank_fusion),
        (10, 17, 16, 17)
    );
}

#[test]
fn rank_fusion_weight_sensitivity_is_recorded() {
    let wins_by_history_weight: Vec<_> = [1, 2, 4, 9]
        .map(|history_weight| {
            wins(Strategy::RankFusion {
                history_weight,
                fuzzy_weight: 1,
            })
        })
        .into();

    assert_eq!(wins_by_history_weight.len(), 4);
    assert_eq!(wins_by_history_weight, [17, 17, 17, 17]);
}

#[test]
fn term_order_policies_are_compared_without_selecting_one() {
    let scenarios = scenarios();
    let records = &scenarios["term_order_neutral"];
    let ignored = top_path(records, Strategy::HistoryThenFuzzy, TermOrderPolicy::Ignore);
    let preferred = top_path(
        records,
        Strategy::HistoryThenFuzzy,
        TermOrderPolicy::TieBreak,
    );

    assert_eq!(ignored, "/Users/alex/a/projects/zfz/Documents");
    assert_eq!(preferred, "/Users/alex/z/Documents/projects/zfz");
}

#[test]
fn term_order_never_overrides_a_history_preference() {
    let scenarios = scenarios();
    let records = &scenarios["term_order_history"];
    let expected = expected_path(records).unwrap();

    assert_eq!(
        top_path(records, Strategy::HistoryThenFuzzy, TermOrderPolicy::Ignore),
        expected
    );
    assert_eq!(
        top_path(
            records,
            Strategy::HistoryThenFuzzy,
            TermOrderPolicy::TieBreak,
        ),
        expected
    );
}
