# Persistent-storage benchmark

## Recommendation

Use **embedded SQLite in rollback-journal (`DELETE`) mode**, opened and closed
by each zfz invocation, with full synchronous durability and matching/ranking
remaining in Rust.

Use a `WITHOUT ROWID` records table in the production schema. A post-benchmark
size audit found that the prototype's ordinary rowid table duplicated its text
primary key in a separate unique index. Removing that duplication makes the
SQLite database approximately the same size as the custom snapshot.

The snapshot+journal prototype reads faster, but not by enough at realistic
history sizes to justify its substantially slower writes and bespoke recovery,
locking, and compaction machinery. At 10,000 records its representative read
p95 was 9.15 ms versus SQLite's 11.65 ms: a 2.50 ms absolute improvement but
only 21% relative to SQLite, below the experiment's predeclared 25% threshold.
SQLite's write p95 was 3.55 ms versus 17.45 ms, and its cost stayed flat as the
dataset grew. It also recovered correctly through every concurrency and crash
test with much less application-owned correctness code.

WAL was not selected. It added roughly 0.4--0.6 ms to common reads and writes
without improving this short-lived, write-then-close workload. This does not
preclude revisiting WAL if later workloads keep connections open or sustain
heavy reader/writer overlap.

## Method

The benchmark-only crate in [`../benchmarks/storage`](../benchmarks/storage)
contains equivalent stores behind one interface. Both preserve the same path,
visit count, last tick, frecency score, and global clock. Both query paths load
all records and call the production matcher and canonical ranker. SQLite does
not perform matching or ranking.

The deterministic generator produced 100, 1,000, 5,000, 10,000, 50,000, and
100,000-record histories with nested project-like paths, ambiguous components,
spaces, punctuation, Unicode, tabs/newlines, and symlink-like spelling. Four
query classes covered broad matches, focused matches, multiple AND terms, and
misses. Every normal read and single-write distribution used 10/5 warmups and
100 measured release-process invocations. Burst tests launched 10, 100, or
1,000 independent update processes. All timings below are milliseconds.

Measurements ran on 2026-09-07 on an 8-core Apple M2 MacBook Air with 8 GB RAM,
macOS 26.6.2, an APFS project volume, Rust 1.98.1, bundled SQLite through
`rusqlite` 0.37.0, and Hyperfine 1.20.0. The reproducible runner emits raw
Hyperfine samples, internal phase samples, sizes, and environment metadata
beneath the ignored `benchmarks/storage/results/` directory. The measured run
was archived separately for external attachment rather than committed as more
than 100 individual repository files.

A bounded follow-up on 2026-09-12 used the same machine, compiler, generated
histories, query classes, release profile, warm-cache conditions, and 100-run
distributions. It compared only the original SQLite schema, `WITHOUT ROWID`,
and production-like connection setup. The reproducible runner is
`benchmarks/storage/run-sqlite-followup.sh`; it deliberately does not vary
durability or unrelated SQLite settings. Its raw results are archived
separately under the same external-evidence policy as the original run.

## Results

Representative end-to-end `projects` query distributions:

| Records | Backend | Min | Median | p95 | p99 | Max |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1,000 | snapshot+journal | 2.74 | 2.83 | 2.99 | 3.04 | 3.05 |
| 1,000 | SQLite DELETE | 3.16 | 3.28 | 3.38 | 3.45 | 3.53 |
| 1,000 | SQLite WAL | 3.48 | 3.68 | 3.89 | 4.05 | 4.06 |
| 10,000 | snapshot+journal | 8.69 | 8.90 | 9.15 | 9.23 | 9.26 |
| 10,000 | SQLite DELETE | 11.11 | 11.42 | 11.65 | 11.75 | 11.83 |
| 10,000 | SQLite WAL | 11.62 | 11.95 | 12.21 | 12.27 | 12.35 |

At 100,000 records, representative medians were 63.25 ms for snapshot/journal,
85.09 ms for SQLite DELETE, and 85.74 ms for WAL. Internal timing showed that
matching/ranking itself took about 55 ms; SQLite row retrieval/decoding added
about 26 ms versus about 5.4 ms for snapshot read plus decode. The result
therefore reflects full Rust-side candidate loading rather than an indexed
SQLite matcher.

Single-update distributions:

