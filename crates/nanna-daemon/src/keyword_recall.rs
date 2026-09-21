//! Keyword recall: what `memory.search` answers with when semantic recall
//! cannot.
//!
//! Semantic recall needs an embedding of the query and of every memory. With
//! no embedder answering — none configured, or every provider benched — the
//! query cannot be embedded at all, and a memory written in that state is
//! stored whole but queued for embedding, so even a recovered embedder cannot
//! find it until the backfill drains. `memory.search` used to turn both into
//! an empty result, and the model was told "No memories found matching" about
//! a memory stored seconds earlier. On a host with no embedder that made
//! recall useless and made it lie.
//!
//! This ranks memories by how many of the query's words they contain. It is
//! a fallback, not a retriever: no stemming, no synonyms, no meaning — and the
//! results say so (`"match": "keyword"`), so the reader knows a miss here is
//! a miss of words, not of meaning.
//!
//! Cost: one pass over the stored contents, O(total bytes × terms), with the
//! terms bounded by [`QUERY_TERMS_MAX`]. It runs only on the degraded path.

/// Most query terms considered. A recall query is a phrase, not a document;
/// past this, extra words only dilute the score.
pub const QUERY_TERMS_MAX: usize = 16;

/// Shortest word that counts as a term. Two-letter words are almost all
/// function words ("is", "my", "to") and match nearly every memory.
pub const TERM_CHARS_MIN: usize = 3;

/// Frequent English function words of [`TERM_CHARS_MIN`] or more characters.
/// They carry no topic, so a memory matching only these is not a match.
const STOPWORDS: &[&str] = &[
    "about", "and", "are", "but", "can", "did", "does", "for", "from", "had", "has", "have", "her",
    "his", "how", "into", "its", "not", "our", "she", "that", "the", "their", "them", "then",
    "there", "they", "this", "was", "were", "what", "when", "where", "which", "who", "why", "will",
    "with", "you", "your",
];

/// One ranked hit: the index of the memory in the caller's list, and the
/// fraction of the query's terms it contains, in `(0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeywordHit {
    pub index: usize,
    pub score: f32,
}

/// The query's terms: lowercase alphanumeric words of at least
/// [`TERM_CHARS_MIN`] characters, stopwords dropped, first occurrence kept,
/// at most [`QUERY_TERMS_MAX`].
#[must_use]
pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        if terms.len() == QUERY_TERMS_MAX {
            break;
        }
        let word = word.to_lowercase();
        if word.chars().count() < TERM_CHARS_MIN
            || STOPWORDS.contains(&word.as_str())
            || terms.contains(&word)
        {
            continue;
        }
        terms.push(word);
    }
    debug_assert!(terms.len() <= QUERY_TERMS_MAX, "terms are bounded");
    terms
}

/// Rank `contents` by the fraction of `terms` each contains (case-insensitive
/// substring), best first, ties by original order; memories matching no term
/// are left out. At most `limit` hits.
#[must_use]
pub fn rank<'a>(
    terms: &[String],
    contents: impl Iterator<Item = &'a str>,
    limit: usize,
) -> Vec<KeywordHit> {
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }
    let term_count = u16::try_from(terms.len()).unwrap_or(u16::MAX);
    let mut hits: Vec<KeywordHit> = contents
        .enumerate()
        .filter_map(|(index, content)| {
            let haystack = content.to_lowercase();
            let matched = terms
                .iter()
                .filter(|term| haystack.contains(term.as_str()))
                .count();
            let matched = u16::try_from(matched).unwrap_or(u16::MAX);
            (matched > 0).then(|| KeywordHit {
                index,
                score: f32::from(matched) / f32::from(term_count),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.index.cmp(&b.index)));
    hits.truncate(limit);
    debug_assert!(
        hits.iter().all(|hit| hit.score > 0.0 && hit.score <= 1.0),
        "scores are fractions of the terms matched"
    );
    hits
}

#[cfg(test)]
mod tests {
    use super::{QUERY_TERMS_MAX, query_terms, rank};

    #[test]
    fn terms_drop_short_words_stopwords_and_repeats() {
        assert_eq!(
            query_terms("What's my cat's name? The CAT!"),
            ["cat", "name"]
        );
        assert!(
            query_terms("is it on?").is_empty(),
            "nothing topical to search"
        );
        let long = (0..40)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(query_terms(&long).len(), QUERY_TERMS_MAX);
    }

    #[test]
    fn memories_rank_by_the_share_of_terms_they_contain() {
        let memories = [
            "The user prefers dark roast coffee.",
            "The user's cat is named Moonpie.",
            "Moonpie the cat was adopted in 2024; her name came from a snack.",
        ];
        let hits = rank(&query_terms("cat name"), memories.iter().copied(), 10);
        let order: Vec<usize> = hits.iter().map(|hit| hit.index).collect();
        assert_eq!(
            order,
            [1, 2],
            "both cat memories match both terms; coffee matches none"
        );
        assert!((hits[0].score - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            rank(&query_terms("cat"), memories.iter().copied(), 1).len(),
            1,
            "limit"
        );
        assert!(
            rank(&[], memories.iter().copied(), 10).is_empty(),
            "no terms, no hits"
        );
    }
}
