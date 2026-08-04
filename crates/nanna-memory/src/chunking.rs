//! Splitting a memory into embeddable chunks.
//!
//! A memory row's content is unbounded — that is the point of the storage
//! change. An embedding model's input is not. Chunking is what reconciles the
//! two, and it has exactly one hard invariant:
//!
//! > **Concatenating a memory's chunks in ordinal order reproduces its content
//! > byte for byte.**
//!
//! Everything else here is subordinate to that. Chunks are the retrieval index
//! *and* the pagination unit, so any character a chunk drops is a character the
//! reader can never get back, and any character duplicated across chunks is a
//! character the reader sees twice. That rules out overlapping windows, which
//! are the usual retrieval-quality trick: they would buy slightly better recall
//! of boundary-spanning phrases at the cost of making paged reads repeat text
//! and making reconstruction lossy. Recall is recoverable by other means — the
//! parent row is returned whole once any of its chunks matches. Losslessness is
//! not recoverable by any means.

use crate::service::{chunk_max_chars_for_window, MEMORY_CHUNK_TARGET_CHARS};

/// Identifies the splitting algorithm that produced a chunk set.
///
/// Stored per chunk so a future change to the split points can be detected and
/// re-chunked, rather than leaving a store with two incompatible chunkings that
/// look identical from the outside.
pub const CHUNKER_VERSION: i64 = 1;

/// How far back from the window edge a nicer break point is worth looking.
///
/// Breaking on a paragraph or sentence boundary means giving up some of the
/// window. Searching the whole window would let a single early newline produce
/// a one-character chunk and stall progress; searching the back half bounds the
/// waste at half a chunk and guarantees every chunk after a break is at least
/// half full. It is the same reasoning as a minimum-fill rule, expressed as a
/// search bound instead of a rejection test.
const BREAK_SEARCH_FRACTION: usize = 2;

/// The chunking configuration derived from a specific embedding model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkParams {
    /// Hard upper bound on one chunk, in characters.
    pub max_chars: usize,
    /// Algorithm identity — see [`CHUNKER_VERSION`].
    pub version: i64,
}

impl ChunkParams {
    /// Whether chunks written under `self` are still valid under `other`.
    ///
    /// Both the size and the algorithm have to match. A chunk written for a
    /// 512-token embedder is not wrong under a 8192-token one, but it is
    /// needlessly fine-grained, and — more to the point — a *smaller* new
    /// bound means existing chunks now overrun the model and would embed
    /// truncated.
    #[must_use]
    pub const fn supersedes(&self, other: &Self) -> bool {
        self.max_chars != other.max_chars || self.version != other.version
    }
}

/// Derive chunking parameters from an embedding model's input window.
///
/// `None` means the window is not discoverable (every OpenAI-compatible
/// embeddings endpoint, and any Ollama model whose GGUF omits the key). That is
/// not permission to send arbitrarily long text — it falls back to the
/// retrieval-granularity target, which is what the store used before any of
/// this existed.
#[must_use]
pub fn derive_chunk_params(window_tokens: Option<usize>) -> ChunkParams {
    ChunkParams {
        max_chars: match window_tokens {
            Some(w) => chunk_max_chars_for_window(w),
            None => MEMORY_CHUNK_TARGET_CHARS,
        },
        version: CHUNKER_VERSION,
    }
}

/// One piece of a memory's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Position in the parent, from zero. Also the pagination key.
    pub ordinal: i64,
    /// The slice itself. Concatenating these in ordinal order rebuilds the
    /// parent exactly.
    pub content: String,
    /// Character offset of this chunk's first character within the parent.
    pub char_start: i64,
    /// Character offset one past this chunk's last character.
    pub char_end: i64,
}

