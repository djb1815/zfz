# Prototype fixtures

These small, hand-written fixtures provide stable inputs for matching, ranking,
and storage prototypes. They are deliberately storage-format independent.

- `directory_histories.tsv` uses `path`, `visit_count`, and an illustrative
  last-visit timestamp. It is not a persistence-file specification.
- `queries.tsv` lists query terms and the behaviour each later prototype should
  make observable.
