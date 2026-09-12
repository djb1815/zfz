//! Ordered-character fuzzy matching for directory paths.
//!
//! This module deliberately exposes eligibility and match quality separately.
//! Ranking experiments can therefore decide how (or whether) to use the score
//! alongside a directory's history score.

use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

/// The score and alignment selected for one query term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermMatch {
    /// Higher values represent a more natural fuzzy alignment.
    score: i64,
    /// Character offsets in the candidate that form the selected alignment.
    positions: Vec<usize>,
}

impl TermMatch {
    /// Returns the fuzzy-match quality for this term.
    #[must_use]
    pub const fn score(&self) -> i64 {
        self.score
    }

    /// Returns the character offsets that form this term's alignment.
    #[must_use]
    pub fn positions(&self) -> &[usize] {
        &self.positions
    }
}

/// The independently observable result of matching all query terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateMatch {
    /// The optimal alignment for each term, in query order.
    terms: Vec<TermMatch>,
    /// The sum of the per-term fuzzy scores. It is not a final ranking score.
    fuzzy_score: i64,
    /// Whether the selected term alignments occur in query order in the path.
    ///
    /// This is deliberately observational: it does not affect eligibility or
    /// `fuzzy_score` until ranking experiments establish that it is useful.
    terms_in_path_order: bool,
}

impl CandidateMatch {
    /// Returns the optimal alignment for each term, in query order.
    #[must_use]
    pub fn terms(&self) -> &[TermMatch] {
        &self.terms
    }

    /// Returns the sum of the per-term fuzzy scores.
    ///
    /// This is not a final ranking score.
    #[must_use]
    pub const fn fuzzy_score(&self) -> i64 {
        self.fuzzy_score
    }

    /// Returns whether the selected term alignments occur in query order.
    ///
    /// This is observational and does not affect match eligibility or the
    /// fuzzy score.
    #[must_use]
    pub const fn terms_in_path_order(&self) -> bool {
        self.terms_in_path_order
    }
}

fn matcher() -> SkimMatcherV2 {
    // This matcher's ASCII smart-case mode ignores ASCII case for lowercase
    // terms and respects case when a term contains an ASCII uppercase letter.
    SkimMatcherV2::default().smart_case()
}

/// Returns whether `term` is an ordered-character fuzzy match for `path`.
#[must_use]
pub fn matches(path: &str, term: &str) -> bool {
    fuzzy_score(path, term).is_some()
}

/// Returns fuzzy-match quality independently of any directory-history score.
///
/// The score is produced by `fuzzy-matcher`'s Smith-Waterman-based
/// `SkimMatcherV2`, an fzf V2-style matcher. `None` means the term is not an
/// ordered-character match.
#[must_use]
pub fn fuzzy_score(path: &str, term: &str) -> Option<i64> {
    matcher().fuzzy_match(path, term)
}

/// Matches every query term against `path` with AND semantics.
///
/// Terms may occur in any order in the path. Empty terms are ignored because a
/// shell can pass an explicitly quoted empty argument, which should not filter
/// an otherwise valid candidate.
#[must_use]
pub fn match_terms<'a, I>(path: &str, terms: I) -> Option<CandidateMatch>
where
    I: IntoIterator<Item = &'a str>,
{
    let matcher = matcher();
    let mut matches = Vec::new();

    for term in terms.into_iter().filter(|term| !term.is_empty()) {
        let (score, positions) = matcher.fuzzy_indices(path, term)?;
        matches.push(TermMatch { score, positions });
    }

    let fuzzy_score = matches.iter().map(|term| term.score).sum();
    // With multiple terms, record whether each alignment begins after the
    // preceding alignment ends. This does not affect match eligibility.
    let terms_in_path_order = matches.windows(2).all(|pair| {
        let previous_end = pair[0].positions.last();
        let next_start = pair[1].positions.first();
        previous_end
            .zip(next_start)
            .is_none_or(|(end, start)| end < start)
    });

    Some(CandidateMatch {
        terms: matches,
        fuzzy_score,
        terms_in_path_order,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{TermMatch, fuzzy_score, match_terms, matches};

    #[test]
    fn contiguous_matches_score_higher_than_gapped_matches() {
        let contiguous = fuzzy_score("/home/alice/Documents", "docs").unwrap();
        let gapped = fuzzy_score("/home/alice/Development/clients", "dcs").unwrap();

        assert!(contiguous > gapped);
    }

    #[test]
    fn ordered_non_contiguous_characters_are_eligible() {
        assert!(matches("/home/alice/Documents", "dcs"));
    }

    #[test]
    fn out_of_order_characters_are_rejected() {
        assert!(!matches("/home/alice/Documents", "sdoc"));
        assert_eq!(fuzzy_score("/home/alice/Documents", "sdoc"), None);
    }

    #[test]
    fn every_term_must_match_but_terms_need_not_be_in_path_order() {
        let in_order = match_terms("/home/alice/Documents/projects/zfz", ["docs", "proj"])
            .expect("both terms match");
        let reverse_order = match_terms("/home/alice/projects/zfz/Documents", ["docs", "proj"])
            .expect("both terms match");

        assert!(in_order.terms_in_path_order());
        assert!(!reverse_order.terms_in_path_order());
        assert!(match_terms("/home/alice/Documents", ["docs", "proj"]).is_none());
    }

    #[test]
    fn component_boundaries_improve_match_quality() {
        let boundary = fuzzy_score("/work/client-project", "cp").unwrap();
        let interior = fuzzy_score("/work/receiptpaper", "cp").unwrap();

        assert!(boundary > interior);
    }

    #[test]
    fn ascii_lowercase_is_case_insensitive_but_uppercase_is_case_sensitive() {
        assert!(matches("/work/Documents", "docs"));
        assert!(matches("/work/documents", "docs"));
        assert!(matches("/work/Documents", "Docs"));
        assert!(!matches("/work/documents", "Docs"));
    }

    #[test]
    fn non_ascii_case_is_not_folded_by_the_dependency() {
        assert!(matches("/work/Éclair", "Écl"));
        assert!(!matches("/work/Éclair", "écl"));
    }

    #[test]
    fn fuzzy_score_is_not_a_history_ranking_formula() {
        let result = match_terms("/work/Documents/projects", ["docs", "proj"])
            .expect("path matches every term");

        assert_eq!(
            result.fuzzy_score(),
            result.terms().iter().map(TermMatch::score).sum()
        );
    }
}