/// Split `content` into chunks of at most `params.max_chars` characters.
///
/// Empty content yields no chunks: there is nothing to embed and nothing to
/// page through, and the concatenation invariant holds trivially.
///
/// Breaks prefer, in order: a paragraph end, a line end, a sentence end, any
/// whitespace, and finally the window edge itself. A break is only taken in the
/// back half of the window (see [`BREAK_SEARCH_FRACTION`]). Delimiters stay
/// with the chunk that precedes them — nothing is trimmed, because trimming is
/// how a splitter silently loses text.
#[must_use]
pub fn chunk_text(content: &str, params: &ChunkParams) -> Vec<Chunk> {
    let max_chars = params.max_chars.max(1);
    let mut chunks = Vec::new();
    // Byte cursor into `content`; `char_pos` shadows it in characters, since
    // the stored offsets are character offsets but Rust slices by byte.
    let mut byte_pos = 0usize;
    let mut char_pos = 0i64;

    while byte_pos < content.len() {
        let rest = &content[byte_pos..];

        // Byte length of the first `max_chars` characters of `rest`, or all of
        // `rest` when it is shorter than that.
        let window_bytes = rest
            .char_indices()
            .nth(max_chars)
            .map_or(rest.len(), |(byte_idx, _)| byte_idx);

        let take_bytes = if byte_pos + window_bytes >= content.len() {
            // Final chunk: no break to look for, and looking would only split
            // the tail into a needless extra chunk.
            window_bytes
        } else {
            find_break(&rest[..window_bytes]).unwrap_or(window_bytes)
        };

        let piece = &rest[..take_bytes];
        let piece_chars = i64::try_from(piece.chars().count()).unwrap_or(i64::MAX);
        chunks.push(Chunk {
            ordinal: i64::try_from(chunks.len()).unwrap_or(i64::MAX),
            content: piece.to_string(),
            char_start: char_pos,
            char_end: char_pos + piece_chars,
        });

        debug_assert!(take_bytes > 0, "a break must consume at least one character");
        byte_pos += take_bytes;
        char_pos += piece_chars;
    }

    chunks
}

