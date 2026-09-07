# zfz Design

> **Status:** Living design document  
> **Project:** `zfz`  
> **User-facing command:** `z`

This document captures the current design of **zfz**, a small, fast, Fish-native directory jumper inspired primarily by rupa's `z`, with fuzzy matching and a modern continuously decaying frecency model.

It is intended to be committed to the repository and updated as benchmarking, prototyping, and implementation resolve open questions. It distinguishes **established requirements** from **designs still under evaluation**; unresolved points should not be treated as accidental omissions.

## 1. Goals

zfz should provide the simplicity and predictability of a classic `z`-style directory jumper while improving two areas deliberately:

1. **Fish-native tracking** — directory changes are observed through Fish's `$PWD` variable-change event rather than by wrapping `cd`, running prompt hooks, or keeping a daemon alive.
2. **Fuzzy navigation** — query terms use ordered-character fuzzy matching rather than classic `z` substring matching, while historical usage remains an important ranking signal.

The common path should remain simple:

```fish
z docs
```

This should select the directory that zfz determines the user most likely intended and change to it with minimal latency.

Later versions may support additional shell integrations such as ZSH or Bash.

## 2. Design Principles

The project should be guided by the following principles:

- **Optimise the common interactive path.** `z <query>` should feel effectively instantaneous.
- **Keep Fish integration thin.** Shell-specific code observes `$PWD`, invokes the compiled core, performs `cd`, and integrates with fzf/completions.
- **Track navigation, not filesystem identity.** Preserve the path reported by Fish rather than canonicalising it.
- **Separate matching, match quality, and history ranking.** These are distinct signals and should remain independently understandable.
- **Prefer established algorithms.** Reuse or independently implement proven matching/frecency ideas rather than inventing complexity without evidence.
- **Prefer simple persistence unless measurements justify complexity.** The storage architecture is intentionally not fixed before benchmarking.
- **Make correctness under concurrency and crashes a requirement.** Persistence must not trade data integrity for small latency gains.
- **Avoid feature parity for its own sake.** Familiarity with `z`, `zsh-z`, and `ze` is useful, but compatibility does not override this project's design.
- **Let implementation experience change this document.** When prototypes reveal a better design, update the design explicitly rather than preserving obsolete assumptions.

## 3. High-Level Architecture

The intended responsibility split is:

```text
Fish integration
  ├─ observe $PWD changes
  ├─ invoke tracking/update operation
  ├─ expose user-facing `z` function/command
  ├─ perform `cd` to selected path
  ├─ invoke/present fzf where appropriate
  └─ provide Fish completion integration (later)

Rust core
  ├─ persistent state
  ├─ directory updates/removals
  ├─ candidate loading
  ├─ fuzzy matching
  ├─ fuzzy match scoring
  ├─ frecency/frequency/recency ranking
  ├─ canonical ordered result set
  └─ CLI/output modes
```

The core should be a short-lived compiled process. No background service or persistent database process is part of the design.

Rust is the initial implementation language.

## 4. Fish Integration

### 4.1 Directory tracking

Fish should observe changes to `$PWD` directly:

```fish
function __z_on_pwd --on-variable PWD
    # invoke zfz tracking operation with $PWD
end
```

The implementation must not:

- wrap or replace `cd`;
- depend on prompt execution;
- require a daemon.

A tracking update should use the directory reported by Fish after the `$PWD` change.

### 4.2 Preserve paths as navigated

Paths must be persisted as supplied by `$PWD`.

In particular:

- do not resolve/canonicalise symlinks;
- two navigational paths that refer to the same physical directory may have independent history;
- do not collapse entries based on inode or canonical filesystem identity.

This is intentional: zfz models the paths the user navigates through, not the underlying filesystem object graph.

### 4.3 Navigation interface

The Fish-facing `z` wrapper should be thin. For normal navigation it should obtain a selected path from the compiled core and then perform `cd` itself.

The exact executable/wrapper naming and transport mechanism are implementation details, but an echo-style interface is expected to be sufficient conceptually:

```fish
cd (zfz --echo docs)
```

The real wrapper must handle failure and unusual path contents safely rather than blindly relying on this illustrative form.

If no suitable match exists, the command must fail without changing the current directory.

## 5. Directory Records and Historical Ranking

### 5.1 Default frecency model