| Records | Backend | Min | Median | p95 | p99 | Max |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1,000 | snapshot+journal | 6.28 | 12.19 | 19.71 | 30.82 | 37.10 |
| 1,000 | SQLite DELETE | 2.81 | 3.13 | 3.34 | 3.60 | 5.32 |
| 1,000 | SQLite WAL | 3.27 | 3.79 | 4.25 | 4.56 | 4.89 |
| 10,000 | snapshot+journal | 6.96 | 12.91 | 17.45 | 22.19 | 24.55 |
| 10,000 | SQLite DELETE | 2.74 | 3.06 | 3.55 | 3.82 | 4.40 |
| 10,000 | SQLite WAL | 3.08 | 3.41 | 3.65 | 3.75 | 3.86 |

SQLite DELETE's 1,000-update burst median was 2.94 seconds at 1,000 records and
2.91 seconds at 100,000. Snapshot+journal took 11.35 and 16.90 seconds
respectively because each safe writer reloaded snapshot and journal state. WAL
took 3.30 and 3.22 seconds.

For a 10,000-record snapshot (832,577 bytes), replay grew from a 9.05 ms median
with no journal to 9.27 ms at 1,000 entries and 11.06 ms at 10,000 entries. The
10,000-entry journal was 631,394 bytes. Compaction medians ranged from 26.7 to
32.2 ms in the end-to-end runs. The instrumented 10,000-entry case attributed
about 0.13 ms to encoding, 21.62 ms to temporary-file write/flush and journal
reset, and 10.16 ms to atomic rename plus directory sync. If this design were
used, a journal/snapshot ratio of 10% (with an approximately 2,000-entry cap)
would keep observed replay overhead below roughly 0.5 ms, but no production
trigger is needed for the selected backend.

### Post-benchmark size audit

The original size comparison used the prototype schema's ordinary rowid table:

```sql
CREATE TABLE records (
    path TEXT PRIMARY KEY,
    visits INTEGER NOT NULL,
    last_tick INTEGER NOT NULL,
    score REAL NOT NULL
);
```

For a non-integer primary key, this stores the complete record in a rowid table
and the path again in an automatically created unique index. The measured
stores were not inflated by repeated benchmark writes: the size report examined
the original stores, while every write benchmark modified a copy. Freshly
regenerated 10,000- and 100,000-record databases had no freelist pages.

The follow-up compared that schema with the same table declared `WITHOUT
ROWID` at every existing dataset size:

| Records | Snapshot | Original SQLite | `WITHOUT ROWID` | After `VACUUM` |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 8,354 | 32,768 | 20,480 | 20,480 |
| 1,000 | 83,281 | 163,840 | 94,208 | 90,112 |
| 5,000 | 416,304 | 774,144 | 442,368 | 397,312 |
| 10,000 | 832,577 | 1,523,712 | 872,448 | 778,240 |
| 50,000 | 4,162,766 | 7,618,560 | 4,296,704 | 3,829,760 |
| 100,000 | 8,325,506 | 15,278,080 | 8,581,120 | 7,639,040 |

