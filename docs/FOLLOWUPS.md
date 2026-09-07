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
