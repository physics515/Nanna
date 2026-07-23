//! Lexical tool search: BM25 scoring + Snowball (porter2) stemming.
//!
//! Replaces the brittle per-term substring matching in `discover_tools` with
//! real relevance ranking, so a small local model's queries ("file write",
//! "replace", "run command") reliably surface the right tools. This is
//! meilisearch-grade relevance *in process*: the corpus is ~40 tools, so every
//! query simply scores all documents (microseconds) — no index, no external
//! server, IDF recomputed from the live corpus on each call.
//!
//! Pipeline per query:
//! 1. **Tokenize** — lowercase, split on non-alphanumeric. Underscored words
//!    split too (`write_file` → `write`, `file`), but the joined form is ALSO
//!    kept as a verbatim term so an exact tool-name query outranks everything.
//!    Joined forms are never stemmed: porter2 mangles them (`write_file` →
//!    `write_fil`), and verbatim-on-both-sides is exact anyway.
//! 2. **Stem** — Snowball porter2 English on every plain token, document and
//!    query alike. Measured outputs (encoded in tests below): `write`/`writes`/
//!    `writing` → `write`; `replace`/`replacing`/`replacement` → `replac`;
//!    `command`/`commands` → `command`; `execute`/`execution` → `execut`.
//! 3. **Fuzzy per-term fallback** — a query term matching zero document terms
//!    is matched against the corpus's *raw* vocabulary by normalized
//!    Damerau-style similarity (adjacent transpositions count as one edit) at
//!    the same ≥ 0.7 threshold family `ToolRegistry::resolve_tool` uses, and
//!    the closest vocab term's stem stands in for it — catching typos like
//!    `wirte file`.
//! 4. **Score** — BM25 (k1 = 1.2, b = 0.75) over per-tool documents built from
//!    name + description, with name tokens weighted 3x (see [`NAME_WEIGHT`]).
//!    Ties break by name, so ranking is fully deterministic.

use rust_stemmers::{Algorithm, Stemmer};
use std::collections::{BTreeMap, HashMap};

/// BM25 term-frequency saturation parameter (standard default).
const BM25_K1: f64 = 1.2;

/// BM25 document-length normalization parameter (standard default).
const BM25_B: f64 = 0.75;

/// How many times more a name-token occurrence counts than a description
/// token. Implemented as *field boost via term-frequency repetition*: name
/// tokens contribute `NAME_WEIGHT` to the term frequency (and to document
/// length, so the boost is honest under BM25's length normalization). 3x is
/// enough that a query hitting a tool's name outranks any description-only
/// match, without letting a name hit on one term drown a two-term description
/// match — names are 1-3 tokens, descriptions dozens.
const NAME_WEIGHT: usize = 3;

/// Minimum normalized similarity for the per-term fuzzy fallback — the same
/// 0.7 threshold family `ToolRegistry::resolve_tool` applies to whole tool
/// names. Distance here is OSA (Levenshtein + adjacent transposition as one
/// edit): plain Levenshtein scores the classic transposition typo
/// `wirte`/`write` at 2 edits = 0.6 similarity — under threshold — while OSA
/// scores it 1 edit = 0.8. Tool queries are short words, where transpositions
/// dominate real typos.
const FUZZY_MIN_SIMILARITY: f64 = 0.7;

/// One ranked search result from [`search_docs`] /
/// [`crate::ToolRegistry::search_tools`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSearchHit {
    /// Canonical tool name (never an alias).
    pub name: String,
    /// Tool description, verbatim from the definition.
    pub description: String,
    /// BM25 relevance score (> 0.0; non-matching tools are omitted).
    pub score: f64,
}

/// A searchable document: one canonical tool.
#[derive(Debug, Clone)]
pub struct SearchDoc {
    /// Canonical tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
}

/// Lowercase and split `text` into words of `[a-z0-9_]`.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// An atomic query/document unit: the raw (pre-stem) form and the term that
/// is actually indexed/matched.
#[derive(Debug, Clone)]
struct Atom {
    /// Raw lowercased token — what fuzzy fallback compares against.
    raw: String,
    /// Indexed term: the stem for plain words; verbatim for joined `a_b` forms.
    term: String,
}

/// Expand one word into atoms. A word containing `_` yields the verbatim
/// joined form plus each stemmed part; a plain word yields its stem.
fn atoms_for_word(word: &str, stemmer: &Stemmer, out: &mut Vec<Atom>) {
    if word.contains('_') {
        out.push(Atom {
            raw: word.to_string(),
            term: word.to_string(),
        });
        for part in word.split('_').filter(|p| !p.is_empty()) {
            out.push(Atom {
                raw: part.to_string(),
                term: stemmer.stem(part).into_owned(),
            });
        }
    } else {
        out.push(Atom {
            raw: word.to_string(),
            term: stemmer.stem(word).into_owned(),
        });
    }
}

