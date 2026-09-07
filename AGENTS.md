# Repository Guidelines

## Architecture and current stage

- `zfz` is a Fish-native directory jumper: Fish provides `z`; Rust provides
  short-lived executable and core logic.
- Keep frecency/history, matching, ranking, storage, CLI, and shell layers
  separate. No layer should take on another's work.
- Do not add a daemon, wrap `cd`, track from the prompt, or canonicalise Fish
  paths. Preserve paths verbatim, including symlink spelling and unusual names.
- Keep production APIs focused on established behaviour; experimental or
  rejected alternatives belong in test support.
- `src/main.rs` is intentionally a placeholder until persistence and CLI work.
- Put shell code in `shell/`; Fish is the initial target.

## Layout and documentation

- `src/` contains production Rust. Keep Rust dependencies directed inward from
  the CLI; shell integration calls the CLI and must not duplicate core logic.
- `tests/` contains integration, scenario, and property tests;
  `tests/support/` is test-only. `tests/fixtures/` is maintained behavioural
  evidence, not disposable data: update affected fixtures and, when their
  format or documented semantics change, their README.
- `shell/` contains shell integration and its documentation.
- `docs/DESIGN.md` records confirmed decisions and constraints;
  `docs/TASKS.md` records planned work and status;
  `docs/FOLLOWUPS.md` records deferred decisions & work.
- Use lowercase filenames for new focused benchmark or research documents under
  `docs/`; existing documents cover frecency, matching, and ranking.
- Do not update `DESIGN.md` speculatively. Record a durable rationale when
  implementation establishes a decision.

## Development and tests

- Use the Rust toolchain and tasks pinned in `mise.toml`; add no separate
  toolchain configuration unless mise cannot express a real need.
- Use `cargo test` for focused work and `cargo fmt` to format Rust. Before
  handoff, run `mise run check` when practical; it checks format, Clippy
  (`-D warnings`), and tests.
- Run `mise run build-release` for release, benchmark, startup-cost, or
  distribution changes.
- Follow `rustfmt`: `snake_case` functions/modules, `PascalCase` types, and
  `SCREAMING_SNAKE_CASE` constants. Keep Fish helpers small, safely quote
  paths, and name private helpers `__zfz_*`.
- Put focused module tests beside production logic; use `tests/` for
  cross-module, CLI, shell-facing, scenario, and property coverage.
- Name tests for observable behaviour. Use `rstest` for genuine parameter
  sets and existing fixture loaders before adding harnesses.
- Fuzzy matching, frecency, and canonical-ranking changes need unit evidence
  plus relevant scenario, property, or fixture updates. Keep ordering
  deterministic; errors must not partially reorder input.
- Cover applicable risks: ordered-character AND matching, ranking ties and
  modes, case/non-ASCII semantics, unusual paths, persistence recovery and
  concurrency, and symlink preservation.

## Git, commits, and pull requests

### Commits

- Keep changes and commits focused; do not overwrite unrelated uncommitted
  work.
- Use concise imperative subjects. Substantive commits need motivation, key
  decisions, and validation in the body.

### Pull requests

- Merge to `main` by PR. Open or create one only when asked.
- Before opening, review the diff against the target branch and run relevant
  checks.
- Use a short descriptive title and `Summary` and `Testing` sections. PRs
  squash-merge to durable `main` history, so both must stand alone.
- `Summary`: purpose, observable behaviour, important choices, and material
  limitations or follow-ups; keep self-evident fixes proportionate.
- `Testing`: only checks actually performed, including relevant manual/live
  validation; name commands and outcomes where useful.
- Use GitHub closing syntax for an issue the PR resolves.
