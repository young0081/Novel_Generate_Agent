//! Output-processing pipeline applied to everything a tool produces.
//!
//! Raw tool output (file bytes, command stdout/stderr, fetched web pages) is
//! adversarial: it may be binary, full of terminal escape codes, contain leaked
//! secrets, or be enormous. [`OutputProcessor`] runs a fixed pipeline that makes
//! the text safe and bounded before it reaches the model:
//!
//! 1. **Binary detection** — a NUL byte or more than 30% non-text bytes marks the
//!    payload binary; we emit `"[binary data: N bytes]"` and stop.
//! 2. **Lossy UTF-8 decode** — invalid sequences become the replacement char.
//! 3. **ANSI stripping** — CSI/SGR sequences (`\x1b[...m`, cursor moves, ...) and
//!    other escape sequences are removed.
//! 4. **Secret redaction** — AWS keys, `api_key=…`, `Bearer …`, PEM private
//!    keys, long hex/base64 blobs, and `SECRET=value` assignments are replaced
//!    with `[REDACTED:<kind>]`, counting each redaction.
//! 5. **Line truncation** — when over `max_lines`, keep the first `head_lines`
//!    and last `tail_lines` with a `… <N> lines omitted …` marker.
//! 6. **Byte cap** — if still over `max_bytes`, keep a head and tail of the bytes
//!    with a `… <N> bytes omitted …` marker.
//!
//! The error-first helper [`OutputProcessor::process_result`] composes stdout and
//! stderr so that, on failure, the (processed) error text comes *first* — the
//! model should see what went wrong before the noisy success output.

use regex::Regex;
use std::sync::OnceLock;

/// Size/line ceilings for [`OutputProcessor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLimits {
    /// Maximum bytes of text to keep.
    pub max_bytes: usize,
    /// Maximum number of lines before head/tail truncation kicks in.
    pub max_lines: usize,
    /// Lines kept from the start when truncating.
    pub head_lines: usize,
    /// Lines kept from the end when truncating.
    pub tail_lines: usize,
}

impl Default for OutputLimits {
    /// 16 KiB, 400 lines (200 head + 100 tail kept).
    fn default() -> Self {
        OutputLimits {
            max_bytes: 16 * 1024,
            max_lines: 400,
            head_lines: 200,
            tail_lines: 100,
        }
    }
}

/// The result of running [`OutputProcessor::process`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedOutput {
    /// The cleaned, bounded text.
    pub text: String,
    /// Byte length of `text`.
    pub bytes: usize,
    /// Line count of `text`.
    pub lines: usize,
    /// Whether truncation occurred (lines and/or bytes).
    pub truncated: bool,
    /// Whether the input was detected as binary.
    pub was_binary: bool,
    /// Number of secret redactions performed.
    pub redactions: u32,
}

/// Applies the output pipeline using a configured set of [`OutputLimits`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputProcessor {
    /// The limits in force.
    pub limits: OutputLimits,
}

impl OutputProcessor {
    /// Construct with explicit limits.
    pub fn new(limits: OutputLimits) -> Self {
        OutputProcessor { limits }
    }

    /// Run the full pipeline on raw bytes.
    pub fn process(&self, raw: &[u8]) -> ProcessedOutput {
        // (a) binary detection.
        if is_binary(raw) {
            let text = format!("[binary data: {} bytes]", raw.len());
            let bytes = text.len();
            return ProcessedOutput {
                text,
                bytes,
                lines: 1,
                truncated: false,
                was_binary: true,
                redactions: 0,
            };
        }

        // (b) lossy UTF-8 decode.
        let decoded = String::from_utf8_lossy(raw).into_owned();

        // (c) strip ANSI.
        let stripped = strip_ansi(&decoded);

        // (d) redact secrets.
        let (redacted, redactions) = redact_secrets(&stripped);

        // (e) line truncation.
        let (line_trunc, lines_truncated) = self.truncate_lines(&redacted);

        // (f) byte cap.
        let (byte_trunc, bytes_truncated) = self.truncate_bytes(&line_trunc);

        let lines = count_lines(&byte_trunc);
        ProcessedOutput {
            bytes: byte_trunc.len(),
            text: byte_trunc,
            lines,
            truncated: lines_truncated || bytes_truncated,
            was_binary: false,
            redactions,
        }
    }

