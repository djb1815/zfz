# Storage benchmark harness

This crate contains the two disposable persistence prototypes used for task 5.
It is deliberately separate from zfz's production crate; the selected backend
will be implemented properly in task 6.

Build and test it with:

```console
cargo test --manifest-path benchmarks/storage/Cargo.toml
cargo build --release --manifest-path benchmarks/storage/Cargo.toml
```

The release executable accepts:

```text
storage-benchmark init BACKEND STORE RECORDS
storage-benchmark query BACKEND STORE TERM...
storage-benchmark update BACKEND STORE PATH
storage-benchmark burst BACKEND STORE UPDATES WORKING_SET
storage-benchmark compact journal STORE
storage-benchmark vacuum SQLITE_BACKEND STORE
storage-benchmark verify BACKEND STORE
storage-benchmark bytes BACKEND STORE
```

`BACKEND` is `journal`, `sqlite-delete`, `sqlite-delete-without-rowid`,
`sqlite-delete-production`, or `sqlite-wal`. The `sqlite-delete` backend retains
the original task 5 rowid schema and per-invocation setup.
`sqlite-delete-without-rowid` changes only the records-table schema.
`sqlite-delete-production` additionally models the intended connection split:
read-only query connections, no repeated journal-mode/schema setup, a 100 ms
busy timeout, and read-write update connections with `synchronous=FULL`.
`ZFZ_TIMINGS=1` emits tab-separated phase timings to stderr.
The columns after `timings_ns` are open, load/read, decode, match/rank, encode,
commit/write, atomic replacement, and total elapsed time. SQLite row decoding
is included in its load phase because `rusqlite` exposes both as one iterator.
Setting `ZFZ_FAULT` to one of the fault points exercised by the integration
tests aborts the process at that point.

Run the complete reproducible measurement matrix with `run.sh OUTPUT_DIRECTORY`.
It requires a release build, Hyperfine, and jq. Write output beneath `results/`,
which is ignored by Git. Archive a completed run for external attachment when
the raw evidence needs to accompany a review; commit only the harness and the
resulting conclusions in `docs/storage.md`.

Run `run-sqlite-followup.sh OUTPUT_DIRECTORY` for the bounded post-benchmark
comparison of the original rowid schema, `WITHOUT ROWID`, and production-like
connection setup. It deliberately does not vary durability or unrelated SQLite
settings.