The initial default ranking model should follow the frecency approach used by `ze`: a visit contributes to a directory's score and that contribution decays exponentially over time.

The initial implementation should begin from `ze`'s current scoring formula and decay parameters/half-life rather than inventing new constants, subject to confirming licensing and deciding what must be independently reimplemented.

The model should avoid classic `z`'s periodic global score decay in favour of continuous time-based decay.

### 5.2 Incremental state

The database should not need to retain a complete visit log. It should store enough per-directory state to update ranking incrementally.

The logical per-directory record requires:

```text
path
frecency_score
last_visit_tick
visit_count
```

The persistence benchmark may choose an appropriate encoding, but it must preserve these values. A global event-clock tick is also required to decay frecency at query time.

### 5.3 Ranking modes

The intended modes are:

- **default:** normal decaying frecency;
- **`-r`, `--rank`:** total visit count descending;
- **`-t`, `--time`:** last event-clock tick descending.

Frequency and recency use their native integer values rather than converting them to floating point. Whether their CLI options are mutually exclusive remains to be established with the CLI design.

### 5.4 Ranking qualities

Whichever formula is used should remain:

- deterministic;
- small enough to understand;
- explainable;
- cheap at query time.

Avoid opaque ranking machinery or extensive tuning controls unless real usage demonstrates a need.

## 6. Matching

### 6.1 Matching and ranking are separate stages

A candidate's path can provide three conceptually separate signals:

1. **Match eligibility:** does it satisfy every query term?
2. **Fuzzy match quality:** how naturally/tightly do the query characters align with the path?
3. **Directory history:** how strongly do frecency/frequency/recency favour the directory?

The conceptual pipeline is:

```text
query
  ↓
candidate matching
  ↓
matching candidates
  ├─ fuzzy match quality ──┐
  └─ history score ────────┤
                           ↓
                     final ranking
```

Exactly how fuzzy quality and history score combine is intentionally an experimental question.

### 6.2 Single-term fuzzy matching

Each query argument is matched against the candidate path using **ordered-character fuzzy matching**.

Characters must occur in the same order but do not need to be contiguous.

For `~/Documents`:

| Query | Match | Reason |
|---|---|---|
| `docs` | yes | ordered characters, effectively contiguous |
| `doc` | yes | ordered characters |
| `dcs` | yes | ordered subsequence |
| `sdoc` | no | character order is wrong |

This is deliberately more permissive than classic `z`/`zsh-z`; for example, `docs` should match `Documents`.

### 6.3 Multiple terms use AND semantics

Whitespace-separated shell arguments are independent query terms. A candidate must match **all** terms.

For example:

```fish
z docs proj
```

may match both:

```text
~/Documents/projects/foo
~/projects/foo/Documents
```

The terms do not need to occur in query order for the candidate to be eligible.

This deliberately differs from classic `z`'s effective ordered `*docs*proj*` behaviour.

### 6.4 Term order is provisionally observational only

Term ordering is not an eligibility rule. The matcher records whether its chosen alignments occur in query order. The initial canonical ranking does not award a separate bonus, pending evaluation against real navigation choices.

For:

```fish
z docs proj
```

The ranking experiment confirmed that the forward and reversed forms can receive the same aggregate fuzzy score. In that exact-tie case, ignoring order falls through to path order while an order-aware variant selects the forward form. Neither outcome was hand-labelled as correct: typed order may express intent, but synthetic data cannot establish that it does. The signal is retained for evaluation against real usage later.

### 6.5 Preferred fuzzy algorithm

The current preferred candidate for matching/scoring is **fzf V2** because it provides ordered-character fuzzy matching plus an established optimal-alignment score.

The prototype should investigate whether:

- an existing Rust implementation is suitable and licensable;
- fzf V2 can be independently implemented cleanly;
- its scoring behaves naturally for directory paths.

The initial product does **not** need multiple selectable fuzzy algorithms.

`fzy` remains useful background material, particularly for its framing of matching versus scoring and its references to other algorithms, but it should not be treated as the specification for this matcher.

### 6.6 Path-specific score signals

The prototype should observe the effects of signals such as:

- path-component boundaries;
- word boundaries;
- contiguous character runs;
- beginning-of-component matches;
- position within the full path;
- case transitions;
- overall compactness of the alignment.

These should not become a custom scoring formula unless experiments show that fzf-style behaviour is inadequate for directory navigation.

