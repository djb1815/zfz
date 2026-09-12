# zfz Tasks

Status: Initial implementation plan

This file defines the initial deliverables for taking zfz from design into benchmarking, prototyping, and a first usable implementation.

The emphasis is on resolving the open technical decisions with measurements before committing to architecture that will be expensive to change later.

---

## 1. Establish the project baseline

- [x] Create the initial Rust crate and repository structure.
- [x] Add basic development commands through `mise` where useful.
- [x] Establish release-build settings suitable for latency benchmarking.
- [x] Add a minimal CI workflow for formatting, linting, tests, and release builds.
- [x] Add representative test fixtures for directory histories and query cases.
- [x] Keep the shell integration separate from the Rust core from the beginning.

### Deliverable

A small buildable Rust project with enough structure to support benchmarks and prototypes without prematurely committing to the final architecture.

---

## 2. Reproduce and understand the frecency model

The initial ranking model should be based on the current `ze` scoring/decay behavior.

- [x] Inspect the current `ze` implementation and documentation.
- [x] Confirm the exact score update formula.
- [x] Confirm the decay formula and effective half-life.
- [x] Determine the minimum per-directory state required for incremental updates.
- [x] Confirm how frequency-only (`--rank`) and recency-only (`--time`) modes can be derived.
- [x] Review licensing and determine what may be reused directly versus independently implemented.
- [x] Encode the resulting behavior in focused unit tests.

### Deliverable

A documented and tested frecency implementation whose behavior is understood independently of the persistence layer.

---

## 3. Prototype fuzzy matching

The matcher should implement ordered-character fuzzy matching with AND semantics across query terms.

- [x] Define a small corpus of representative directory paths and queries.
- [x] Include cases covering:
  - contiguous matches;
  - non-contiguous ordered matches;
  - rejected out-of-order characters;
  - multiple AND terms;
  - query-term ordering;
  - path-component boundaries;
  - case behavior.
- [x] Investigate a Rust implementation of an fzf V2-style matcher.
- [x] Confirm licensing constraints around reuse or porting.
- [x] Implement or integrate the simplest viable matcher.
- [x] Expose match eligibility and fuzzy match score separately.
- [x] Verify that matching does not require or imply the final ranking formula.

### Deliverable

A matcher prototype with a stable test corpus and an independently observable fuzzy-match score.

---

## 4. Experiment with final ranking

Do not assume in advance how fuzzy-match quality should interact with frecency.

Compare at least:

1. frecency only;
2. frecency with fuzzy score as a secondary/tie-break signal;
3. an explicit combined fuzzy + frecency score.

- [x] Construct realistic synthetic histories with intentionally ambiguous candidates.
- [x] Add hand-written scenarios where the expected top result is easy to reason about.
- [x] Compare ranking strategies against those scenarios.
- [x] Evaluate whether query-term ordering should contribute to match quality.
- [x] Record surprising or undesirable results.
- [x] Select the simplest ranking strategy that produces consistently useful top results.
- [x] Harden the selected policy as the sole production ranking API with
  precomputed history keys and typed, non-mutating validation errors.
- [x] Retain rejected strategies and term-order evaluation in shared test
  support rather than application code.

### Deliverable

A concrete recommendation for the canonical candidate ranking pipeline, backed by examples rather than intuition alone.

---

## 5. Benchmark persistent storage

Compare the two currently viable persistence approaches:

- snapshot + append-only journal;
- embedded SQLite opened independently on each invocation.

The benchmark must use the same matching and ranking implementation for both. Hyperfine has been installed to aid benchmarking work.

### 5.1 Benchmark harness

- [x] Generate realistic synthetic datasets containing:
  - 100 records;
  - 1,000 records;
  - 5,000 records;
  - 10,000 records;
  - 50,000 records;
  - 100,000 records.
- [x] Measure release builds only.
- [x] Report distributions where practical: median, p95, p99, minimum, maximum.

### 5.2 Read path

Model:

`z <query>`

Include:

- [x] process startup;
- [x] storage open/load;
- [x] decoding/parsing;
- [x] fuzzy matching;
- [x] ranking;
- [x] best-result output;
- [x] process exit.

Record storage-related timings separately where practical.

### 5.3 Write path

Model one Fish `$PWD` update.

- [x] Measure single updates.
- [x] Measure bursts of 10, 100, and 1,000 updates.
- [x] Include open/load, update, commit/flush, close, and process exit.

### 5.4 Two-file-specific tests

- [x] Benchmark journal replay.
- [x] Benchmark snapshot compaction.
- [x] Benchmark atomic snapshot replacement.
- [x] Explore reasonable compaction triggers.

### 5.5 Concurrency and crash safety

For both implementations:

- [x] concurrent readers;
- [x] reader/writer overlap;
- [x] concurrent writers;
- [x] interruption during writes/commit;
- [x] verification that the next invocation can recover and continue.

### 5.6 Secondary measurements

- [x] Persistent storage size.
- [x] SQLite-related release binary size increase.
- [x] Warm-cache behavior.
- [x] Cold-cache behavior where reproducibly measurable.

### 5.7 Production-relevant SQLite follow-up

- [x] Compare the original rowid schema with `WITHOUT ROWID` at every dataset
  size for full queries, single updates, and database size.
