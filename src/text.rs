//! What happens to the text before it is spoken: cleaning, the style cue, and cutting
//! into sentence-sized pieces. Each function follows the one of the same name in the
//! earlier Python front; `tests/fixtures/text_cases.json` holds that front's own answers
//! and the tests below compare against them.

use std::sync::LazyLock;

use regex::Regex;

/// Longest text accepted after cleaning, in characters.
pub const MAX_INPUT: usize = 4096;
/// Longest piece sent to the model, in characters. Longer requests drift.
pub const MAX_CHARS: usize = 180;

static CODE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());
static MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*_`#>|~]+").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static INLINE_CUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^\(([^()\n]{1,300})\)\s*(.+)$").unwrap());

/// Drops code blocks and markup, keeps link labels, and collapses whitespace.
pub fn clean_text(text: &str) -> String {
    let text = CODE_BLOCK.replace_all(text, " ");
    let text = LINK.replace_all(&text, "${1}");
    let text = MARKUP.replace_all(&text, " ");
    SPACE.replace_all(&text, " ").trim().to_string()
}

/// Separates a style cue from the text. Returns `(text, cue)`.
///
/// The cue comes from `instructions`, else from round brackets at the start of the
/// text; those brackets are always cut off. The Magpie adapter has no use for the cue,
/// but the checks are part of the API.
pub fn speech_control(text: &str, instructions: Option<&str>) -> Result<(String, String), &'static str> {
    let mut control = SPACE.replace_all(instructions.unwrap_or(""), " ").trim().to_string();
    let mut text = text.to_string();
    if let Some(m) = INLINE_CUE.captures(&text) {
        let cue = m[1].trim().to_string();
        let rest = m[2].trim().to_string();
        text = rest;
        if control.is_empty() {
            control = cue;
        }
    }
    if control.chars().count() > 300 || control.contains('(') || control.contains(')') {
        return Err("instructions must be at most 300 chars without parentheses");
    }
    Ok((text, control))
}

/// Splits after `.`, `!` or `?` followed by whitespace.
fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() && i > 0 && matches!(chars[i - 1], '.' | '!' | '?') {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(chars[i]);
        i += 1;
    }
    out.push(cur);
    out
}

/// Packs sentences into pieces of at most [`MAX_CHARS`] characters; a piece that is
/// still too long is cut at the last space before the limit.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for s in sentences(text) {
        if cur.chars().count() + s.chars().count() + 1 > MAX_CHARS && !cur.is_empty() {
            chunks.push(std::mem::replace(&mut cur, s));
        } else {
            cur = format!("{cur} {s}").trim().to_string();
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }

    let mut out = Vec::new();
    for chunk in chunks {
        let mut c: Vec<char> = chunk.chars().collect();
        while c.len() > MAX_CHARS {
            let cut = c[..MAX_CHARS].iter().rposition(|&ch| ch == ' ').filter(|&i| i > 0).unwrap_or(MAX_CHARS);
            out.push(c[..cut].iter().collect());
            c = c[cut..].iter().collect::<String>().trim().chars().collect();
        }
        if !c.is_empty() {
            out.push(c.into_iter().collect());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// Every case in the fixture was answered by the Python front's own functions.
    #[test]
    fn same_answers_as_the_python_front() {
        let cases: Vec<Value> = serde_json::from_str(include_str!("../tests/fixtures/text_cases.json")).unwrap();
        assert!(cases.len() >= 20, "fixture looks truncated");
        for case in &cases {
            let input = case["input"].as_str().unwrap();
            let instructions = case["instructions"].as_str();
            let cleaned = clean_text(input);
            assert_eq!(cleaned, case["clean"].as_str().unwrap(), "clean_text({input:?})");
            match speech_control(&cleaned, instructions) {
                Ok((text, control)) => {
                    assert_eq!(case["error"], Value::Null, "speech_control({cleaned:?}) should fail");
                    assert_eq!(text, case["text"].as_str().unwrap(), "text of {input:?}");
                    assert_eq!(control, case["control"].as_str().unwrap(), "cue of {input:?}");
                    let want: Vec<&str> = case["chunks"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
                    assert_eq!(chunk_text(&text), want, "chunk_text({text:?})");
                }
                Err(e) => assert_eq!(e, case["error"].as_str().unwrap(), "error of {input:?}"),
            }
        }
    }

    #[test]
    fn no_piece_is_longer_than_the_limit() {
        let text = "word ".repeat(400) + &"x".repeat(500);
        let chunks = chunk_text(text.trim());
        assert!(chunks.len() > 10);
        assert!(chunks.iter().all(|c| c.chars().count() <= MAX_CHARS));
    }
}