### 6.7 Query syntax

The initial implementation should not introduce an fzf-style query language.

Do not initially implement operators for:

- exact matching;
- prefix/suffix matching;
- inverse matching;
- other operator-controlled modes.

Fish quoting, escaping, and glob syntax already provide enough surface area; operator syntax can be reconsidered later if a real use case emerges.

### 6.8 Case behaviour

Case-sensitive versus case-insensitive/smart-case behaviour is still open and should be tested against fzf/classic-z expectations.

## 7. Final Candidate Ordering

The project uses one canonical ordering of matched directories:

1. the selected history score descending;
2. aggregate fuzzy-match score descending, only when history ties;
3. preserved path ascending as a deterministic final tie-breaker.

This is the only production ranking policy. Ranking decorates each candidate
with one mode-specific history key, preserving native `u64` values for
frequency and recency, before sorting. Frecency ranking validates the complete
input and returns a typed error for a backwards event clock or non-finite score;
an error leaves candidate order unchanged. Experimental strategies and query-
term-order tie-breaking live only in test support.

That same ordering should feed all consumption modes:

```text
matching + ranking
  ├─ normal navigation  → select top result
  ├─ --list             → emit ordered results
  └─ --interactive      → present ordered candidates to fzf
```

The ranking experiment compared history-only ordering, history with fuzzy quality as a secondary signal, a normalised 80:20 combined score, and 4:1 weighted dense-rank fusion across frecency, frequency, and recency modes. History-primary/fuzzy-secondary and rank fusion each selected 17 of 20 hand-labelled results; the former was selected because it achieves that result without a weighting policy.

Min-max combination made results depend on the range of other eligible candidates: adding a low-history candidate could change which of two existing candidates ranked first. Rank fusion avoided that failure, but a focused scenario changed its top result between 4:1 and 9:1 history weighting. Neither complexity improved on history-primary/fuzzy-secondary ordering.

The reproducible strategies, fixtures, detailed examples, and recommendation are recorded in [`ranking.md`](ranking.md).

The first priority is the quality and predictability of the **top result**, because that is what normal navigation selects automatically.

## 8. CLI

### 8.1 CLI philosophy

The CLI should feel familiar to users of rupa `z`, `zsh-z`, and `ze`, but exact compatibility is not a goal.

Short options should be retained where useful and familiar, while long options improve discoverability.

The common form remains:

```text
z <query>
```

### 8.2 Initial option set

| Short | Long | Purpose |
|---|---|---|
| — | `--add` | Explicitly add/update a directory in the database |
| `-c` | **TBD** | Restrict matches to directories beneath the current directory |
| `-e` | `--echo` | Print the selected path without navigating |
| `-i` | `--interactive` | Interactively select a result using fzf |
| `-h` | `--help` | Display help |
| `-l` | `--list` | List matching directories without navigating |
| `-r` | `--rank` | Rank by visit frequency |
| `-t` | `--time` | Rank by recency |
| `-x` | `--remove` | Remove one directory from the database |
| `-X` | `--remove-recursive` | Remove a directory and its descendants |

The long name for `-c` remains open.

Administrative operations may later become subcommands (`z add`, `z remove`) if this proves clearer without introducing ambiguity with query terms. Do not add subcommands merely for stylistic consistency.

### 8.3 Normal navigation

```fish
z docs
```

should:

1. match candidate directories;
2. rank candidates using the selected/default ranking mode;
3. select the top candidate;
4. cause Fish to change to that path.

### 8.4 No-argument invocation

```fish
z
```

should launch the fzf-based interactive workflow over the relevant directory history.

### 8.5 Explicit interactive mode

```fish
z -i docs
z --interactive docs
```

should filter/rank normally, then present matching candidates to fzf and navigate to the selected result.

The earlier draft spelling `-f` is superseded by `-i`.

### 8.6 Echo mode

```fish
z --echo docs
```

should resolve the best matching path and emit it without changing directory.

This is also a likely primitive for the Fish wrapper.

### 8.7 List mode

```fish
z --list docs
```

is a first-class composability feature, not merely diagnostic output.

It should:

- perform normal matching;
- apply the requested ranking mode;
- emit all matching directories in canonical order;
- never change directory.

A user should be able to feed this output into custom fzf pipelines or other CLI tools.