/// Tokenize + stem a whole text into atoms.
fn atomize(text: &str, stemmer: &Stemmer) -> Vec<Atom> {
    let mut out = Vec::new();
    for word in words(text) {
        atoms_for_word(&word, stemmer, &mut out);
    }
    out
}

/// Optimal-string-alignment distance: Levenshtein plus adjacent transposition
/// counted as a single edit. Same DP shape as `registry.rs`'s `levenshtein`,
/// extended with one extra row of history for the transposition case.
fn osa_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    // Three rolling rows: i-2, i-1, i.
    let mut prev2: Vec<usize> = vec![0; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2] + 1);
            }
            cur[j] = best;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

/// Normalized OSA similarity: 1.0 identical, 0.0 unrelated. The same
/// normalization `registry.rs`'s `normalized_similarity` applies to plain
/// Levenshtein.
fn osa_similarity(a: &str, b: &str) -> f64 {
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (osa_distance(a, b) as f64 / max_len as f64)
}

/// One indexed document: weighted term frequencies + weighted length.
struct IndexedDoc<'a> {
    doc: &'a SearchDoc,
    /// term → weighted term frequency (name tokens count [`NAME_WEIGHT`]).
    tf: HashMap<String, f64>,
    /// Weighted document length (sum of all tf values).
    len: f64,
}

