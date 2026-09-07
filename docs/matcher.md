# Fuzzy Matcher Prototype

Task 3 uses [`fuzzy-matcher` 0.3.7](https://crates.io/crates/fuzzy-matcher),
specifically `SkimMatcherV2`. It is the smallest suitable Rust dependency for
the prototype: it supplies a Smith-Waterman-based, fzf V2-style optimal
ordered-character matcher and returns a score independently of eligibility.

This is intentionally a matcher prototype, not a final ranking decision. The
public API exposes:

- `matches(path, term)` for eligibility;
- `fuzzy_score(path, term)` for one term's quality;
- `match_terms(path, terms)` for AND semantics, a sum of per-term scores, and
  the separately observable `terms_in_path_order` signal.

The aggregate fuzzy score is not numerically combined with frecency. The Task 4
ranking experiment selected history-primary ordering with fuzzy score as an
exact-tie signal; see [`ranking.md`](ranking.md).

## Case behaviour

The matcher uses the dependency's **ASCII** smart-case policy: lowercase ASCII
terms are ASCII-case-insensitive and a term containing an ASCII uppercase
character is case-sensitive. Its case folding is not Unicode-aware; for
example, `écl` does not match `Éclair`. The fixture corpus records this
limitation so it can be reassessed before the matcher is considered final.

## Investigation and licensing

fzf's V2 implementation is a modified Smith-Waterman algorithm which finds
the optimal alignment and scores word boundaries, compactness, and consecutive
matches ([fzf source](https://github.com/junegunn/fzf/blob/master/src/algo/algo.go)).
fzf is MIT licensed ([LICENSE](https://github.com/junegunn/fzf/blob/master/LICENSE)).

`fuzzy-matcher` is also MIT licensed, so adding it is compatible with zfz's
MIT license. Its documentation describes `SkimMatcherV2` as Smith-Waterman
based and fzf V2-style. Exact fzf score parity is not a goal: fzf is a model
for practical matching behaviour, rather than a specification. The prototype
therefore provides established behaviour without copying or porting fzf code.