Output must therefore have a safe story for whitespace and unusual characters. Whether normal line-oriented output is supplemented by a null-delimited/machine mode remains open.

### 8.8 Administrative operations

Required behaviours:

- `--add PATH`: explicitly add/update an entry;
- `-x/--remove PATH`: remove exactly one entry;
- `-X/--remove-recursive PATH`: remove the path and its descendants.

Recursive removal is an explicit database operation and is unrelated to ignored-path semantics.

### 8.9 Option conflicts

The precise validation rules for combinations such as `--list`, `--echo`, and `--interactive` are not yet fixed. They should be made explicit before the CLI is considered stable.

## 9. Interactive fzf Integration

fzf is an external interactive selector, not the persistent store or canonical matcher.

The expected workflow for `--interactive` is:

1. load directory records;
2. apply zfz's candidate matching;
3. apply zfz's canonical ranking;
4. provide ranked candidates to fzf;
5. read the selected path;
6. return it to the Fish wrapper for navigation.

The integration must preserve arbitrary valid path content safely.

Future Fish tab-completion integration with fzf is explicitly deferred until the core command is established.

## 10. Ignored Directories

### 10.1 Configurable ignored paths

zfz must support configurable ignored directories.

### 10.2 Exact-path semantics by default

Ignoring a path means ignoring that **exact path only**.

For example, ignoring:

```text
/home/user
```

must not implicitly ignore:

```text
/home/user/projects
/home/user/projects/foo
```

This is particularly important because `$HOME` should be ignored by default.

### 10.3 Subtree exclusion

Pattern- or wildcard-based subtree exclusion may be added later, but it is not a core requirement and should not complicate the initial implementation.

### 10.4 Normalisation

Configuration format, path expansion, and the exact normalisation performed before comparing ignored paths remain open. Any normalisation must not undermine the separate requirement to persist navigated paths without symlink canonicalisation.

## 11. Stale Directories

Tracked directories may later cease to exist.

The final policy is open. Candidate behaviours include:

- retain stale entries and let their score decay naturally;
- ignore nonexistent entries during navigation but retain them in storage;
- remove stale entries opportunistically;
- expose explicit cleanup behaviour.

The initial implementation should prefer a simple, predictable policy and must never `cd` to a path that is no longer usable.

## 12. Persistent Storage

Persistence is the major architecture decision intentionally left open for benchmarking.

Two designs are currently in scope.

### 12.1 Option A: snapshot + journal

A custom two-file design consisting of:

1. a compact snapshot containing the current record state;
2. an append-only journal containing incremental updates;
3. periodic compaction of journal updates into a replacement snapshot.

The journal should represent incremental state updates rather than a permanent complete visit history wherever practical.

Potential advantages:

- simple application-specific format;
- minimal dependency footprint;
- potentially very cheap reads for small histories;
- control over decoding and layout.

Costs/risks to evaluate:

- custom locking/coordination;
- safe concurrent appends;
- crash recovery;
- compaction correctness;
- atomic snapshot replacement;
- journal replay overhead.

### 12.2 Option B: SQLite

A normal embedded SQLite database opened and closed by each Rust invocation.

There is **no SQLite daemon** in this design.

A query/update invocation would conceptually:

1. start the Rust process;
2. open the database;
3. perform the operation;
4. commit if needed;
5. close the database;
6. exit.

The initial schema should be deliberately simple. Specialized fuzzy matching/ranking should remain in Rust; SQLite does not need to perform the matcher unless later evidence suggests an advantage.

Potential advantages:

- mature transactions;
- mature locking/concurrency semantics;
- crash recovery;
- incremental writes;
- less custom persistence code.

Costs to evaluate:

- open/initialisation overhead for short-lived processes;
- read/row-decoding overhead versus a compact file;
- dependency and binary-size impact;
- operational files such as rollback journal/WAL where applicable.

### 12.3 Selection principle

Prefer the simpler overall solution unless the alternative provides a meaningful practical advantage.

"Simpler" includes correctness code: a custom format that saves a small amount of read latency but requires substantial bespoke locking and crash-recovery logic may be less simple overall than SQLite.

No persistence architecture should be treated as final until the benchmark below has been run.

## 13. Persistence Benchmark Plan

The benchmark should model zfz's actual short-lived workload rather than general database throughput.

### 13.1 Dataset sizes

Run against realistic synthetic directory histories of:

