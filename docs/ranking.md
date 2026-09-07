# Final ranking experiment

## Recommendation

Use **history as the primary key and fuzzy-match quality only as a tie-breaker**.
The canonical ordering for already eligible candidates is:

1. the selected history mode descending: decayed frecency, visit count, or last
   visit tick;
2. aggregate fuzzy score descending;
3. preserved path ascending for a deterministic final tie.

This policy keeps learned navigation history decisive, compares frequency and
recency exactly as integers, and improves equal-history results without a
tuning constant. Query-term order remains observable but is provisionally
ignored until real usage can establish whether typed order expresses intent.

## Strategies and selection rule

The same 20 hand-labelled scenarios were evaluated with four strategies:

| Strategy | Definition | Expected top results |
| --- | --- | ---: |
| History only | History descending, then path | 10/20 |
| History then fuzzy | History descending, fuzzy descending, then path | 17/20 |
| Min-max combined | Normalise both signals within the result set; 80% history plus 20% fuzzy | 16/20 |
| Dense-rank fusion | Combine history and fuzzy dense-rank points at 4:1 | 17/20 |

A strategy is eligible for canonical use only if it is deterministic, monotonic
in history, and preserves the relative order of existing candidates when a
candidate inferior on both signals is added. Among eligible strategies, the
most expected results wins; a tie favours the simpler policy.

History-only, history-then-fuzzy, and rank fusion pass those invariants. Min-max
combination fails candidate-set independence: a history-10 candidate beats a
history-9 candidate in isolation, but adding a history-0 candidate changes the
normalisation range and makes history-9 the winner. History-then-fuzzy and rank
fusion tie on the curated corpus, so the former wins on simplicity.

Rank fusion also needs a policy weight. The curated win count is 17/20 at 1:1,
2:1, 4:1, and 9:1, but a focused six-candidate case selects the fuzzy leader at
4:1 and the history leader at 9:1. The stable aggregate count therefore does
not make the weight inconsequential.

## Scenario matrix

`tests/fixtures/ranking_histories.tsv` supplies real matcher inputs and complete
incremental records. Tests derive fuzzy scores, decay frecency at the stated
event tick, and use native integer comparisons for frequency and recency.

| Scenario | Mode | Expected behaviour | Observed result |
| --- | --- | --- | --- |
| `strong_frecency` | Frecency | Large history advantage wins | All pass |
| `close_frecency` | Frecency | Small history advantage wins | All pass |
| `tied_frecency` | Frecency | Tighter `docs` match breaks tie | All fuzzy-aware strategies pass |
| `realistic_pool` | Frecency | Frequently used `projects/zfz` wins five-path pool | All pass |
| `boundary_tie` | Frecency | Component-boundary `cp` match wins | All fuzzy-aware strategies pass |
| `deep_path_tie` | Frecency | Contiguous `/docs` beats `/d/o/c/s` | All fail; matcher over-rewards repeated boundaries |
| `repeated_component_tie` | Frecency | Contiguous repeated component wins | All fuzzy-aware strategies pass |
| `single_character_history` | Frecency | History controls a broad query | All pass |
| `long_query_tie` | Frecency | Shorter exact-containing path wins | All fail; matcher gives no trailing-length preference |
| `overlapping_terms_tie` | Frecency | Compact `doc oc` alignment wins | All fuzzy-aware strategies pass |
| `three_way_tie` | Frecency | Exact `api` component wins | All fuzzy-aware strategies pass |
| `combined_pressure` | Frecency | Best history survives a strong fuzzy alternative | Min-max alone fails |
| `frequency_clear` | Frequency | Visit count overrides recency/frecency | All pass |
| `frequency_tie` | Frequency | Fuzzy score breaks equal visits | All fuzzy-aware strategies pass |
| `frequency_complete_tie` | Frequency | Path makes a complete tie deterministic | All pass |
| `recency_clear` | Recency | Last tick overrides visits/frecency | All pass |
| `recency_tie` | Recency | Fuzzy score breaks equal ticks | All fuzzy-aware strategies pass |
| `recency_complete_tie` | Recency | Path makes a complete tie deterministic | All pass |
| `term_order_history` | Frecency | Term order cannot override history | Both order policies pass |
| `smart_case_tie` | Frecency | Shorter exact smart-case match wins | All fail; matcher gives no trailing-length preference |
| `term_order_neutral` | Frecency | Compare forward and reversed terms without a label | Ignore selects path order; tie-break selects query order |

The three misses shared by the selected strategy and rank fusion are fuzzy-score
limitations rather than interactions with history. They stay labelled as
undesirable results and feed the planned matcher reassessment.

## Generated checks

Deterministic tests supplement the curated corpus with all 24 input
permutations of a four-candidate set across every history mode and strategy,
generated history/fuzzy score grids, all tested rank-fusion weights, and
boundary cases. They establish:

- input order never affects output;
- increasing history cannot lower a canonical or rank-fusion candidate;
- increasing fuzzy quality cannot lower a candidate when history ties;
- adding a candidate inferior on both signals preserves pairwise order for all
  eligible strategies;
- empty and singleton inputs are valid;
- complete ties use preserved path order;
- invalid event clocks, non-finite frecency, and zero fusion weights fail
  explicitly;
- frequency and recency retain exact `u64` ordering above `f64`'s exact integer
  range.

## Deferred empirical check

After the tracking CLI and Fish integration can accumulate real histories,
re-evaluate query-term order and review mis-selections from normal use. This is
the appropriate point to challenge the provisional term-order policy and the
hand-labelled expectations; it does not block persistence benchmarking.
