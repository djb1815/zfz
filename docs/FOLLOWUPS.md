# zfz Follow-ups

Deferred work and design questions that should be revisited when the relevant
prototype or benchmark results are available.

## Reassess the fuzzy-matcher dependency

**When:** After matcher and storage benchmarks, before committing to the
production matcher.

Task 3 uses `fuzzy-matcher` 0.3.7's `SkimMatcherV2` as a small, MIT-licensed
fzf V2-style prototype dependency. It is suitable for evaluating the matching
and ranking pipeline, but it should not be assumed to be the permanent choice.

- Its smart-case matching and case folding are ASCII-only; for example, `écl`
  does not match `Éclair`.
- The crate does not appear to be actively maintained: 0.3.7 was released in
  2020.
- The current `i64` fuzzy-score type follows the dependency's default API rather
  than observed score requirements. When replacing or finalising the matcher,
  choose the score representation deliberately, account for multi-term
  aggregation, and do not assume numeric score parity between implementations.
- Its practical matching behavior should continue to be assessed against the
  corpus; exact score parity with fzf is not a goal.
- Ranking scenarios found that component bonuses can make `/d/o/c/s` score
  above a contiguous `/docs` match.
- Exact query text embedded in a longer path can tie the shorter component-only
  path because trailing candidate length is not penalised; deterministic path
  order then selects between them.

Evaluate whether these limitations matter for real directory histories and
benchmark the alternatives using the same corpus. Reasonable options are to
retain the dependency, replace it with a maintained Rust matcher, or implement
the required fzf-style subset in zfz with explicit Unicode and parity tests.

## Reassess release executable size

**When:** After the production storage and CLI are implemented, before the
first distribution release.

The task 5 follow-up audit reduced the bundled-SQLite benchmark harness from
2.24 MB to 1.73 MB by disabling unused optional SQLite facilities without a
measurable latency regression. Compiling SQLite's C code for size reduced it
further to 1.20 MB but added about 0.8 ms to a representative 10,000-record
read. Optimising the entire Rust application for size caused a much larger
matching/ranking regression and should not be used based on current evidence.

Reassess the final stripped production executable rather than extrapolating
from the benchmark harness. Compare normal and C-only size optimisation on all
supported release targets, inspect linked sections/symbols for additional safe
savings, and retain bundled SQLite unless a system-library distribution model
can provide equally predictable installation and behaviour. Any custom SQLite
compile configuration must run the complete storage concurrency and recovery
suite.
