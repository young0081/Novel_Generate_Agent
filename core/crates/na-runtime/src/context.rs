//! Context-window management and history compression.
//!
//! A model has a finite context window. As a writing session grows, the raw
//! [`Message`] history will eventually overflow it. [`ContextManager`] solves
//! this two ways:
//!
//! * [`window`](ContextManager::window) selects the most recent messages that
//!   fit a token budget while *always* keeping the system messages (the
//!   instructions / tool catalog must never be dropped). This is the cheap,
//!   lossless-for-recent path applied on every model call.
//!
//! * [`compress`](ContextManager::compress) is the lossy fallback for when the
//!   history is genuinely too long: it folds the *oldest* messages into a single
//!   structured summary, **writes that summary into long-term
//!   [`MemoryStore`]** (so nothing is truly lost and it becomes RAG-recallable),
//!   and replaces those messages with one concise summary message. This realizes
//!   "context compression" wired into the memory subsystem.
//!
//! Summarization is pluggable via the object-safe [`Summarizer`] trait. The
//! built-in [`HeuristicSummarizer`] needs no model and works fully offline, so
//! compression is deterministic and testable.

use na_common::time::now_millis;
use na_common::Result;
use na_memory::{MemoryKind, MemoryStore};

use crate::message::{Message, Role};
use crate::session::Session;

/// A boxed, `Send` future with an explicit lifetime — the object-safe stand-in
/// for `async fn` in the [`Summarizer`] trait (Rust async-fn-in-trait is not
/// `dyn`-compatible).
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Estimate the number of model tokens `text` would consume.
///
/// We avoid a real tokenizer dependency and use a robust heuristic tuned for the
/// mixed Chinese/English prose this system writes:
///
/// * Each CJK ideograph counts as ~1 token (Chinese is dense; one char ≈ one
///   token for most tokenizers).
/// * Every other run of characters (ASCII words, punctuation, whitespace) is
///   estimated at ~1 token per 4 characters, matching the common English
///   rule-of-thumb.
///
/// The estimate is intentionally an upper-ish bound so windowing stays safely
/// inside the real limit.
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other_chars = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other_chars += 1;
        }
    }
    // Non-CJK: ~4 chars per token, rounded up.
    let other_tokens = other_chars.div_ceil(4);
    cjk + other_tokens
}

/// Estimate the tokens a single message contributes (content plus a small fixed
/// per-message and per-tool-call overhead for role tags / structure).
pub fn estimate_message_tokens(message: &Message) -> usize {
    // Fixed overhead per message for the role marker / framing.
    let mut total = 4 + estimate_tokens(&message.content);
    if let Some(call) = &message.tool_call {
        total += 4 + estimate_tokens(&call.name);
        total += estimate_tokens(&call.args.to_string());
    }
    if let Some(res) = &message.tool_result {
        total += 4 + estimate_tokens(&res.name);
    }
    total
}

/// Whether `c` is a CJK ideograph (counted as one token).
fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x4E00..=0x9FFF).contains(&u)
        || (0x3400..=0x4DBF).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0x20000..=0x2A6DF).contains(&u)
        // Common CJK punctuation / fullwidth forms also read as ~1 token.
        || (0x3000..=0x303F).contains(&u)
        || (0xFF00..=0xFFEF).contains(&u)
}

/// A text summarizer. Object-safe (via [`BoxFuture`]) and `Send + Sync` so the
/// runtime can hold a `Box<dyn Summarizer>` and swap in a model-backed
/// implementation later without changing the compression logic.
pub trait Summarizer: Send + Sync {
    /// Produce a concise summary of `text`.
    fn summarize<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String>>;
}

/// A dependency-free, deterministic summarizer that needs no model.
///
/// It produces a *structured* digest: a short header, then the leading sentences
/// up to a character budget, then an ellipsis marker noting how much was elided.
/// This is good enough to preserve the gist of older turns for both the model
/// context and the long-term memory record, and it keeps compression fully
/// offline and testable.
#[derive(Debug, Clone, Copy)]
pub struct HeuristicSummarizer {
    /// Approximate maximum characters of body to keep in a summary.
    pub max_chars: usize,
}

