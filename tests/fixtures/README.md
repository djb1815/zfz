# Prototype fixtures

These small, hand-written fixtures provide stable inputs for matching, ranking,
and storage prototypes. They are deliberately storage-format independent.

- `directory_histories.tsv` uses `path`, `visit_count`, and an illustrative
  last-visit timestamp. It is not a persistence-file specification.
- `queries.tsv` lists query terms and the behaviour each later prototype should
  make observable.
- `ranking_histories.tsv` groups realistic event-clock records into 20 scored
  ambiguous scenarios plus one neutral term-order comparison. Each row names
  the history mode and expected-result role. Stored frecency is evaluated at
  `current_tick`; frequency and recency use their native record fields. The
  file remains independent of any persistence encoding.