- 100 records;
- 1,000 records;
- 5,000 records;
- 10,000 records;
- 50,000 records;
- 100,000 records.

Synthetic paths should resemble real filesystem paths. The smaller sets are the most representative; the larger sets establish scaling behaviour.

### 13.2 Read benchmark

Model the complete:

```text
z <query>
```

path:

```text
process startup
  ↓
storage open/initialisation
  ↓
load/retrieve records
  ↓
decode/parse records
  ↓
fuzzy match
  ↓
score/rank
  ↓
select/output
  ↓
process exit
```

At minimum report:

- end-to-end latency;
- storage open/load time;
- record decoding/parsing time;
- matching/ranking time.

The matching/ranking implementation and dataset must be identical for both persistence implementations.

### 13.3 Write benchmark

Model a Fish `$PWD` update.

Measure:

- end-to-end update latency;
- storage open/load overhead;
- update/write time;
- commit/flush cost;
- close/exit time.

Test both isolated and sequential update workloads:

- 1 update;
- 10 updates;
- 100 updates;
- 1,000 updates.

The important question is whether synchronous persistence on every directory change remains imperceptible in normal shell use.

### 13.4 Journal replay and compaction

For the two-file design, benchmark journals with progressively larger numbers of updates.

Measure:

- replay time;
- new snapshot construction time;
- replacement snapshot write time;
- total compaction time;
- pre/post-compaction storage size.

Evaluate compaction triggers such as:

- journal entry count;
- journal byte size;
- journal-to-snapshot size ratio.

Compaction must be treated as real work, not hidden from the comparison.

### 13.5 Warm and cold cache

Where practical distinguish:

- warm filesystem-cache measurements;
- cold-cache measurements.

If reliable cache eviction is not practical on the benchmark platform, document the measurement conditions rather than pretending the runs are cold.

### 13.6 Concurrency

Use multiple short-lived processes to test:

**Concurrent readers**

- all readers observe valid complete state;
- no reader can observe a partially written snapshot.

**Reader/writer overlap**

- queries continue to see a consistent representation while updates occur;
- no partial/corrupt update becomes visible.

**Concurrent writers**

- independent updates are not silently lost;
- final state reflects all logically successful updates.

For the two-file design this should expose the true cost/complexity of custom coordination. For SQLite, built-in locking/transaction behaviour should be measured rather than assumed to be free.

### 13.7 Crash safety

Deliberately terminate writers at useful points.

For snapshot+journal, test interruption during:

- journal append;
- compaction;
- snapshot replacement.

For SQLite, test interruption during:

- update;
- transaction commit.

After each simulated crash verify:

- storage remains readable;
- committed updates remain present;
- incomplete updates are recoverable or safely discarded;
- no silent corruption occurs;
- the next normal invocation succeeds.

Crash safety is a functional requirement, not merely a benchmark dimension.

### 13.8 Storage and binary size

Record:

- snapshot size;
- journal size before/after compaction;
- SQLite DB plus auxiliary journal/WAL files;
- release binary size without SQLite;
- release binary size with SQLite;
- stripped binary sizes.

These are secondary criteria unless the differences become substantial.

### 13.9 Measurement quality

Use representative release builds on the same machine/filesystem/compiler configuration.

Report distributions where practical:

- minimum;
- median;
- p95;
- p99;
- maximum.

Do not base the decision on a single run or debug-build timing.

## 14. Initial Prototype Work

The first implementation phase should be exploratory and should minimise coupling between decisions that are still open.

A useful sequence is:

1. **Define the in-memory directory record and update API.** Keep persistence behind an interface so both benchmark stores exercise the same core behaviour.
2. **Confirm the `ze` frecency formula and licensing constraints.** Implement the smallest independent scoring/update module possible.
3. **Prototype fuzzy matching.** Start with fzf V2 or the closest clean Rust implementation; preserve per-candidate match scores even if final ranking initially ignores them.
4. **Create deterministic synthetic datasets and query fixtures.** Include realistic paths and examples designed to distinguish fuzzy-quality behaviours.
5. **Implement both persistence prototypes.** Keep schemas/formats deliberately simple and equivalent in represented information.
6. **Build end-to-end benchmark commands.** Measure real short-lived process invocation rather than only Criterion-style in-process functions.
7. **Exercise concurrency and crash tests.** Do this before declaring a custom file format "simpler" than SQLite.
8. **Compare final-ranking strategies.** Use realistic query/history fixtures to compare frecency-only, secondary fuzzy score, and combined scoring.
9. **Choose the storage design.** Record measured results and rationale in this document (or an ADR linked from it).
10. **Stabilise the Fish wrapper and public CLI.** Resolve output encoding, option conflicts, and administrative syntax once the core behaviours are proven.