impl Default for HeuristicSummarizer {
    fn default() -> Self {
        HeuristicSummarizer { max_chars: 600 }
    }
}

impl HeuristicSummarizer {
    /// Construct with an explicit body budget.
    pub fn new(max_chars: usize) -> Self {
        HeuristicSummarizer { max_chars }
    }

    /// The synchronous core (also used directly by [`ContextManager`]).
    pub fn summarize_sync(&self, text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let total_chars = trimmed.chars().count();
        if total_chars <= self.max_chars {
            return trimmed.to_string();
        }

        // Keep whole leading sentences up to the budget so we don't cut a word
        // (or a CJK clause) mid-stream.
        let mut kept = String::new();
        let mut kept_chars = 0usize;
        for sentence in split_sentences(trimmed) {
            let s_chars = sentence.chars().count();
            if kept_chars + s_chars > self.max_chars && !kept.is_empty() {
                break;
            }
            kept.push_str(sentence);
            kept_chars += s_chars;
            if kept_chars >= self.max_chars {
                break;
            }
        }
        if kept.is_empty() {
            // A single huge sentence: hard-truncate on a char boundary.
            kept = trimmed.chars().take(self.max_chars).collect();
        }
        let elided = total_chars.saturating_sub(kept.chars().count());
        if elided > 0 {
            format!("{}… [+{elided} chars elided]", kept.trim_end())
        } else {
            kept
        }
    }
}

impl Summarizer for HeuristicSummarizer {
    fn summarize<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String>> {
        let out = self.summarize_sync(text);
        Box::pin(async move { Ok(out) })
    }
}

/// Split text into sentence-ish chunks, keeping the terminating punctuation so
/// reassembly is faithful. Recognizes both ASCII (`.`, `!`, `?`) and common CJK
/// terminators (`。！？` and newlines).
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | '\n') {
            let end = i + ch.len_utf8();
            out.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    if out.is_empty() && !text.is_empty() {
        out.push(text);
    }
    out
}

/// Manages how much conversation fits the model and compresses overflow.
#[derive(Debug, Clone)]
pub struct ContextManager {
    /// Token budget the windowed history must fit within.
    pub max_tokens: usize,
    /// When the *full* history exceeds this many tokens,
    /// [`compress`](Self::compress) will fold the oldest messages away.
    pub compress_threshold_tokens: usize,
    /// How many of the most recent messages to always keep verbatim during
    /// compression (so the agent never loses its immediate working context).
    pub keep_recent: usize,
    /// Max characters of body the built-in summarizer keeps.
    pub summary_max_chars: usize,
}

impl Default for ContextManager {
    fn default() -> Self {
        ContextManager {
            max_tokens: 8_000,
            compress_threshold_tokens: 6_000,
            keep_recent: 6,
            summary_max_chars: 600,
        }
    }
}

impl ContextManager {
    /// Construct with an explicit window budget; other fields take sensible
    /// defaults derived from it.
    pub fn new(max_tokens: usize) -> Self {
        ContextManager {
            max_tokens,
            compress_threshold_tokens: max_tokens.saturating_mul(3) / 4,
            keep_recent: 6,
            summary_max_chars: 600,
        }
    }

    /// Builder: set how many recent messages are kept verbatim on compression.
    pub fn keep_recent(mut self, n: usize) -> Self {
        self.keep_recent = n;
        self
    }

    /// Builder: set the compression threshold (in tokens).
    pub fn compress_threshold(mut self, tokens: usize) -> Self {
        self.compress_threshold_tokens = tokens;
        self
    }

    /// Total estimated tokens of a message slice.
    pub fn total_tokens(&self, messages: &[Message]) -> usize {
        messages.iter().map(estimate_message_tokens).sum()
    }