/// Search `docs` for `query`, returning up to `limit` hits ranked by BM25
/// score (descending), ties broken by name (ascending) for determinism.
/// Only positively-scoring documents are returned; an empty or non-matching
/// query yields an empty vec.
#[must_use]
pub fn search_docs(docs: &[SearchDoc], query: &str, limit: usize) -> Vec<ToolSearchHit> {
    if docs.is_empty() || limit == 0 {
        return Vec::new();
    }

    // Bound the query to the corpus's own text size. Every query atom either
    // matches an indexed term or fuzzy-maps onto the corpus vocabulary, so a
    // query longer than the ENTIRE corpus text cannot carry more matching
    // signal than the corpus itself — anything beyond that is pure tokenize/
    // stem cost with zero score effect. This is the principled ceiling; no
    // magic constant. (Cut lands on a char boundary; excess is discarded.)
    let corpus_chars: usize = docs
        .iter()
        .map(|d| d.name.len() + d.description.len() + 2)
        .sum();
    let query = if query.len() > corpus_chars {
        let mut end = corpus_chars;
        while end > 0 && !query.is_char_boundary(end) {
            end -= 1;
        }
        &query[..end]
    } else {
        query
    };

    let stemmer = Stemmer::create(Algorithm::English);

    // ---- Index: weighted tf per doc + raw-vocabulary map (raw → term). ----
    // BTreeMap so fuzzy-fallback iteration order (and thus tie-breaking on
    // equal similarity) is deterministic.
    let mut vocab: BTreeMap<String, String> = BTreeMap::new();
    let mut indexed: Vec<IndexedDoc<'_>> = Vec::with_capacity(docs.len());
    for doc in docs {
        let mut tf: HashMap<String, f64> = HashMap::new();
        let mut len = 0.0;
        for atom in atomize(&doc.name, &stemmer) {
            *tf.entry(atom.term.clone()).or_insert(0.0) += NAME_WEIGHT as f64;
            len += NAME_WEIGHT as f64;
            vocab.entry(atom.raw).or_insert(atom.term);
        }
        for atom in atomize(&doc.description, &stemmer) {
            *tf.entry(atom.term.clone()).or_insert(0.0) += 1.0;
            len += 1.0;
            vocab.entry(atom.raw).or_insert(atom.term);
        }
        indexed.push(IndexedDoc { doc, tf, len });
    }

    let n = indexed.len() as f64;
    let total_len: f64 = indexed.iter().map(|d| d.len).sum();
    let avgdl = if total_len > 0.0 { total_len / n } else { 1.0 };

    let df = |term: &str| -> usize { indexed.iter().filter(|d| d.tf.contains_key(term)).count() };

    // Vocabulary with precomputed char lengths for the fuzzy prefilter.
    let fuzzy_vocab: Vec<(&str, &str, usize)> = vocab
        .iter()
        .map(|(raw, term)| (raw.as_str(), term.as_str(), raw.chars().count()))
        .collect();

    // ---- Resolve query atoms to effective terms (with fuzzy fallback). ----
    let mut terms: Vec<String> = Vec::new();
    for atom in atomize(query, &stemmer) {
        if df(&atom.term) > 0 {
            terms.push(atom.term);
            continue;
        }
        // Zero exact matches: fuzzy-match the RAW token against the raw
        // vocabulary (stemming a typo yields a garbage stem, so raw-vs-raw is
        // the meaningful comparison) and adopt the best match's indexed term.
        //
        // Length-ratio prefilter: OSA distance is at least the length
        // difference, so similarity can never reach FUZZY_MIN_SIMILARITY when
        // |len(a) - len(b)| / max > 1 - FUZZY_MIN_SIMILARITY. Checking that
        // is O(1) per vocab entry (lengths precomputed), which keeps a huge
        // garbage token from paying O(|token| * |word|) against every entry.
        let atom_len = atom.raw.chars().count();
        let mut best: Option<(f64, &str)> = None;
        for (raw, term, raw_len) in &fuzzy_vocab {
            let max_len = atom_len.max(*raw_len);
            if max_len == 0
                || atom_len.abs_diff(*raw_len) as f64 / max_len as f64
                    > 1.0 - FUZZY_MIN_SIMILARITY
            {
                continue;
            }
            let sim = osa_similarity(&atom.raw, raw);
            if sim >= FUZZY_MIN_SIMILARITY && best.is_none_or(|(bs, _)| sim > bs) {
                best = Some((sim, term));
            }
        }
        if let Some((_, term)) = best {
            terms.push(term.to_string());
        }
        // No plausible vocab term: drop the atom (it contributes nothing).
    }

    if terms.is_empty() {
        return Vec::new();
    }

    // ---- BM25 ----
    // IDF is the Lucene/BM25+ variant, ln(1 + (N - df + 0.5)/(df + 0.5)):
    // strictly positive even for a term present in every document. With a
    // ~40-doc corpus, ubiquitous words ("file", "tool") would go *negative*
    // under classic Robertson-Sparck-Jones IDF and actively repel matches.
    let mut hits: Vec<ToolSearchHit> = Vec::new();
    for d in &indexed {
        let mut score = 0.0;
        for term in &terms {
            let Some(&tf) = d.tf.get(term) else { continue };
            let df = df(term) as f64;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            let norm = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * d.len / avgdl);
            score += idf * (tf * (BM25_K1 + 1.0)) / norm;
        }
        if score > 0.0 {
            hits.push(ToolSearchHit {
                name: d.doc.name.clone(),
                description: d.doc.description.clone(),
                score,
            });
        }
    }

    // Rank: score descending, name ascending on ties — deterministic.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Tool, ToolDefinition, ToolError, ToolRegistry, ToolResult};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    /// Descriptions copied verbatim from the real default-skills tool.ts files
    /// so ranking assertions exercise the corpus the agent actually sees.
    const WRITE_FILE_DESC: &str = "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. SAFETY: the write is blocked if new content is less than 30% of the existing file size (likely truncation). Use force=true to override.";
    const EDIT_FILE_DESC: &str = "Replace one exact text snippet in a file with new text — an in-place edit for small changes. Use this instead of rewriting the whole file with write_file. ALL THREE main parameters are REQUIRED: file_path, old_string, new_string. old_string must be text that exists in the file EXACTLY as written — include 2-3 surrounding lines to make it unique. Only the matched snippet changes; the rest of the file is untouched. Use write_file only for new files or full rewrites.";
    const READ_FILE_DESC: &str = "Read a file from the filesystem. Returns the file contents with line numbers. Supports optional offset and limit for reading portions of large files.";
    const EXEC_DESC: &str = "Execute a shell command in a POSIX bash shell (Git Bash on Windows, sh on Unix) and return its output. ALWAYS bash syntax: pipes, &&, ||, [ -f x ] / [ -d x ], ls, cat/grep/tail, mkdir -p, 2>/dev/null, forward-slash paths. NEVER cmd.exe syntax — 'if exist', '2>nul', 'cd /d', 'errorlevel' all FAIL here. To search code, use the code_search tool — rg/ripgrep is not guaranteed on PATH. Use for build commands, scripts, git operations, etc.";
    const DISCOVER_TOOLS_DESC: &str = "Activate tools for file access, shell commands, web browsing, code analysis, and more. Call with no arguments to see all available tools, or with a query (e.g. 'file', 'exec', 'web', 'code') to filter. Activated tools persist for the rest of this conversation. You MUST call this before using any tool beyond remember/recall/reflect.";

    fn corpus() -> Vec<SearchDoc> {
        [
            ("write_file", WRITE_FILE_DESC),
            ("edit_file", EDIT_FILE_DESC),
            ("read_file", READ_FILE_DESC),
            ("exec", EXEC_DESC),
            ("discover_tools", DISCOVER_TOOLS_DESC),
        ]
        .into_iter()
        .map(|(name, desc)| SearchDoc {
            name: name.to_string(),
            description: desc.to_string(),
        })
        .collect()
    }

    fn names(hits: &[ToolSearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.name.as_str()).collect()
    }

    // ---- stemming: REAL porter2 outputs (verified by running rust-stemmers
    // 1.x Algorithm::English), not guesses. Notably `rewriting` → `rewrit`
    // (NOT `write`) and `write_file` → `write_fil` — the latter is why joined
    // forms are indexed verbatim, never stemmed. ----

    #[test]
    fn porter2_stems_match_measured_outputs() {
        let s = Stemmer::create(Algorithm::English);
        for (word, stem) in [
            ("write", "write"),
            ("writes", "write"),
            ("writing", "write"),
            ("rewriting", "rewrit"),
            ("replace", "replac"),
            ("replacing", "replac"),
            ("replaced", "replac"),
            ("replacement", "replac"),
            ("run", "run"),
            ("running", "run"),
            ("command", "command"),
            ("commands", "command"),
            ("execute", "execut"),
            ("execution", "execut"),
            ("file", "file"),
            ("files", "file"),
            ("wirte", "wirt"),
            ("write_file", "write_fil"),
        ] {
            assert_eq!(s.stem(word), stem, "porter2({word})");
        }
    }

    // ---- tokenizer ----

    #[test]
    fn underscored_words_index_joined_and_split_forms() {
        let stemmer = Stemmer::create(Algorithm::English);
        let atoms = atomize("write_file", &stemmer);
        let terms: Vec<&str> = atoms.iter().map(|a| a.term.as_str()).collect();
        assert_eq!(terms, vec!["write_file", "write", "file"]);
    }

    #[test]
    fn tokenizer_lowercases_and_splits_on_non_alphanumeric() {
        assert_eq!(
            words("Read a file, FAST-path 2x!"),
            vec!["read", "a", "file", "fast", "path", "2x"]
        );
    }

    // ---- fuzzy distance ----

    #[test]
    fn osa_counts_transposition_as_one_edit() {
        // Plain Levenshtein gives 2 here (similarity 0.6 — under threshold);
        // OSA gives 1 (0.8) — the entire reason the typo fallback works.
        assert_eq!(osa_distance("wirte", "write"), 1);
        assert!(osa_similarity("wirte", "write") >= FUZZY_MIN_SIMILARITY);
        assert!(osa_similarity("zzqqx", "write") < FUZZY_MIN_SIMILARITY);
    }

    // ---- ranking ----

    #[test]
    fn file_write_ranks_write_file_first_and_finds_siblings() {
        let hits = search_docs(&corpus(), "file write", 10);
        let ns = names(&hits);
        assert_eq!(ns.first(), Some(&"write_file"), "got: {ns:?}");
        assert!(ns.contains(&"edit_file"), "got: {ns:?}");
        assert!(ns.contains(&"read_file"), "got: {ns:?}");
    }

    #[test]
    fn replace_finds_edit_file_via_stemming() {
        // Query "replace" → stem "replac"; the edit_file description contains
        // "Replace" (→ "replac"), no literal "replace " needed.
        let hits = search_docs(&corpus(), "replace", 10);
        let ns = names(&hits);
        assert_eq!(ns.first(), Some(&"edit_file"), "got: {ns:?}");
    }

    #[test]
    fn run_command_finds_exec() {
        let hits = search_docs(&corpus(), "run command", 10);
        let ns = names(&hits);
        assert!(ns.contains(&"exec"), "got: {ns:?}");
        assert_eq!(ns.first(), Some(&"exec"), "got: {ns:?}");
    }

    #[test]
    fn typo_wirte_file_still_finds_write_file() {
        // "wirte" stems to "wirt" (df 0) → fuzzy fallback maps raw "wirte" to
        // vocab "write" (OSA similarity 0.8) → effective term "write".
        let hits = search_docs(&corpus(), "wirte file", 10);
        let ns = names(&hits);
        assert_eq!(ns.first(), Some(&"write_file"), "got: {ns:?}");
    }

    #[test]
    fn exact_tool_name_query_wins_via_joined_form() {
        let hits = search_docs(&corpus(), "write_file", 10);
        assert_eq!(names(&hits).first(), Some(&"write_file"));
    }

    #[test]
    fn garbage_query_returns_empty() {
        assert!(search_docs(&corpus(), "zzqqx", 10).is_empty());
        assert!(search_docs(&corpus(), "", 10).is_empty());
        assert!(search_docs(&corpus(), "  ~~!!  ", 10).is_empty());
    }

    // ---- bounds (Tiger Style: every cost has a derived ceiling) ----

    #[test]
    fn giant_query_is_bounded_and_signal_at_front_survives() {
        // A query longer than the whole corpus text is truncated to the
        // corpus size before tokenize/stem — the tail cannot add signal.
        // 1M chars of garbage after real signal must neither slow the call
        // to a crawl nor break ranking of the surviving prefix.
        let query = format!("write file {}", "z".repeat(1_000_000));
        let started = std::time::Instant::now();
        let hits = search_docs(&corpus(), &query, 10);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "giant query took {:?} — bound regressed",
            started.elapsed()
        );
        assert_eq!(names(&hits).first(), Some(&"write_file"));
    }

    #[test]
    fn oversized_token_skips_fuzzy_via_length_prefilter() {
        // A single huge unmatched token: the length-ratio prefilter must
        // reject every vocab entry in O(1) instead of running OSA against
        // each — and the result is empty, not a bogus fuzzy match.
        let query = "q".repeat(10_000);
        let started = std::time::Instant::now();
        let hits = search_docs(&corpus(), &query, 10);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "oversized token took {:?} — prefilter regressed",
            started.elapsed()
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn limit_truncates_ranked_results() {
        let hits = search_docs(&corpus(), "file", 2);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn same_query_twice_is_deterministic() {
        let a = search_docs(&corpus(), "file write shell command", 10);
        let b = search_docs(&corpus(), "file write shell command", 10);
        assert_eq!(names(&a), names(&b));
        let scores_a: Vec<f64> = a.iter().map(|h| h.score).collect();
        let scores_b: Vec<f64> = b.iter().map(|h| h.score).collect();
        assert_eq!(scores_a, scores_b);
    }

    #[test]
    fn equal_scores_tie_break_by_name() {
        let docs = vec![
            SearchDoc {
                name: "beta".to_string(),
                description: "frobnicate the widget".to_string(),
            },
            SearchDoc {
                name: "alpha".to_string(),
                description: "frobnicate the widget".to_string(),
            },
        ];
        let hits = search_docs(&docs, "frobnicate", 10);
        assert_eq!(names(&hits), vec!["alpha", "beta"]);
    }

    // ---- registry integration ----

    struct FakeTool {
        name: &'static str,
        description: &'static str,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                description: self.description.to_string(),
                parameters: vec![],
                output_schema: None,
            }
        }

        async fn execute(&self, _: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("ok"))
        }
    }

    async fn seeded_registry() -> ToolRegistry {
        let reg = ToolRegistry::new();
        reg.register(FakeTool {
            name: "write_file",
            description: WRITE_FILE_DESC,
        })
        .await;
        reg.register(FakeTool {
            name: "edit_file",
            description: EDIT_FILE_DESC,
        })
        .await;
        reg.register(FakeTool {
            name: "read_file",
            description: READ_FILE_DESC,
        })
        .await;
        reg.register(FakeTool {
            name: "exec",
            description: EXEC_DESC,
        })
        .await;
        reg
    }

    #[tokio::test]
    async fn registry_search_ranks_and_dedupes() {
        let reg = seeded_registry().await;
        let hits = reg.search_tools("file write", 10).await;
        let ns: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(ns.first(), Some(&"write_file"), "got: {ns:?}");
        assert!(ns.contains(&"edit_file"), "got: {ns:?}");
    }

    #[tokio::test]
    async fn registry_search_excludes_alias_rows() {
        let reg = seeded_registry().await;
        reg.register_alias("write", "write_file").await;
        reg.register_alias("bash", "exec").await;

        let hits = reg.search_tools("write file", 10).await;
        let ns: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert!(!ns.contains(&"write"), "alias leaked into results: {ns:?}");
        // No duplicates at all.
        let mut deduped = ns.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), ns.len(), "duplicate rows: {ns:?}");
        assert_eq!(ns.first(), Some(&"write_file"));
    }

    #[tokio::test]
    async fn registry_search_hides_policy_denied_tools() {
        let reg = seeded_registry().await;
        reg.set_policy(crate::ToolPolicy::deny_only(["exec"])).await;
        let hits = reg.search_tools("run command", 10).await;
        assert!(
            !hits.iter().any(|h| h.name == "exec"),
            "denied tool surfaced: {:?}",
            hits.iter().map(|h| &h.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn registry_search_empty_for_garbage() {
        let reg = seeded_registry().await;
        assert!(reg.search_tools("zzqqx", 10).await.is_empty());
    }
}
