//! Tiny glob matcher used by the permission and command policies.
//!
//! The supported wildcard grammar is deliberately small but expressive enough
//! for path- and command-line rules:
//!
//! * `?` — matches exactly one character, *except* a path separator (`/`).
//! * `*` — matches zero or more characters *within a single path segment* (it
//!   will not cross a `/`).
//! * `**` — matches zero or more characters *across* segments (it will cross
//!   `/`), i.e. the usual "globstar".
//!
//! Everything else is treated as a literal, including regex metacharacters such
//! as `.`, `+`, `(`, `[` and so on — they are escaped before being handed to
//! the [`regex`] engine. The whole pattern is anchored (`^...$`) so a match
//! always means the *entire* text matched.
//!
//! We translate the glob into a regular expression once and compile it with the
//! `regex` crate (a declared dependency). Translation never fails for arbitrary
//! input; a malformed pattern simply cannot match anything useful. Should the
//! regex engine itself reject the (escaped, well-formed) program — which should
//! be impossible — we conservatively return `false`.

use regex::Regex;

/// Returns `true` when `text` matches the glob `pattern`.
///
/// The match is anchored: the pattern must describe the *entire* `text`.
///
/// # Examples
/// ```
/// use na_sandbox::glob::glob_match;
/// assert!(glob_match("src/*.rs", "src/main.rs"));
/// assert!(!glob_match("src/*.rs", "src/a/main.rs")); // * does not cross '/'
/// assert!(glob_match("src/**/*.rs", "src/a/b/main.rs"));
/// assert!(glob_match("file?.txt", "file1.txt"));
/// ```
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let re_src = glob_to_regex(pattern);
    match Regex::new(&re_src) {
        Ok(re) => re.is_match(text),
        Err(_) => false,
    }
}

/// Translate a glob pattern into an anchored regular-expression source string.
///
/// Exposed within the crate (and via `pub`) primarily so the behaviour can be
/// unit-tested directly without compiling a [`Regex`].
pub fn glob_to_regex(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() * 2 + 2);
    out.push('^');

    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '*' => {
                // Look ahead for a second '*' to detect the globstar.
                if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    // Consume the run of consecutive '*' so "***" behaves as "**".
                    while i < bytes.len() && bytes[i] == b'*' {
                        i += 1;
                    }
                    // `**/` (globstar followed by a separator) matches zero or
                    // more whole path segments — so `src/**/main.rs` matches
                    // both `src/main.rs` (zero segments) and `src/a/b/main.rs`.
                    // We translate `**/` to an optional "anything-then-slash"
                    // group and swallow the separator. A bare `**` (end of
                    // pattern, or not followed by `/`) crosses everything.
                    if i < bytes.len() && bytes[i] == b'/' {
                        out.push_str("(?:.*/)?");
                        i += 1; // consume the '/'
                    } else {
                        out.push_str(".*");
                    }
                    continue;
                } else {
                    // Single `*`: any char except the path separator, zero or more.
                    out.push_str("[^/]*");
                    i += 1;
                    continue;
                }
            }
            '?' => {
                // Exactly one char, but not a path separator.
                out.push_str("[^/]");
                i += 1;
                continue;
            }
            other => {
                push_escaped(&mut out, other);
                i += 1;
            }
        }
    }

    out.push('$');
    out
}

/// Append `c` to `out`, escaping any character that is significant to the
/// regex engine so it is matched literally.
fn push_escaped(out: &mut String, c: char) {
    // Characters that must be escaped to be treated literally by `regex`.
    const SPECIAL: &[char] = &[
        '\\', '.', '+', '(', ')', '|', '[', ']', '{', '}', '^', '$', '#', '&', '~',
    ];
    if SPECIAL.contains(&c) {
        out.push('\\');
        out.push(c);
    } else {
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_match() {
        assert!(glob_match("hello.txt", "hello.txt"));
        assert!(!glob_match("hello.txt", "hello_txt")); // '.' is literal, not wildcard
        assert!(!glob_match("hello.txt", "hello.txtx"));
        assert!(!glob_match("hello.txt", "xhello.txt"));
    }

    #[test]
    fn empty_pattern_matches_only_empty() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    #[test]
    fn single_star_within_segment() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(glob_match("*", "anything_no_slash"));
        // '*' must not cross a path separator.
        assert!(!glob_match("src/*.rs", "src/sub/main.rs"));
        assert!(!glob_match("*", "a/b"));
    }

    #[test]
    fn star_matches_zero_chars() {
        assert!(glob_match("foo*", "foo"));
        assert!(glob_match("*bar", "bar"));
        assert!(glob_match("a*b", "ab"));
    }

    #[test]
    fn globstar_crosses_segments() {
        assert!(glob_match("src/**", "src/a/b/c"));
        assert!(glob_match("src/**/main.rs", "src/main.rs"));
        assert!(glob_match("src/**/main.rs", "src/a/main.rs"));
        assert!(glob_match("src/**/main.rs", "src/a/b/c/main.rs"));
        assert!(glob_match("**", "literally/anything/at/all"));
        assert!(glob_match("**/*.rs", "a/b/c.rs"));
    }

    #[test]
    fn triple_star_behaves_like_globstar() {
        assert!(glob_match("a/***/b", "a/x/y/b"));
    }

    #[test]
    fn question_mark_one_char() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(glob_match("?", "z"));
        assert!(!glob_match("file?.txt", "file.txt")); // needs exactly one char
        assert!(!glob_match("file?.txt", "file12.txt")); // exactly one, not two
        assert!(!glob_match("?", "/")); // '?' does not match separator
    }

    #[test]
    fn regex_metachars_are_literal() {
        assert!(glob_match("a+b", "a+b"));
        assert!(!glob_match("a+b", "aaab"));
        assert!(glob_match("(group)", "(group)"));
        assert!(glob_match("price$", "price$"));
        assert!(glob_match("a[b]c", "a[b]c"));
        assert!(glob_match("a{b}c", "a{b}c"));
        assert!(glob_match("back\\slash", "back\\slash"));
    }

    #[test]
    fn command_style_globs() {
        // A single `*` stays within a segment, so it stops at the first '/'.
        assert!(glob_match("rm -rf*", "rm -rf"));
        assert!(!glob_match("rm -rf*", "rm -rf /")); // '*' will not cross '/'
                                                     // The globstar `**` crosses '/', which is what command-line rules use
                                                     // when the argument is a path.
        assert!(glob_match("rm -rf**", "rm -rf /"));
        assert!(glob_match("rm -rf**", "rm -rf /home/user"));
        assert!(glob_match("git *", "git status"));
        assert!(!glob_match("git *", "gitstatus"));
    }

    #[test]
    fn to_regex_is_anchored() {
        let re = glob_to_regex("abc");
        assert!(re.starts_with('^'));
        assert!(re.ends_with('$'));
    }
}