    /// Select the most recent messages that fit [`max_tokens`](Self::max_tokens),
    /// **always** keeping every system message regardless of budget. System
    /// messages are emitted first (in original order), followed by the most
    /// recent non-system messages that fit the remaining budget, in chronological
    /// order.
    pub fn window(&self, messages: &[Message]) -> Vec<Message> {
        // 1. Always keep system messages; they cost against the budget first.
        let mut system_tokens = 0usize;
        let mut systems: Vec<&Message> = Vec::new();
        for m in messages {
            if m.role == Role::System {
                system_tokens += estimate_message_tokens(m);
                systems.push(m);
            }
        }

        let remaining = self.max_tokens.saturating_sub(system_tokens);

        // 2. Walk non-system messages from newest to oldest, keeping those that
        //    fit the remaining budget.
        let mut kept_rev: Vec<&Message> = Vec::new();
        let mut used = 0usize;
        for m in messages.iter().rev() {
            if m.role == Role::System {
                continue;
            }
            let cost = estimate_message_tokens(m);
            if used + cost > remaining {
                break;
            }
            used += cost;
            kept_rev.push(m);
        }
        kept_rev.reverse(); // back to chronological order

        // 3. Reassemble: systems first (instructions), then recent dialogue.
        let mut out = Vec::with_capacity(systems.len() + kept_rev.len());
        out.extend(systems.into_iter().cloned());
        out.extend(kept_rev.into_iter().cloned());
        out
    }

    /// Whether the full history currently exceeds the compression threshold.
    pub fn should_compress(&self, session: &Session) -> bool {
        self.total_tokens(session.history()) > self.compress_threshold_tokens
    }

    /// Compress the session if its history exceeds the threshold.
    ///
    /// When triggered, the oldest non-system messages (everything before the last
    /// [`keep_recent`](Self::keep_recent) messages) are folded into a single
    /// structured summary. That summary is:
    ///
    /// 1. written into long-term [`MemoryStore`] as an [`Outline`](MemoryKind::Outline)
    ///    entry (so it is recallable later and nothing is truly lost), and
    /// 2. spliced back into the history as one [`System`](Role::System) summary
    ///    message placed *after* the original system messages but *before* the
    ///    retained recent tail.
    ///
    /// Returns `Ok(true)` if it compressed, `Ok(false)` otherwise.
    pub async fn compress(&self, session: &mut Session, memory: &mut MemoryStore) -> Result<bool> {
        let summarizer = HeuristicSummarizer::new(self.summary_max_chars);
        self.compress_with(session, memory, &summarizer).await
    }

    /// Like [`compress`](Self::compress) but with a caller-supplied
    /// [`Summarizer`] (e.g. a model-backed one).
    pub async fn compress_with(
        &self,
        session: &mut Session,
        memory: &mut MemoryStore,
        summarizer: &dyn Summarizer,
    ) -> Result<bool> {
        if !self.should_compress(session) {
            return Ok(false);
        }

        let messages = &session.messages;

        // Partition indices: leading system messages, the "old" middle to fold,
        // and the recent tail to keep verbatim.
        let leading_systems: Vec<usize> = messages
            .iter()
            .enumerate()
            .take_while(|(_, m)| m.role == Role::System)
            .map(|(i, _)| i)
            .collect();
        let first_non_system = leading_systems.len();
        let total = messages.len();
        // Keep the last `keep_recent` messages.
        let tail_start = total.saturating_sub(self.keep_recent);

        // The fold window is [first_non_system, tail_start). If there is nothing
        // meaningful to fold, do not claim a compression.
        if tail_start <= first_non_system {
            return Ok(false);
        }

        // Render the transcript of the folded window for summarization.
        let folded: Vec<&Message> = messages[first_non_system..tail_start].iter().collect();
        if folded.is_empty() {
            return Ok(false);
        }
        let transcript = render_transcript(&folded);
        let summary_body = summarizer.summarize(&transcript).await?;

        let folded_count = folded.len();
        let title = format!("Conversation summary ({folded_count} earlier messages)");
        let summary_text =
            format!("[compressed summary of {folded_count} earlier messages]\n{summary_body}");

        // 1. Persist into long-term memory (Outline) so it is RAG-recallable and
        //    nothing is truly lost. A best-effort tag aids later retrieval.
        memory.save(
            MemoryKind::Outline,
            title,
            // The recall surface is the (already concise) summary body.
            truncate_chars(&summary_body, 240),
            summary_body.clone(),
            vec!["context_summary".to_string(), "compressed".to_string()],
            3,
        )?;

        // 2. Splice the history: leading systems, one summary system message,
        //    then the retained tail.
        let mut new_messages: Vec<Message> = Vec::with_capacity(total - folded_count + 1);
        for &i in &leading_systems {
            new_messages.push(messages[i].clone());
        }
        let mut summary_msg = Message::system(summary_text);
        // Preserve a stable-ish timestamp ordering: place it just before the tail.
        summary_msg.ts_ms = messages
            .get(tail_start.saturating_sub(1))
            .map(|m| m.ts_ms)
            .unwrap_or_else(now_millis);
        new_messages.push(summary_msg);
        new_messages.extend(messages[tail_start..].iter().cloned());

        session.messages = new_messages;
        session.touch();
        Ok(true)
    }
}

