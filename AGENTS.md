# Repository Guidelines

## Project Structure & Module Organization

`zfz` is a design-first Rust project for a Fish-native directory jumper; the
user-facing Fish command will be `z`. `docs/DESIGN.md` is the source of truth
for confirmed design substance. Keep planned work in `docs/TASKS.md` and add
separate documents under `docs/` for benchmarks and other research. Rust
implementation code should live in `src/`, integration code in a clearly named
Fish directory (for example, `fish/`), and tests in `tests/` when they are
added. Keep persistent-state, matching, ranking, and CLI concerns separate.

## Build, Test, and Development Commands

Use [mise](https://mise.jdx.dev/) to obtain the pinned stable Rust toolchain
and components. Once the Cargo crate exists, run:

```sh
mise run check
```

This runs `cargo fmt --check`, Clippy across all targets and features with
warnings denied, and the test suite. During development, `cargo test` runs the
tests and `cargo fmt` applies Rust formatting. Do not add a separate toolchain
configuration unless the existing `mise.toml` cannot express the need.

## Coding Style & Naming Conventions

Follow idiomatic Rust and let `rustfmt` decide layout; use four-space
indentation where formatting is not automated. Use `snake_case` for functions,
modules, and variables; `PascalCase` for types; and `SCREAMING_SNAKE_CASE` for
constants. Keep Fish functions small, safely quote paths, and use names such as
`__zfz_on_pwd` for private helpers. Preserve paths reported by Fish rather than
canonicalising them, as required by the design.

## Testing Guidelines

Add focused unit tests next to logic and integration tests in `tests/` for CLI
and Fish-facing behaviour. Name tests after observable outcomes, such as
`multiple_terms_require_all_matches`. Prefer parameterised tests with `rstest`
when multiple examples exercise the same behaviour. Cover ordered-character
matching, ranking modes, unusual path contents, persistence failures, and
concurrent updates where relevant. Run `mise run check` before submitting
changes.

## Commit & Pull Request Guidelines

History currently uses short, imperative summaries (for example, `Initial
commit of design doc & toolchain setup`); continue with concise imperative
subjects. For substantive changes, add a commit body explaining the motivation,
key implementation choices, and validation performed, so the history remains
useful without reopening the diff. Keep commits focused.

Changes merge to `main` through pull requests, but raise a PR only when
explicitly instructed to do so. Before raising one, review its diff against the
target branch and run the relevant checks. Use a short, descriptive title and a
complete description with `Summary` and `Testing` sections. Summarise the
included changes in `Summary`, and list the checks performed as bullets in
`Testing`. Reference any GitHub issue using GitHub's closing syntax when the PR
resolves it. The repository squash-merges PRs to `main`, using the PR title and
description as the resulting commit message, so ensure both accurately and
durably describe the complete change.

## Documentation Changes

Update `docs/DESIGN.md` only when implementation establishes or changes a
real design decision. Record planned work in `docs/TASKS.md`; put benchmarks,
prototypes, and supporting research in purpose-specific `docs/` files. Do not
silently diverge from the Fish-native, short-lived-process architecture.
