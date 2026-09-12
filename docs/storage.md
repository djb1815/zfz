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

An audit on 2026-09-12 compared that schema with the same table declared
`WITHOUT ROWID`:

| Records | Snapshot | Original SQLite | After `VACUUM` | `WITHOUT ROWID` | `WITHOUT ROWID` after `VACUUM` |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 832,577 | 1,523,712 | 1,454,080 | 872,448 | 778,240 |
| 100,000 | 8,325,506 | 15,278,080 | 14,520,320 | 8,597,504 | 7,651,328 |

All values are bytes. `VACUUM` recovered only about 5% from the original schema;
the main reduction came from eliminating the duplicate primary-key B-tree.
`VACUUM` is therefore not required for normal operation, although explicit
maintenance may be useful after unusually large removals. The production
schema should use `WITHOUT ROWID`; its update, read, concurrency, and recovery
behaviour must be revalidated as part of task 6. This use matches SQLite's
[documented storage optimisation](https://www.sqlite.org/withoutrowid.html)
for tables with non-integer primary keys.

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