All values are bytes; the final column is the `WITHOUT ROWID` database after
`VACUUM`. From 1,000 through 100,000 records the unvacuumed database was about
3--13% larger than the custom snapshot rather than the prototype schema's
roughly 1.8--2.0 times. `VACUUM` recovered only about 0--11% from the new
schema; the main reduction came from eliminating the duplicate primary-key
B-tree.
`VACUUM` is therefore not required for normal operation, although explicit
maintenance may be useful after unusually large removals. The production
schema should use `WITHOUT ROWID`. This use matches SQLite's
[documented storage optimisation](https://www.sqlite.org/withoutrowid.html)
for tables with non-integer primary keys.

### `WITHOUT ROWID` performance

Representative end-to-end `projects` query distributions from the paired
follow-up were:

| Records | Schema | Median | p95 |
| ---: | --- | ---: | ---: |
| 100 | rowid | 2.50 | 2.63 |
| 100 | `WITHOUT ROWID` | 2.49 | 2.59 |
| 1,000 | rowid | 3.37 | 3.52 |
| 1,000 | `WITHOUT ROWID` | 3.35 | 3.47 |
| 10,000 | rowid | 11.58 | 11.82 |
| 10,000 | `WITHOUT ROWID` | 11.15 | 11.35 |
| 50,000 | rowid | 44.82 | 46.76 |
| 50,000 | `WITHOUT ROWID` | 43.03 | 43.69 |
| 100,000 | rowid | 86.48 | 89.36 |
| 100,000 | `WITHOUT ROWID` | 83.06 | 84.54 |

The 5,000-record result followed the same trend (7.10 versus 6.89 ms median).
All four query classes were measured at every size and showed neutral results
for small histories and modest improvements as histories grew. Every backend
returned the same selected path.

Paired single-update medians stayed between 2.93 and 3.34 ms for both schemas
at every size. Neither schema had a consistent advantage larger than about
0.2 ms, and `WITHOUT ROWID` showed no size-dependent write penalty. Its query,
update, integrity, crash-recovery, concurrent-writer, and reader/writer-overlap
tests all passed. The explicit stress run completed 100 rounds per backend with
eight readers and eight writers in each round. The size reduction therefore
comes without an observed performance or correctness trade-off and justifies
adoption in task 6.

### Connection setup and production recommendations

The prototype opens every database read-write/create, applies a 30-second busy
timeout, sets `journal_mode=DELETE` and `synchronous=FULL`, and repeats `CREATE
TABLE IF NOT EXISTS` before updates. A production-like benchmark variant
instead used read-only query connections; existing read-write update
connections; `synchronous=FULL` only on writes; no repeated journal-mode or
schema setup; and a 100 ms busy timeout.

Removing the repeated setup reduced median measured open/setup time by roughly
70--80 microseconds for queries and about 20--30 microseconds for updates. It
did not produce a consistent end-to-end difference: at 10,000 records query
medians were 11.24 ms with repeated setup and 11.20 ms production-like, while
update medians were 3.07 and 3.05 ms. The 100-record results were likewise
effectively tied once outliers were retained.

This is not a reason to retain setup on every invocation. Production should
establish schema and rollback journal mode during initialization/migration,
validate the schema version cheaply on every open, and reserve connection-local
durability settings for connections that need them. Queries should open an
existing database read-only, validate without migrating, and keep the
already-required `DEFERRED` transaction around both the global tick and record
rows. Updates should remain read-write with `synchronous=FULL` and `BEGIN
IMMEDIATE`; writable initialization is responsible for any required migration.

The production-like 100 ms busy timeout completed all ordinary concurrency
tests and the 100-round stress run without losing an update. This is evidence
that the bound is ample for the synthetic workload, not a final user-experience
measurement. Task 6 should make the tracking timeout operation-specific and
treat contention as a droppable visit; explicit administrative operations may
justify a longer bound. The prototype's 30-second timeout must not carry into
the synchronous Fish tracking hook.

`synchronous=FULL` remains the recommendation because it already produced
acceptable update latency. `NORMAL` and `EXTRA` were deliberately not measured:
neither can change the current decision. No page-size, cache, memory-map,
temporary-storage, auto-vacuum, or speculative-index tuning was performed.
Those settings remain defaults unless a production workload establishes a
specific problem.

The original stripped release harness was 496,032 bytes without SQLite and
2,244,384 bytes with the default bundled SQLite build, an incremental increase
of 1,748,352 bytes (1.67 MiB). This is an isolation measurement from a benchmark
executable, not a prediction of the final zfz executable's absolute size.

The bundled build enables optional facilities zfz does not use, including FTS,
R-tree, DBSTAT, extension loading, column metadata, and STAT4. Disabling those
optional compile-time features reduced the same harness to 1,731,120 bytes with
no measurable change in 100-run 10,000-record read or update samples. Compiling
SQLite's C code with `-Oz` as well reduced it to 1,204,464 bytes, but increased
the representative read mean from about 11.5 ms to 12.3 ms; update latency
remained about 2.6 ms. Applying size optimisation to the entire Rust executable
was rejected because it increased that read mean to about 18.6 ms.

The initial production build should omit unused optional SQLite facilities but
retain normal optimisation. More aggressive C-only size optimisation remains a
distribution trade-off to revisit after the production executable exists.
SQLite itself notes that compiler size optimisation generally has more effect
than feature omission and cautions that arbitrary `SQLITE_OMIT_*` combinations
are not all supported; any non-default production configuration must run the
full zfz storage correctness suite. See SQLite's documentation on
[compile-time options](https://www.sqlite.org/compile.html) and
[library footprint](https://www.sqlite.org/footprint.html).

## Correctness and limitations

Process tests covered concurrent readers, reader/writer overlap, concurrent
writers, torn journal frames, pre/post journal flush, both SQLite transaction
boundaries, temporary snapshot flush, atomic snapshot replacement, and the gap
before journal reset. A 100-round stress run interleaved eight readers and eight
writers for every backend. Successful updates were never lost; reads always
returned a complete clock/record version; integrity verification and a normal
subsequent update succeeded after every injected crash.

The overlap experiment initially exposed an inconsistent SQLite load when the
clock and rows were read in separate autocommit statements. The retained
prototype fixes this by loading both within one read transaction; all reported
read timings were rerun after that fix.

Measurements are warm-cache results. Fresh store copies and first opens were
also sampled, but APFS copying does not prove cache eviction, so they are not
labelled cold. Privileged system-wide cache purging was intentionally not used.
Tail outliers remain in the raw results and are represented by p95/p99/max
rather than removed.
