//! A small, pure-Rust, in-memory BM25 ranking index.
//!
//! BM25 (Okapi BM25) is the standard sparse lexical ranking function used by
//! search engines. It scores a document `d` against a query `q` as:
//!
//! ```text
//! score(d, q) = Σ_{t ∈ q} IDF(t) · ( f(t,d) · (k1 + 1) )
//!                                  / ( f(t,d) + k1 · (1 - b + b · |d| / avgdl) )
//! ```
//!
//! where `f(t,d)` is the term frequency of `t` in `d`, `|d|` is the document
//! length in tokens, `avgdl` is the average document length, and `IDF(t)` is the
//! inverse document frequency. We use the standard smoothed IDF
//! `ln(1 + (N - n + 0.5) / (n + 0.5))`, which is always non-negative.
//!
//! This index is intentionally dependency-free and deterministic so the
//! retriever in [`crate::memory`] produces stable, testable rankings for both
//! Chinese (CJK) and English text. It does not persist itself; the
//! [`crate::memory::MemoryStore`] rebuilds it from disk on open.

use std::collections::HashMap;

/// BM25 `k1` parameter: controls term-frequency saturation.
pub const DEFAULT_K1: f32 = 1.5;
/// BM25 `b` parameter: controls document-length normalization (0 = none, 1 = full).
pub const DEFAULT_B: f32 = 0.75;

/// One indexed document: its length and the frequency of each term it contains.
#[derive(Debug, Clone)]
struct Doc {
    /// External document identifier supplied by the caller.
    id: String,
    /// Number of tokens in the document (including repeats).
    len: u32,
    /// term -> frequency within this document.
    term_freqs: HashMap<String, u32>,
}

/// An in-memory BM25 index over a collection of tokenized documents.
///
/// Add documents with [`Bm25Index::add`] (each `doc_id` should be unique; adding
/// the same id twice keeps both, so callers that mutate documents should rebuild
/// the index instead). Query with [`Bm25Index::search`].
#[derive(Debug, Clone)]
pub struct Bm25Index {
    k1: f32,
    b: f32,
    docs: Vec<Doc>,
    /// term -> number of documents that contain it (document frequency).
    doc_freq: HashMap<String, u32>,
    /// Sum of all document lengths, used to derive the average length.
    total_len: u64,
}

impl Bm25Index {
    /// Create an empty index with the standard `k1 = 1.5`, `b = 0.75` parameters.
    pub fn new() -> Self {
        Self::with_params(DEFAULT_K1, DEFAULT_B)
    }

    /// Create an empty index with custom BM25 parameters.
    pub fn with_params(k1: f32, b: f32) -> Self {
        Bm25Index {
            k1,
            b,
            docs: Vec::new(),
            doc_freq: HashMap::new(),
            total_len: 0,
        }
    }

    /// Number of documents currently indexed.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether the index contains no documents.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// The average document length across the corpus (0.0 when empty).
    pub fn avg_doc_len(&self) -> f32 {
        if self.docs.is_empty() {
            0.0
        } else {
            self.total_len as f32 / self.docs.len() as f32
        }
    }

    /// Ingest a document. `tokens` is the already-tokenized text; duplicate
    /// tokens are counted as term frequency. An empty token slice still adds a
    /// (zero-length) document so corpus statistics stay consistent.
    pub fn add(&mut self, doc_id: String, tokens: &[String]) {
        let mut term_freqs: HashMap<String, u32> = HashMap::new();
        for tok in tokens {
            *term_freqs.entry(tok.clone()).or_insert(0) += 1;
        }
        // Each distinct term in this document contributes 1 to its doc frequency.
        for term in term_freqs.keys() {
            *self.doc_freq.entry(term.clone()).or_insert(0) += 1;
        }
        let len = tokens.len() as u32;
        self.total_len += len as u64;
        self.docs.push(Doc {
            id: doc_id,
            len,
            term_freqs,
        });
    }

    /// Smoothed inverse document frequency for a term. Always `>= 0`.
    fn idf(&self, term: &str) -> f32 {
        let n = self.doc_freq.get(term).copied().unwrap_or(0) as f32;
        let big_n = self.docs.len() as f32;
        // ln(1 + (N - n + 0.5) / (n + 0.5)) — non-negative for all n in [0, N].
        (1.0 + (big_n - n + 0.5) / (n + 0.5)).ln()
    }