    /// Error-first composition of a command's streams.
    ///
    /// Each stream is processed independently. When `exit_code != 0` or `stderr`
    /// is non-empty, the processed stderr is placed *first* (under a `[stderr]`
    /// heading) followed by the processed stdout (under `[stdout]`). On clean
    /// success only the processed stdout is returned.
    ///
    /// The aggregate [`ProcessedOutput`] reports combined byte/line counts and
    /// ORs the binary/truncation flags and sums redactions.
    pub fn process_result(&self, stdout: &[u8], stderr: &[u8], exit_code: i32) -> ProcessedOutput {
        let out = self.process(stdout);
        let errp = self.process(stderr);

        let is_error = exit_code != 0 || !stderr_is_blank(&errp.text);

        let text = if is_error {
            let mut s = String::new();
            s.push_str(&format!("[exit {exit_code}]\n"));
            if !errp.text.is_empty() {
                s.push_str("[stderr]\n");
                s.push_str(&errp.text);
                if !errp.text.ends_with('\n') {
                    s.push('\n');
                }
            }
            if !out.text.is_empty() {
                s.push_str("[stdout]\n");
                s.push_str(&out.text);
            }
            s
        } else {
            out.text.clone()
        };

        ProcessedOutput {
            bytes: text.len(),
            lines: count_lines(&text),
            text,
            truncated: out.truncated || errp.truncated,
            was_binary: out.was_binary || errp.was_binary,
            redactions: out.redactions + errp.redactions,
        }
    }

    /// Keep `head_lines` + marker + `tail_lines` when over `max_lines`.
    fn truncate_lines(&self, text: &str) -> (String, bool) {
        let lines: Vec<&str> = text.split('\n').collect();
        let total = lines.len();
        if total <= self.limits.max_lines {
            return (text.to_string(), false);
        }
        let head = self.limits.head_lines.min(total);
        let tail = self.limits.tail_lines.min(total.saturating_sub(head));
        let omitted = total.saturating_sub(head + tail);
        if omitted == 0 {
            return (text.to_string(), false);
        }

        let mut out = String::with_capacity(text.len());
        for (i, line) in lines.iter().take(head).enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
        }
        out.push('\n');
        out.push_str(&format!("... {omitted} lines omitted ..."));
        for line in lines.iter().skip(total - tail) {
            out.push('\n');
            out.push_str(line);
        }
        (out, true)
    }

    /// Keep a head and tail of bytes (split on a char boundary) when over
    /// `max_bytes`, with a middle marker.
    fn truncate_bytes(&self, text: &str) -> (String, bool) {
        let len = text.len();
        if len <= self.limits.max_bytes {
            return (text.to_string(), false);
        }
        // Reserve room for the marker; split the remaining budget head/tail.
        let omitted = len - self.limits.max_bytes;
        let marker = format!("\n... {omitted} bytes omitted ...\n");
        // Budget for actual content (never negative); keep ~2/3 head, 1/3 tail.
        let budget = self.limits.max_bytes.saturating_sub(marker.len());
        let head_budget = (budget * 2) / 3;
        let tail_budget = budget.saturating_sub(head_budget);

        let head_end = floor_char_boundary(text, head_budget);
        let tail_start = ceil_char_boundary(text, len.saturating_sub(tail_budget));

        // Guard against overlap (tiny budgets).
        let (head_end, tail_start) = if head_end >= tail_start {
            (head_end, head_end)
        } else {
            (head_end, tail_start)
        };

        let mut out = String::with_capacity(head_end + marker.len() + (len - tail_start));
        out.push_str(&text[..head_end]);
        out.push_str(&marker);
        out.push_str(&text[tail_start..]);
        (out, true)
    }
}

