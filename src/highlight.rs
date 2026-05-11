use std::collections::HashSet;

use tantivy::tokenizer::{TextAnalyzer, TokenStream};

use crate::analyzer::normalize;

const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_RESET: &str = "\x1b[0m";

fn extract_tokens_with_offsets(
    analyzer: &mut TextAnalyzer,
    text: &str,
) -> Vec<(String, usize, usize)> {
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while stream.advance() {
        let t = stream.token();
        tokens.push((t.text.clone(), t.offset_from, t.offset_to));
    }
    tokens
}

fn extract_tokens(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while stream.advance() {
        tokens.push(stream.token().text.clone());
    }
    tokens
}

/// Normalized Japanese analyzer tokens of the query (used for base-form matching against the display line).
/// Unknown-word tokens ("*") are excluded — they match too broadly.
pub fn japanese_query_tokens(analyzer: &mut TextAnalyzer, query_str: &str) -> HashSet<String> {
    extract_tokens(analyzer, query_str)
        .into_iter()
        .filter(|t| t != "*")
        .collect()
}

/// Highlight matching regions in `line`.
///
/// Two independent passes, unioned:
/// 1. Japanese pass: tokenize `line` with the Japanese analyzer; highlight any token whose
///    normalized (base) form is in `japanese_tokens`. This handles 走る → 走っていた.
/// 2. Substring pass: search for the raw `query_str` (lowercased) directly in `line`.
///    This handles ASCII keywords, symbols, and any query that hits only via bigram.
pub fn highlight(
    line: &str,
    japanese_tokens: &HashSet<String>,
    query_str: &str,
    japanese: &mut TextAnalyzer,
) -> String {
    if !is_tty() {
        return line.to_string();
    }

    let mut regions: Vec<(usize, usize)> = Vec::new();

    // --- Pass 1: Japanese base-form match ---
    if !japanese_tokens.is_empty() {
        for (norm_text, start, end) in extract_tokens_with_offsets(japanese, line) {
            if japanese_tokens.contains(&norm_text)
                && line.is_char_boundary(start)
                && line.is_char_boundary(end)
            {
                regions.push((start, end));
            }
        }
    }

    // --- Pass 2: normalized substring match ---
    // Both the query and the line are normalized (NFKC, lowercase, kata→hira, long-vowel
    // removal) before comparison, so e.g. "もでる" matches "モデル".
    // After finding a match in the normalized string, we map the char-level indices back
    // to byte offsets in the original `line` via a precomputed table.
    let raw = strip_query_syntax(query_str);
    let raw_norm = normalize(raw);
    // Build a table: char_index → byte_offset in original `line`, plus a sentinel at the end.
    let char_to_byte: Vec<usize> = line
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(line.len()))
        .collect();
    // Build a parallel normalized char sequence from the original line.
    // Each original char maps to zero or more normalized chars; we track which original
    // char each normalized char came from via an alignment table.
    let mut norm_chars: Vec<char> = Vec::new();
    let mut norm_to_orig: Vec<usize> = Vec::new(); // norm char index → orig char index
    for (orig_idx, orig_char) in line.chars().enumerate() {
        let norm_fragment: String = normalize(&orig_char.to_string());
        for nc in norm_fragment.chars() {
            norm_chars.push(nc);
            norm_to_orig.push(orig_idx);
        }
    }
    let norm_line: String = norm_chars.iter().collect();

    if !raw_norm.is_empty() {
        let mut search_from = 0; // byte offset in norm_line
        while search_from < norm_line.len() {
            if let Some(pos) = norm_line[search_from..].find(raw_norm.as_str()) {
                let match_byte_start = search_from + pos;
                let match_byte_end = match_byte_start + raw_norm.len();

                // Convert norm byte offsets → norm char indices.
                let norm_char_start = norm_line[..match_byte_start].chars().count();
                let norm_char_end = norm_line[..match_byte_end].chars().count();

                // Map norm char indices → orig char indices.
                if let (Some(&orig_start), Some(&orig_end_inclusive)) = (
                    norm_to_orig.get(norm_char_start),
                    norm_to_orig.get(norm_char_end.saturating_sub(1)),
                ) {
                    let byte_start = char_to_byte[orig_start];
                    // The region covers through the end of orig_end_inclusive char.
                    let byte_end = char_to_byte[orig_end_inclusive + 1];
                    if line.is_char_boundary(byte_start) && line.is_char_boundary(byte_end) {
                        regions.push((byte_start, byte_end));
                    }
                }

                // Advance by one norm char to avoid infinite loop.
                let step = norm_line[match_byte_start..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                search_from = match_byte_start + step;
            } else {
                break;
            }
        }
    }

    if regions.is_empty() {
        return line.to_string();
    }

    // Sort and merge overlapping regions.
    regions.sort_unstable_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in regions {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    let mut out = String::with_capacity(line.len() + merged.len() * 16);
    let mut cursor = 0;
    for (s, e) in merged {
        out.push_str(&line[cursor..s]);
        out.push_str(ANSI_BOLD_RED);
        out.push_str(&line[s..e]);
        out.push_str(ANSI_RESET);
        cursor = e;
    }
    out.push_str(&line[cursor..]);
    out
}

/// Remove tantivy query syntax characters to get a plain search term for substring matching.
/// For field-qualified terms like `frontmatter.author:foo` or `filename:bar`, returns only
/// the value part (`foo` / `bar`) so it can be matched against the rendered snippet text.
pub fn strip_query_syntax(q: &str) -> &str {
    let q = q.trim_start_matches(['+', '-']);
    // If the token contains a colon (field qualifier), keep only the value after it.
    let q = if let Some(pos) = q.find(':') { &q[pos + 1..] } else { q };
    let q = q.trim_matches('"');
    q.trim()
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    if std::env::var("FORCE_COLOR").is_ok() {
        return true;
    }
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    std::io::stdout().is_terminal()
}
