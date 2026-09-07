use zfz::{
    frecency::Record,
    ranking::{Candidate, HistoryMode, Strategy, TermOrderPolicy, rank},
};

const TICK: u64 = 100;
const STRATEGIES: [Strategy; 4] = [
    Strategy::HistoryOnly,
    Strategy::HistoryThenFuzzy,
    Strategy::MinMaxCombined,
    Strategy::RankFusion {
        history_weight: 4,
        fuzzy_weight: 1,
    },
];

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

fn paths<'a>(candidates: &[Candidate<'a>]) -> Vec<&'a str> {
    candidates.iter().map(|candidate| candidate.path).collect()
}

fn permutations(values: &mut [usize], start: usize, output: &mut Vec<Vec<usize>>) {
    if start == values.len() {
        output.push(values.to_vec());
        return;
    }
    for index in start..values.len() {
        values.swap(start, index);
        permutations(values, start + 1, output);
        values.swap(start, index);
    }
}

#[test]
fn every_strategy_is_independent_of_input_order() {
    let original = [
        candidate("/alpha", 4.0, 7, 90, 30),
        candidate("/bravo", 8.0, 4, 95, 10),
        candidate("/charlie", 4.0, 7, 95, 50),
        candidate("/delta", 1.0, 1, 70, 20),
    ];
    let mut orders = Vec::new();
    permutations(&mut [0, 1, 2, 3], 0, &mut orders);

    for mode in [
        HistoryMode::Frecency,
        HistoryMode::Frequency,
        HistoryMode::Recency,
    ] {
        for strategy in STRATEGIES {
            let mut expected = original;
            rank(&mut expected, mode, TICK, strategy, TermOrderPolicy::Ignore);
            let expected_paths = paths(&expected);

            for order in &orders {
                let mut permuted: Vec<_> = order.iter().map(|index| original[*index]).collect();
                rank(&mut permuted, mode, TICK, strategy, TermOrderPolicy::Ignore);
                assert_eq!(paths(&permuted), expected_paths);
            }
        }
    }
}

#[test]
fn canonical_history_is_monotonic_across_a_generated_score_grid() {
    for lower in 0..5 {
        for higher in (lower + 1)..6 {
            for lower_fuzzy in [0, 50, 500] {
                for higher_fuzzy in [0, 50, 500] {
                    let mut frecency = [
                        candidate("/lower", lower as f64, 1, 1, lower_fuzzy),
                        candidate("/higher", higher as f64, 1, 1, higher_fuzzy),
                    ];
                    rank(
                        &mut frecency,
                        HistoryMode::Frecency,
                        TICK,
                        Strategy::HistoryThenFuzzy,
                        TermOrderPolicy::Ignore,
                    );
                    assert_eq!(frecency[0].path, "/higher");

                    let mut frequency = [
                        candidate("/lower", 1.0, lower, 1, lower_fuzzy),
                        candidate("/higher", 1.0, higher, 1, higher_fuzzy),
                    ];
                    rank(
                        &mut frequency,
                        HistoryMode::Frequency,
                        TICK,
                        Strategy::HistoryThenFuzzy,
                        TermOrderPolicy::Ignore,
                    );
                    assert_eq!(frequency[0].path, "/higher");

                    let mut recency = [
                        candidate("/lower", 1.0, 1, lower, lower_fuzzy),
                        candidate("/higher", 1.0, 1, higher, higher_fuzzy),
                    ];
                    rank(
                        &mut recency,
                        HistoryMode::Recency,
                        TICK,
                        Strategy::HistoryThenFuzzy,
                        TermOrderPolicy::Ignore,
                    );
                    assert_eq!(recency[0].path, "/higher");
                }
            }
        }
    }
}

#[test]
fn canonical_fuzzy_score_is_monotonic_when_history_ties() {
    for lower_fuzzy in -20..20 {
        let mut candidates = [
            candidate("/lower", 5.0, 5, 90, lower_fuzzy),
            candidate("/higher", 5.0, 5, 90, lower_fuzzy + 1),
        ];
        rank(
            &mut candidates,
            HistoryMode::Frecency,
            TICK,
            Strategy::HistoryThenFuzzy,
            TermOrderPolicy::Ignore,
        );
        assert_eq!(candidates[0].path, "/higher");
    }
}