/// Heuristic: is `raw` binary? A NUL byte, or >30% bytes outside the printable
/// ASCII + common whitespace range *and* not valid leading UTF-8 multibyte.
fn is_binary(raw: &[u8]) -> bool {
    if raw.is_empty() {
        return false;
    }
    if raw.contains(&0u8) {
        return true;
    }
    // Count "non-text" bytes: control chars other than \t \n \r, and bytes that
    // are not part of a valid UTF-8 string. We approximate by decoding lossily
    // and counting replacement characters plus raw control bytes.
    let mut nontext = 0usize;
    for &b in raw {
        let is_text =
            b == b'\t' || b == b'\n' || b == b'\r' || (0x20..=0x7e).contains(&b) || b >= 0x80; // high bytes may be UTF-8 multibyte; counted below
        if !is_text {
            nontext += 1;
        }
    }
    // For high bytes, lean on the decoder: count replacement chars as non-text.
    let decoded = String::from_utf8_lossy(raw);
    let replacements = decoded.chars().filter(|&c| c == '\u{FFFD}').count();
    let total = raw.len();
    let ratio = (nontext + replacements) as f64 / total as f64;
    ratio > 0.30
}

/// Remove ANSI/VT escape sequences.
fn strip_ansi(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // CSI sequences: ESC [ ... final-byte; OSC: ESC ] ... BEL/ST; plus
        // single-char escapes like ESC ( B. Also strip a lone ESC.
        Regex::new(
            r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]|\x1b",
        )
        .expect("ANSI regex is valid")
    });
    re.replace_all(text, "").into_owned()
}

/// Redact common secret shapes. Returns the redacted text and the count.
fn redact_secrets(text: &str) -> (String, u32) {
    let mut count = 0u32;
    let mut out = text.to_string();

    for spec in redaction_specs() {
        let replaced = spec.re.replace_all(&out, |caps: &regex::Captures| {
            count += 1;
            // If the pattern captured a "prefix" group (group 1) we keep it and
            // redact the value; otherwise replace the whole match.
            match caps.get(1) {
                Some(prefix) => format!("{}[REDACTED:{}]", prefix.as_str(), spec.kind),
                None => format!("[REDACTED:{}]", spec.kind),
            }
        });
        out = replaced.into_owned();
    }

    (out, count)
}

/// A single redaction rule: a regex and the label used in its replacement.
struct RedactionSpec {
    re: &'static Regex,
    kind: &'static str,
}

/// The ordered list of redaction rules. Ordered so the most specific shapes run
/// first (e.g. PEM blocks before generic base64).
fn redaction_specs() -> Vec<RedactionSpec> {
    macro_rules! lazy_re {
        ($name:ident, $pat:expr) => {{
            static CELL: OnceLock<Regex> = OnceLock::new();
            CELL.get_or_init(|| Regex::new($pat).expect(concat!("regex ", stringify!($name))))
        }};
    }

    vec![
        // PEM private key blocks (multi-line). `(?s)` makes `.` match newlines.
        RedactionSpec {
            re: lazy_re!(
                pem,
                r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----"
            ),
            kind: "private_key",
        },
        // AWS access key id.
        RedactionSpec {
            re: lazy_re!(aws, r"AKIA[0-9A-Z]{16}"),
            kind: "aws_key",
        },
        // Bearer tokens in Authorization headers / logs.
        RedactionSpec {
            re: lazy_re!(bearer, r"(?i)(Bearer\s+)[A-Za-z0-9\-._~+/]+=*"),
            kind: "bearer",
        },
        // key=value / key: value where the key looks secret. Captures the prefix
        // (key + separator) in group 1 so it is preserved.
        RedactionSpec {
            re: lazy_re!(
                kv,
                r#"(?i)((?:api[_-]?key|secret|token|password|passwd|pwd|access[_-]?key)\s*[:=]\s*)["']?[^\s"',]+["']?"#
            ),
            kind: "credential",
        },
        // Long hex blob (>=32 hex chars) — likely a hash/key. Word-bounded.
        RedactionSpec {
            re: lazy_re!(hex, r"\b[0-9a-fA-F]{32,}\b"),
            kind: "hex",
        },
        // Long base64-ish blob (>=32 chars of base64 alphabet).
        RedactionSpec {
            re: lazy_re!(b64, r"\b[A-Za-z0-9+/]{32,}={0,2}\b"),
            kind: "base64",
        },
    ]
}