/// Render a slice of messages as a plain-text transcript for summarization.
fn render_transcript(messages: &[&Message]) -> String {
    let mut s = String::new();
    for m in messages {
        s.push_str(m.role.as_str());
        s.push_str(": ");
        if let Some(call) = &m.tool_call {
            s.push_str(&format!("[calls tool {} with {}]", call.name, call.args));
            if !m.content.is_empty() {
                s.push(' ');
            }
        }
        if let Some(res) = &m.tool_result {
            s.push_str(&format!("[result of {} ok={}] ", res.name, res.ok));
        }
        s.push_str(&m.content);
        s.push('\n');
    }
    s
}

/// Truncate a string to at most `n` characters (char-safe).
fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, ToolCallRequest, ToolResultRef};
    use crate::session::Session;
    use na_common::json;
    use na_common::ToolCallId;

    fn temp_memory(tag: &str) -> MemoryStore {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_ctxmem_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        std::fs::create_dir_all(&p).unwrap();
        MemoryStore::open(p.join("memory.jsonl")).unwrap()
    }

    #[test]
    fn estimate_tokens_cjk_one_each() {
        // 4 CJK chars ≈ 4 tokens.
        assert_eq!(estimate_tokens("第一二三"), 4);
    }

    #[test]
    fn estimate_tokens_ascii_quarter() {
        // 8 ASCII chars ≈ 2 tokens.
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        // empty
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_mixed() {
        // "hi你好" => 2 ascii (=>1) + 2 CJK (=>2) = 3
        assert_eq!(estimate_tokens("hi你好"), 3);
    }

    #[test]
    fn window_keeps_system_always() {
        let mgr = ContextManager::new(40);
        let mut msgs = vec![Message::system("SYSTEM INSTRUCTIONS persona tools")];
        // Add lots of long user/assistant turns to overflow the budget.
        for i in 0..40 {
            msgs.push(Message::user(format!(
                "this is a fairly long user message number {i} with extra words"
            )));
        }
        let win = mgr.window(&msgs);
        // The system message must survive.
        assert!(win.iter().any(|m| m.is_system()));
        // And the window must be the system + a recent suffix, far smaller than all.
        assert!(win.len() < msgs.len());
        // The newest message should be present.
        let newest = &msgs[msgs.len() - 1];
        assert!(win.iter().any(|m| m.content == newest.content));
    }

    #[test]
    fn window_orders_systems_first_then_recent() {
        let mgr = ContextManager::new(10_000);
        let msgs = vec![
            Message::system("sys"),
            Message::user("u1"),
            Message::assistant("a1"),
        ];
        let win = mgr.window(&msgs);
        assert_eq!(win.len(), 3);
        assert_eq!(win[0].role, Role::System);
        assert_eq!(win[1].content, "u1");
        assert_eq!(win[2].content, "a1");
    }

    #[test]
    fn window_huge_system_still_returns_systems() {
        // Even if system messages alone exceed the budget, they are kept.
        let mgr = ContextManager::new(2);
        let msgs = vec![
            Message::system("a very long system message far over the tiny budget here"),
            Message::user("hi"),
        ];
        let win = mgr.window(&msgs);
        assert!(win.iter().any(|m| m.is_system()));
    }

    #[test]
    fn heuristic_summarizer_truncates_long_text() {
        let s = HeuristicSummarizer::new(20);
        // 40 sentences of 4 chars each = 160 chars; only the first ~20 chars of
        // body are kept, so the summary (body + short annotation) is far shorter.
        let long: String = (0..40).map(|_| "句子内容。").collect();
        let out = s.summarize_sync(&long);
        assert!(out.contains("elided"), "should mark elision: {out}");
        assert!(
            out.chars().count() < long.chars().count(),
            "summary ({}) must be shorter than source ({})",
            out.chars().count(),
            long.chars().count()
        );
        // The kept body is bounded near the budget (well under the full text).
        assert!(
            out.chars().count() < 60,
            "kept body should be near budget: {out}"
        );
    }

    #[test]
    fn heuristic_summarizer_passes_short_text() {
        let s = HeuristicSummarizer::new(100);
        assert_eq!(s.summarize_sync("short"), "short");
        assert_eq!(s.summarize_sync("   "), "");
    }

    #[tokio::test]
    async fn summarizer_trait_object_works() {
        let s: Box<dyn Summarizer> = Box::new(HeuristicSummarizer::default());
        let out = s.summarize("hello world").await.unwrap();
        assert_eq!(out, "hello world");
    }

    #[tokio::test]
    async fn compress_shrinks_and_writes_memory() {
        let mut mem = temp_memory("compress");
        // Low threshold so we trigger easily; keep only the last 3 messages.
        let mgr = ContextManager::new(10_000)
            .compress_threshold(50)
            .keep_recent(3);

        let mut session = Session::new("长篇");
        session.push(Message::system("你是一名小说家，遵守世界观设定。"));
        // 20 sizeable messages to fold.
        for i in 0..20 {
            session.push(Message::user(format!(
                "用户的第 {i} 条很长的消息，包含很多关于剧情和角色的细节描述内容。"
            )));
            session.push(Message::assistant(format!(
                "助手的第 {i} 条回复，继续推进故事情节并描写场景。"
            )));
        }
        let before_len = session.len();
        assert!(mgr.should_compress(&session));

        let compressed = mgr.compress(&mut session, &mut mem).await.unwrap();
        assert!(compressed, "should have compressed");

        // History shrank.
        assert!(session.len() < before_len, "history must shrink");

        // A summary system message remains.
        let summary_present = session
            .history()
            .iter()
            .any(|m| m.is_system() && m.content.contains("compressed summary"));
        assert!(summary_present, "a summary message must remain");

        // The original leading system message is still first.
        assert_eq!(session.history()[0].role, Role::System);
        assert!(session.history()[0].content.contains("小说家"));

        // The recent tail (keep_recent=3) is preserved verbatim at the end.
        assert!(session.history().last().unwrap().content.contains("助手"));

        // A memory entry of kind Outline was written and is recallable.
        let hits = mem.recall("compressed", 5, Some(MemoryKind::Outline), false);
        assert!(
            !hits.is_empty(),
            "summary must be saved to long-term memory"
        );
        assert!(!mem.is_empty());
    }

    #[tokio::test]
    async fn compress_noop_when_under_threshold() {
        let mut mem = temp_memory("noop");
        let mgr = ContextManager::new(10_000).compress_threshold(100_000);
        let mut session = Session::new("t");
        session.push(Message::system("sys"));
        session.push(Message::user("a short message"));
        let did = mgr.compress(&mut session, &mut mem).await.unwrap();
        assert!(!did);
        assert_eq!(session.len(), 2);
        assert_eq!(mem.len(), 0);
    }

    #[tokio::test]
    async fn compress_preserves_tool_messages_in_summary_context() {
        // Sanity: render_transcript includes tool call/result framing.
        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("c1"),
            "write_file",
            json!({ "path": "x" }),
        );
        let m1 = Message::assistant_tool_call("writing", call.clone());
        let m2 = Message::tool("ok", ToolResultRef::new(call.id, "write_file", true, false));
        let refs: Vec<&Message> = vec![&m1, &m2];
        let t = render_transcript(&refs);
        assert!(t.contains("calls tool write_file"));
        assert!(t.contains("result of write_file"));
    }
}