#[test]
fn eligible_strategies_ignore_an_inferior_added_candidate() {
    let strategies = [
        Strategy::HistoryOnly,
        Strategy::HistoryThenFuzzy,
        Strategy::RankFusion {
            history_weight: 1,
            fuzzy_weight: 1,
        },
        Strategy::RankFusion {
            history_weight: 2,
            fuzzy_weight: 1,
        },
        Strategy::RankFusion {
            history_weight: 4,
            fuzzy_weight: 1,
        },
        Strategy::RankFusion {
            history_weight: 9,
            fuzzy_weight: 1,
        },
    ];

    for strategy in strategies {
        let first = candidate("/first", 10.0, 10, 90, 10);
        let second = candidate("/second", 9.0, 9, 80, 100);
        let inferior = candidate("/inferior", 0.0, 0, 0, 0);
        let mut pair = [first, second];
        let mut expanded = [first, second, inferior];

        rank(
            &mut pair,
            HistoryMode::Frecency,
            TICK,
            strategy,
            TermOrderPolicy::Ignore,
        );
        rank(
            &mut expanded,
            HistoryMode::Frecency,
            TICK,
            strategy,
            TermOrderPolicy::Ignore,
        );

        let expanded_pair: Vec<_> = paths(&expanded)
            .into_iter()
            .filter(|path| *path != "/inferior")
            .collect();
        assert_eq!(expanded_pair, paths(&pair));
    }
}

#[test]
fn rank_fusion_is_history_monotonic_across_tested_weights() {
    for mode in [
        HistoryMode::Frecency,
        HistoryMode::Frequency,
        HistoryMode::Recency,
    ] {
        for history_weight in [1, 2, 4, 9] {
            for fuzzy_score in [0, 50, 500] {
                let strategy = Strategy::RankFusion {
                    history_weight,
                    fuzzy_weight: 1,
                };
                let baseline = [
                    candidate("/target", 3.0, 3, 30, fuzzy_score),
                    candidate("/first", 6.0, 6, 60, 10),
                    candidate("/second", 4.0, 4, 40, 100),
                ];
                let mut before = baseline;
                rank(&mut before, mode, TICK, strategy, TermOrderPolicy::Ignore);
                let before_position = paths(&before)
                    .iter()
                    .position(|path| *path == "/target")
                    .unwrap();

                let mut after = baseline;
                after[0].record = Record {
                    score: 7.0,
                    visits: 7,
                    last_tick: 70,
                };
                rank(&mut after, mode, TICK, strategy, TermOrderPolicy::Ignore);
                let after_position = paths(&after)
                    .iter()
                    .position(|path| *path == "/target")
                    .unwrap();

                assert!(after_position <= before_position);
            }
        }
    }
}

#[test]
fn rank_fusion_top_result_can_depend_on_its_weight() {
    let original = [
        candidate("/history", 6.0, 6, 60, 0),
        candidate("/fuzzy", 5.0, 5, 50, 50),
        candidate("/third", 4.0, 4, 40, 40),
        candidate("/fourth", 3.0, 3, 30, 30),
        candidate("/fifth", 2.0, 2, 20, 20),
        candidate("/sixth", 1.0, 1, 10, 10),
    ];

    let mut four_to_one = original;
    rank(
        &mut four_to_one,
        HistoryMode::Frecency,
        TICK,
        Strategy::RankFusion {
            history_weight: 4,
            fuzzy_weight: 1,
        },
        TermOrderPolicy::Ignore,
    );
    assert_eq!(four_to_one[0].path, "/fuzzy");

    let mut nine_to_one = original;
    rank(
        &mut nine_to_one,
        HistoryMode::Frecency,
        TICK,
        Strategy::RankFusion {
            history_weight: 9,
            fuzzy_weight: 1,
        },
        TermOrderPolicy::Ignore,
    );
    assert_eq!(nine_to_one[0].path, "/history");
}

#[test]
fn complete_ties_fall_back_to_preserved_path() {
    for mode in [
        HistoryMode::Frecency,
        HistoryMode::Frequency,
        HistoryMode::Recency,
    ] {
        let mut candidates = [
            candidate("/zeta", 5.0, 5, 90, 50),
            candidate("/alpha", 5.0, 5, 90, 50),
        ];
        rank(
            &mut candidates,
            mode,
            TICK,
            Strategy::HistoryThenFuzzy,
            TermOrderPolicy::Ignore,
        );
        assert_eq!(candidates[0].path, "/alpha");
    }
}