/// Count newlines + 1 (an empty string is one line; trailing newline counted).
fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        return 1;
    }
    text.split('\n').count()
}

/// Is the processed stderr effectively blank (only whitespace)?
fn stderr_is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

/// Largest index `<= idx` that lies on a UTF-8 char boundary.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest index `>= idx` that lies on a UTF-8 char boundary.
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through() {
        let p = OutputProcessor::default();
        let out = p.process(b"hello\nworld");
        assert_eq!(out.text, "hello\nworld");
        assert!(!out.truncated);
        assert!(!out.was_binary);
        assert_eq!(out.redactions, 0);
        assert_eq!(out.lines, 2);
    }

    #[test]
    fn binary_detected_by_nul() {
        let p = OutputProcessor::default();
        let out = p.process(b"abc\0def");
        assert!(out.was_binary);
        assert!(out.text.starts_with("[binary data:"));
        assert!(out.text.contains("7 bytes"));
    }

    #[test]
    fn binary_detected_by_ratio() {
        let p = OutputProcessor::default();
        // Many control bytes (0x01) -> > 30% non-text.
        let raw = vec![0x01u8; 100];
        let out = p.process(&raw);
        assert!(out.was_binary);
    }

    #[test]
    fn high_utf8_is_not_binary() {
        let p = OutputProcessor::default();
        let out = p.process("龙王在北方的雪山之巅。".as_bytes());
        assert!(!out.was_binary);
        assert!(out.text.contains("龙王"));
    }

    #[test]
    fn ansi_is_stripped() {
        let p = OutputProcessor::default();
        // Red "error" then reset, plus a cursor move.
        let raw = "\x1b[31merror\x1b[0m\x1b[2Kdone";
        let out = p.process(raw.as_bytes());
        assert_eq!(out.text, "errordone");
    }

    #[test]
    fn ansi_osc_sequence_stripped() {
        let p = OutputProcessor::default();
        // OSC set-title sequence terminated by BEL.
        let raw = "before\x1b]0;my title\x07after";
        let out = p.process(raw.as_bytes());
        assert_eq!(out.text, "beforeafter");
    }

    #[test]
    fn redacts_aws_key() {
        let p = OutputProcessor::default();
        let out = p.process(b"key is AKIAIOSFODNN7EXAMPLE here");
        assert!(out.text.contains("[REDACTED:aws_key]"));
        assert!(!out.text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(out.redactions, 1);
    }

    #[test]
    fn redacts_api_key_assignment_keeps_key_name() {
        let p = OutputProcessor::default();
        let out = p.process(b"api_key=supersecretvalue123");
        assert!(out.text.contains("api_key="), "{}", out.text);
        assert!(out.text.contains("[REDACTED:credential]"), "{}", out.text);
        assert!(!out.text.contains("supersecretvalue123"));
    }

    #[test]
    fn redacts_bearer_token() {
        let p = OutputProcessor::default();
        let out = p.process(b"Authorization: Bearer abcDEF123456ghiJKL789mno");
        assert!(
            out.text.contains("Bearer [REDACTED:bearer]"),
            "{}",
            out.text
        );
    }

    #[test]
    fn redacts_pem_private_key() {
        let p = OutputProcessor::default();
        let raw =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIBOwIBAAJB\nabc\n-----END RSA PRIVATE KEY-----";
        let out = p.process(raw.as_bytes());
        assert!(out.text.contains("[REDACTED:private_key]"));
        assert!(!out.text.contains("MIIBOwIBAAJB"));
    }

    #[test]
    fn redacts_long_hex_blob() {
        let p = OutputProcessor::default();
        let hex = "deadbeef".repeat(5); // 40 hex chars
        let out = p.process(format!("hash {hex} end").as_bytes());
        assert!(out.text.contains("[REDACTED:hex]"), "{}", out.text);
    }

    #[test]
    fn head_and_tail_kept_with_marker() {
        let limits = OutputLimits {
            max_bytes: 100_000,
            max_lines: 10,
            head_lines: 3,
            tail_lines: 2,
        };
        let p = OutputProcessor::new(limits);
        let input: String = (1..=100)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = p.process(input.as_bytes());
        assert!(out.truncated);
        assert!(out.text.starts_with("line1\nline2\nline3\n"));
        assert!(out.text.contains("lines omitted"));
        assert!(out.text.trim_end().ends_with("line100"));
        // 95 lines omitted (100 - 3 - 2).
        assert!(out.text.contains("95 lines omitted"));
    }

    #[test]
    fn byte_cap_truncates_middle() {
        let limits = OutputLimits {
            max_bytes: 200,
            max_lines: 100_000,
            head_lines: 10,
            tail_lines: 10,
        };
        let p = OutputProcessor::new(limits);
        // Use spaced tokens so the secret-redactor's long-blob rules (which are
        // word-boundary anchored) do not collapse the whole thing first.
        let input = "x ".repeat(500); // 1000 bytes, no 32+ char blob
        let out = p.process(input.as_bytes());
        assert_eq!(out.redactions, 0, "test input must not be redacted");
        assert!(out.truncated);
        assert!(out.text.contains("bytes omitted"));
        assert!(out.text.len() <= 240, "len was {}", out.text.len());
        assert!(out.text.starts_with('x'));
    }

    #[test]
    fn byte_cap_respects_char_boundaries() {
        let limits = OutputLimits {
            max_bytes: 40,
            max_lines: 100_000,
            head_lines: 10,
            tail_lines: 10,
        };
        let p = OutputProcessor::new(limits);
        // Many multi-byte chars.
        let input = "龙".repeat(100); // 300 bytes
        let out = p.process(input.as_bytes());
        // Must still be valid UTF-8 (String guarantees it) and contain marker.
        assert!(out.text.contains("bytes omitted"));
        assert!(out.text.starts_with('龙'));
    }

    #[test]
    fn process_result_error_first() {
        let p = OutputProcessor::default();
        let out = p.process_result(b"normal output", b"a wild error appeared", 1);
        let err_pos = out.text.find("a wild error appeared").unwrap();
        let out_pos = out.text.find("normal output").unwrap();
        assert!(err_pos < out_pos, "stderr must come before stdout");
        assert!(out.text.contains("[stderr]"));
        assert!(out.text.contains("[stdout]"));
        assert!(out.text.contains("[exit 1]"));
        assert!(!out.ok_implies()); // helper below
    }

    #[test]
    fn process_result_clean_success_is_stdout_only() {
        let p = OutputProcessor::default();
        let out = p.process_result(b"all good", b"", 0);
        assert_eq!(out.text, "all good");
        assert!(!out.text.contains("[stderr]"));
    }

    #[test]
    fn process_result_nonzero_exit_with_empty_stderr() {
        let p = OutputProcessor::default();
        let out = p.process_result(b"partial", b"", 2);
        assert!(out.text.contains("[exit 2]"));
        assert!(out.text.contains("partial"));
    }

    // Tiny test-only helper to assert error framing without exposing ok on
    // ProcessedOutput (which intentionally has no ok field).
    impl ProcessedOutput {
        fn ok_implies(&self) -> bool {
            !self.text.contains("[exit ")
        }
    }
}