Where possible, benchmark scaffolding and fixtures should remain in the repository so future changes can be checked against the original design goals.

## 15. Suggested Internal Boundaries

These are architectural guidance rather than fixed Rust module names:

```text
core/
  record          directory state and update semantics
  frecency        ze-style decay and ranking modes
  matcher         candidate eligibility + fuzzy match quality
  ranking         combines history and match signals

storage/
  trait/interface common persistence operations
  journal         snapshot+journal prototype
  sqlite          SQLite prototype

cli/
  argument parsing
  output modes
  administrative operations

fish/
  PWD event hook
  user-facing z wrapper
  fzf integration
  completions (later)

bench/
  dataset generation
  end-to-end runner
  concurrency/crash fixtures
  result capture
```

Important boundaries to preserve:

- matching should not know how records are persisted;
- storage should not own fuzzy ranking policy;
- ranking should be testable using in-memory records;
- Fish integration should not duplicate core matching/ranking logic.

## 16. Out of Scope for the Initial Implementation

The initial implementation should not include:

- a background daemon;
- wrapping/replacing `cd`;
- prompt-based tracking;
- symlink canonicalisation;
- automatic ignored-subtree semantics;
- complete historical visit logs;
- a custom fuzzy query/operator language;
- multiple user-selectable fuzzy algorithms;
- file jumping/opening;
- tab-completion integration before the core command is stable;
- elaborate indexing without benchmark evidence;
- broad feature parity with unrelated directory-jumping tools.

## 17. Open Design Questions

The following are intentionally unresolved and should be updated as implementation provides evidence.

### Matching and ranking

- Can fzf V2 be reused or independently implemented cleanly and compatibly in Rust?
- What case-sensitivity/smart-case policy should apply?
- Should query-term order break otherwise complete history/fuzzy ties once real usage can be evaluated?
- Do path-component-specific signals need adjustment beyond the chosen fuzzy algorithm?
- Should classic `z`/`zsh-z` common-root selection behaviour be retained, modified, or removed?
- Is comparing fzf V2 with fzy worthwhile after the first matcher works?

### Persistence

- Snapshot+journal or SQLite?
- If snapshot+journal wins, what format and locking mechanism should be used?
- What compaction trigger is appropriate?
- If SQLite wins, which Rust binding/linking strategy gives the desired distribution properties?
- What database/storage location and migration/versioning strategy should be used?

### CLI and output

- What is the long-form name for `-c`?
- Options, subcommands, or both for administrative operations?
- What output contract should `--list` provide for arbitrary path contents?
- Is a null-delimited mode needed?
- Which output-mode flags conflict?
- Are `--rank` and `--time` strictly mutually exclusive?

### Configuration and lifecycle

- Configuration file format and location.
- Ignored-path expansion and comparison rules.
- Default ignored paths beyond `$HOME`, if any.
- Stale/nonexistent-directory cleanup policy.
- Whether explicit subtree-ignore patterns are ever needed.

### Fish/fzf integration

- Exact executable/wrapper naming and return-path protocol.
- How cancellation/no-selection should map to exit codes.
- How arbitrary path contents are transferred safely through fzf and Fish.
- Future completion integration design.

## 18. Definition of a Successful Initial Implementation

The first usable version should demonstrate that:

- Fish `$PWD` events can update history cheaply and reliably;
- navigated symlink paths are preserved;
- `z <query>` supports ordered-character fuzzy AND matching;
- the top result is ranked predictably using the selected initial history strategy;
- `z` with no args and `z -i <query>` can select through fzf;
- `--echo` and `--list` provide useful non-navigation interfaces;
- ignored exact paths, including `$HOME` by default, are respected;
- persistence is safe under normal concurrent invocations and process interruption;
- realistic histories remain comfortably fast;
- the chosen storage architecture is backed by repository-contained benchmark evidence rather than intuition alone.

The project can then iterate on match-quality weighting, CLI details, cleanup policy, and completion integration without needing to redesign its fundamental boundaries.