    /// Score every document against the query and return the top `k` as
    /// `(doc_id, score)` pairs sorted by descending score. Documents that match
    /// no query term (score `<= 0`) are excluded. Ties break by ascending
    /// `doc_id` for deterministic output.
    pub fn search(&self, query_tokens: &[String], k: usize) -> Vec<(String, f32)> {
        if self.docs.is_empty() || query_tokens.is_empty() || k == 0 {
            return Vec::new();
        }
        let avgdl = self.avg_doc_len().max(1e-9);

        // Deduplicate query terms but remember the IDF once per distinct term.
        let mut query_terms: HashMap<&str, f32> = HashMap::new();
        for t in query_tokens {
            query_terms.entry(t.as_str()).or_insert_with(|| self.idf(t));
        }

        let mut scored: Vec<(String, f32)> = Vec::with_capacity(self.docs.len());
        for doc in &self.docs {
            let mut score = 0.0f32;
            for (term, &idf) in &query_terms {
                if let Some(&f) = doc.term_freqs.get(*term) {
                    let f = f as f32;
                    let denom = f + self.k1 * (1.0 - self.b + self.b * (doc.len as f32) / avgdl);
                    if denom > 0.0 {
                        score += idf * (f * (self.k1 + 1.0)) / denom;
                    }
                }
            }
            if score > 0.0 {
                scored.push((doc.id.clone(), score));
            }
        }

        // Sort by score desc, then doc_id asc for stable ordering.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_index_returns_nothing() {
        let idx = Bm25Index::new();
        assert!(idx.is_empty());
        assert_eq!(idx.search(&toks(&["anything"]), 5), Vec::new());
        assert_eq!(idx.avg_doc_len(), 0.0);
    }

    #[test]
    fn empty_query_or_zero_k_returns_nothing() {
        let mut idx = Bm25Index::new();
        idx.add("d1".into(), &toks(&["alpha", "beta"]));
        assert!(idx.search(&[], 5).is_empty());
        assert!(idx.search(&toks(&["alpha"]), 0).is_empty());
    }

    #[test]
    fn ranks_more_relevant_document_first() {
        let mut idx = Bm25Index::new();
        // d1 mentions "dragon" twice; d2 once; d3 not at all.
        idx.add(
            "d1".into(),
            &toks(&["the", "dragon", "fought", "the", "dragon"]),
        );
        idx.add("d2".into(), &toks(&["a", "lone", "dragon", "slept"]));
        idx.add("d3".into(), &toks(&["the", "knight", "rode", "away"]));

        let res = idx.search(&toks(&["dragon"]), 10);
        assert_eq!(res.len(), 2, "d3 has no match and must be excluded");
        assert_eq!(res[0].0, "d1", "doc with more occurrences ranks first");
        assert_eq!(res[1].0, "d2");
        assert!(res[0].1 > res[1].1);
    }

    #[test]
    fn rare_term_outweighs_common_term() {
        let mut idx = Bm25Index::new();
        // "the" is in every doc (low IDF); "phoenix" is rare (high IDF).
        idx.add("common".into(), &toks(&["the", "the", "the", "the"]));
        idx.add("rare".into(), &toks(&["the", "phoenix"]));
        for i in 0..8 {
            idx.add(format!("filler{i}"), &toks(&["the", "filler"]));
        }
        let res = idx.search(&toks(&["the", "phoenix"]), 5);
        assert_eq!(res[0].0, "rare", "rare matching term should dominate");
    }

    #[test]
    fn length_normalization_prefers_shorter_doc_for_same_count() {
        let mut idx = Bm25Index::new();
        idx.add("short".into(), &toks(&["quest"]));
        let mut long = toks(&["quest"]);
        long.extend(toks(&["filler"; 50]));
        idx.add("long".into(), &long);
        // add some background docs so avgdl is meaningful
        for i in 0..5 {
            idx.add(format!("bg{i}"), &toks(&["misc", "words"]));
        }
        let res = idx.search(&toks(&["quest"]), 5);
        assert_eq!(
            res[0].0, "short",
            "single-term short doc beats padded long doc"
        );
    }

    #[test]
    fn idf_is_non_negative_even_when_term_in_all_docs() {
        let mut idx = Bm25Index::new();
        idx.add("a".into(), &toks(&["x"]));
        idx.add("b".into(), &toks(&["x"]));
        assert!(idx.idf("x") >= 0.0);
        // term present everywhere still yields a (small, non-negative) score
        let res = idx.search(&toks(&["x"]), 5);
        assert_eq!(res.len(), 2);
        assert!(res.iter().all(|(_, s)| *s >= 0.0));
    }

    #[test]
    fn respects_top_k() {
        let mut idx = Bm25Index::new();
        for i in 0..10 {
            idx.add(format!("d{i}"), &toks(&["match"]));
        }
        let res = idx.search(&toks(&["match"]), 3);
        assert_eq!(res.len(), 3);
    }

    #[test]
    fn ties_break_by_doc_id() {
        let mut idx = Bm25Index::new();
        // identical docs -> identical scores -> ordered by id ascending
        idx.add("zeta".into(), &toks(&["word"]));
        idx.add("alpha".into(), &toks(&["word"]));
        let res = idx.search(&toks(&["word"]), 5);
        assert_eq!(res[0].0, "alpha");
        assert_eq!(res[1].0, "zeta");
    }
}