/// Find the byte offset to cut `window` at, searching its back half for the
/// nicest boundary. Returns `None` when the window has no boundary worth
/// taking, meaning the caller should cut at the window edge.
///
/// The returned offset is always *after* the delimiter, so the delimiter stays
/// in the earlier chunk and no character is consumed by the split itself.
fn find_break(window: &str) -> Option<usize> {
    let floor = window.len() / BREAK_SEARCH_FRACTION;

    // Paragraph, then line. `rfind` gives the last occurrence, which is the
    // one that wastes least of the window.
    for delimiter in ["\n\n", "\r\n\r\n", "\n", "\r\n"] {
        if let Some(idx) = window.rfind(delimiter) {
            let cut = idx + delimiter.len();
            if cut > floor {
                return Some(cut);
            }
        }
    }

    // Sentence end: terminal punctuation followed by whitespace. Checked as a
    // pair so a decimal point or an abbreviation does not read as a sentence.
    let bytes = window.as_bytes();
    let mut best_sentence = None;
    for (idx, window_pair) in bytes.windows(2).enumerate() {
        if matches!(window_pair[0], b'.' | b'!' | b'?') && window_pair[1].is_ascii_whitespace() {
            let cut = idx + 2;
            if cut > floor {
                best_sentence = Some(cut);
            }
        }
    }
    if best_sentence.is_some() {
        return best_sentence;
    }

    // Any whitespace. Walk characters so the cut lands on a UTF-8 boundary.
    let mut best_space = None;
    for (idx, ch) in window.char_indices() {
        if ch.is_whitespace() {
            let cut = idx + ch.len_utf8();
            if cut > floor {
                best_space = Some(cut);
            }
        }
    }
    best_space
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(max_chars: usize) -> ChunkParams {
        ChunkParams {
            max_chars,
            version: CHUNKER_VERSION,
        }
    }

    /// The invariant the whole module exists to preserve. Checked against text
    /// that is deliberately hostile to a naive splitter: multi-byte characters,
    /// runs with no whitespace at all, doubled newlines, and a trailing
    /// delimiter.
    #[test]
    fn concatenating_the_chunks_rebuilds_the_content_exactly() {
        let cases = [
            "",
            "short",
            &"a".repeat(10_000),
            &"héllo wörld ".repeat(500),
            &"para one.\n\npara two!\n\npara three?\n\n".repeat(120),
            &"日本語のテキストです。".repeat(400),
            &format!("{}\n\n{}", "x".repeat(5000), "y".repeat(5000)),
        ];
        for content in cases {
            for max_chars in [1, 7, 64, 512, 3200] {
                let chunks = chunk_text(content, &params(max_chars));
                let rebuilt: String = chunks.iter().map(|c| c.content.as_str()).collect();
                assert_eq!(
                    rebuilt,
                    *content,
                    "lost or duplicated text at max_chars={max_chars}"
                );
            }
        }
    }

    /// A chunk that overruns the model's window gets silently truncated by the
    /// embedder, so this is the bound the whole derivation exists to enforce.
    #[test]
    fn no_chunk_exceeds_the_budget() {
        let content = "sentence one. sentence two! sentence three?\n\n".repeat(400);
        for max_chars in [1, 5, 33, 200, 3200] {
            for chunk in chunk_text(&content, &params(max_chars)) {
                assert!(
                    chunk.content.chars().count() <= max_chars,
                    "chunk of {} chars exceeds budget {max_chars}",
                    chunk.content.chars().count()
                );
            }
        }
    }

    /// Offsets have to locate the chunk inside the parent, or a paged reader
    /// cannot say where it is and a UI cannot highlight it.
    #[test]
    fn offsets_are_contiguous_and_address_the_parent() {
        let content = "The quick brown fox.\n\nJumps over the lazy dog! Again? Yes.\n".repeat(50);
        let chars: Vec<char> = content.chars().collect();
        let chunks = chunk_text(&content, &params(64));

        let mut expected_start = 0i64;
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i as i64, "ordinals must be dense from zero");
            assert_eq!(chunk.char_start, expected_start, "chunks must not have gaps");
            let slice: String = chars
                [usize::try_from(chunk.char_start).unwrap()..usize::try_from(chunk.char_end).unwrap()]
                .iter()
                .collect();
            assert_eq!(slice, chunk.content, "offsets do not address this chunk");
            expected_start = chunk.char_end;
        }
        assert_eq!(
            usize::try_from(expected_start).unwrap(),
            chars.len(),
            "chunks must cover the parent to its end"
        );
    }

    #[test]
    fn a_paragraph_boundary_is_preferred_to_the_window_edge() {
        let content = format!("{}\n\n{}", "a".repeat(60), "b".repeat(60));
        let chunks = chunk_text(&content, &params(100));
        assert!(chunks[0].content.ends_with("\n\n"), "should break after the blank line");
        assert_eq!(chunks[0].content.chars().count(), 62);
    }

    #[test]
    fn a_sentence_boundary_is_preferred_when_there_is_no_line_break() {
        let content = format!("{}. {}", "a".repeat(60), "b".repeat(60));
        let chunks = chunk_text(&content, &params(100));
        assert!(chunks[0].content.ends_with(". "), "should break after the sentence");
    }

    /// A boundary in the front half is not worth taking: honouring it would
    /// emit a nearly-empty chunk and make no progress through a long run.
    #[test]
    fn an_early_boundary_does_not_starve_the_chunk() {
        let content = format!("a\n{}", "b".repeat(500));
        let chunks = chunk_text(&content, &params(100));
        assert!(
            chunks[0].content.chars().count() >= 50,
            "took a break in the front half: {} chars",
            chunks[0].content.chars().count()
        );
    }

    /// Text with no break candidate at all still has to terminate.
    #[test]
    fn an_unbreakable_run_still_chunks() {
        let content = "x".repeat(1000);
        let chunks = chunk_text(&content, &params(100));
        assert_eq!(chunks.len(), 10);
        assert!(chunks.iter().all(|c| c.content.chars().count() == 100));
    }

    /// Multi-byte characters must never be cut in half — that is a panic in
    /// Rust, not a silent corruption, but it is a panic on the ingest path.
    #[test]
    fn a_multibyte_character_is_never_split() {
        let content = "日本語".repeat(1000);
        for max_chars in [1, 2, 3, 17, 100] {
            let chunks = chunk_text(&content, &params(max_chars));
            let rebuilt: String = chunks.iter().map(|c| c.content.as_str()).collect();
            assert_eq!(rebuilt, content);
        }
    }

    #[test]
    fn empty_content_produces_no_chunks() {
        assert!(chunk_text("", &params(3200)).is_empty());
    }

    /// The window is the *hard* bound and the target is the *soft* one; the
    /// smaller has to win in both directions.
    #[test]
    fn params_come_from_the_smaller_of_window_and_target() {
        // 512-token embedder: the window binds.
        let small = derive_chunk_params(Some(512));
        assert!(small.max_chars < MEMORY_CHUNK_TARGET_CHARS);

        // 32k-token embedder: the retrieval target binds.
        let large = derive_chunk_params(Some(32_768));
        assert_eq!(large.max_chars, MEMORY_CHUNK_TARGET_CHARS);

        // Undiscoverable window: fall back to the target, never to "unbounded".
        assert_eq!(derive_chunk_params(None).max_chars, MEMORY_CHUNK_TARGET_CHARS);
    }

    #[test]
    fn a_changed_budget_or_algorithm_supersedes_existing_chunks() {
        let base = derive_chunk_params(Some(512));
        assert!(!base.supersedes(&base), "identical params are not a rewrite");
        assert!(base.supersedes(&derive_chunk_params(Some(8192))));
        assert!(base.supersedes(&ChunkParams {
            version: CHUNKER_VERSION + 1,
            ..base
        }));
    }
}