- [x] Assess the cost of repeated journal-mode and schema setup separately from
  the schema change.
- [x] Exercise the candidate schema and production-like connection policy with
  the concurrency and crash-recovery suite.
- [x] Record intended read/write connection roles, a bounded tracking timeout,
  and the durability baseline for task 6.
- [x] Explicitly defer unrelated SQLite tuning that cannot affect the storage
  decision.

### Deliverable

A benchmark report recommending either SQLite or the snapshot/journal design, including the performance/complexity trade-off and the data supporting the decision.

No final persistence architecture should be selected before this work is complete.

---

## 6. Implement the core database abstraction

After the persistence decision:

- [ ] Define the directory record model.
- [ ] Implement the selected persistence backend.
- [ ] Support:
  - record/update visit;
  - load candidate records;
  - explicit add;
  - remove exact entry;
  - remove entry recursively;
  - stale-entry handling as currently defined.
- [ ] Preserve paths exactly as reported by Fish; do not canonicalize symlinks.
- [ ] Ensure concurrent invocations are safe.
- [ ] Ensure interrupted writes do not corrupt persistent state.

### Deliverable

A production-quality persistence layer supporting the operations required by the CLI.

---

## 7. Implement the initial query CLI

Build the core executable behavior around the established matcher, ranking system, and persistence layer.

- [ ] `z <query>` resolution path in the executable.
- [ ] `--echo` / `-e`.
- [ ] `--list` / `-l`.
- [ ] `--rank` / `-r`.
- [ ] `--time` / `-t`.
- [ ] `--add`.
- [ ] `--remove` / `-x`.
- [ ] `--remove-recursive` / `-X`.
- [ ] `--help` / `-h`.
- [ ] `-c` current-directory restriction.
- [ ] Choose the long-form name for `-c`.
- [ ] Define and validate incompatible option combinations.
- [ ] Decide whether administrative operations remain flags or become subcommands.

### Output safety

- [ ] Define safe normal output semantics for paths containing whitespace.
- [ ] Decide whether `--list` requires a null-delimited/machine-oriented mode.
- [ ] Ensure the Fish wrapper can consume selected paths without unsafe parsing.

### Deliverable

A usable standalone executable that can track, query, rank, list, add, and remove directory records.

---

## 8. Implement Fish-native integration

Keep this layer intentionally thin.

- [ ] Add a `$PWD` variable-change handler.
- [ ] Invoke the tracker after Fish changes directory.
- [ ] Ignore `$HOME` by default.
- [ ] Support configurable exact-path exclusions.
- [ ] Do not wrap `cd`.
- [ ] Do not depend on prompt execution.
- [ ] Do not introduce a daemon.
- [ ] Implement the user-facing `z` Fish function that:
  - queries the executable;
  - obtains the selected path;
  - performs `cd` in Fish.
- [ ] Verify that symlink paths remain uncanonicalized end-to-end.

### Deliverable

The first genuinely usable version of zfz:

- directory visits are tracked automatically;
- `z <query>` jumps to the best candidate;
- the shell integration remains Fish-native and minimal.

---

## 9. Add interactive fzf selection

After normal automatic navigation is solid:

- [ ] `z` with no query launches interactive selection.
- [ ] `-i` / `--interactive` forces interactive selection.
- [ ] Feed the canonical ranked candidate set into fzf.
- [ ] Preserve unusual paths safely across the Rust → fzf → Fish boundary.
- [ ] Decide whether fzf is invoked by the Rust executable or the Fish layer.
- [ ] Ensure cancellation leaves the current directory unchanged.

### Deliverable

A robust fzf-backed interactive workflow using the same candidate ranking as normal navigation and `--list`.

---

## 10. Hardening and initial release readiness

- [ ] Define behavior for missing/stale directories.
- [ ] Add integration tests for concurrent invocations.
- [ ] Add tests for unusual valid path names.
- [ ] Add tests for ignored paths.
- [ ] Add tests for symlink-preserving behavior.
- [ ] Add end-to-end Fish integration tests where practical.
- [ ] Profile cold-start latency of the final executable.
- [ ] Confirm the common path remains effectively instantaneous for realistic histories.
- [ ] Review CLI help and error messages.
- [ ] Update design documentation only after implementation decisions are settled.

### Deliverable

A coherent first release candidate suitable for personal daily use.

---

# Initial milestone sequence

The recommended order is:

1. **Frecency prototype**
2. **Fuzzy matcher prototype**
3. **Ranking experiments**
4. **Storage benchmark**
5. **Persistence decision**
6. **Core CLI**
7. **Fish tracking + automatic navigation**
8. **fzf interactive mode**
9. **Hardening**

The key architectural checkpoint is the end of step 4. At that point the project should have enough evidence to settle the two most important implementation choices:

- how candidates are ranked;
- how directory state is persisted.

Everything after that should primarily be implementation rather than architecture discovery.

---

# Deferred work

The following should not block the first usable version:

- Fish tab-completion integration;
- fzf-style query operators;
- wildcard/subtree ignore syntax;
- multiple fuzzy matching algorithms;
- extensive customization of matching weights;
- broad compatibility with every rupa/z, ze, zsh-z, or zoxide option;
- daemon/background operation;
- tracking complete visit history;
- elaborate indexing before benchmarks demonstrate a need.
