use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser};
use tantivy::schema::{Schema, Value};
use tantivy::{Index, TantivyDocument};

use crate::highlight::{highlight, japanese_query_tokens};
use crate::meta::IndexMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Score,
    Path,
    Mtime,
}

impl SortOrder {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "score" => Some(Self::Score),
            "path" => Some(Self::Path),
            "mtime" => Some(Self::Mtime),
            _ => None,
        }
    }
}

// ---- Result types ----

#[derive(Debug, Serialize)]
pub struct LineResult {
    pub path: String,
    pub line_number: u64,
    pub lines: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FileResult {
    pub path: String,
    pub snippets: Vec<String>,
}

// ---- Query building ----

/// Map a user-facing logical field name to the (japanese_field, bigram_field) pair used
/// internally. Returns None when the name is not a recognised logical field (i.e. it should be
/// passed through unchanged).
fn logical_to_physical(logical: &str) -> Option<(&'static str, &'static str)> {
    match logical {
        "body" => Some(("body_japanese", "body_bigram")),
        "filename" => Some(("filename", "filename_bigram")),
        _ if logical.starts_with("frontmatter.") => {
            // "frontmatter.xxx" → ("frontmatter.xxx", "frontmatter_bigram.xxx")
            // Return None here; handled separately below.
            None
        }
        _ => None,
    }
}

/// Scan a query string for field qualifiers and return which logical target fields are present.
/// Shared by build_query (to pick parsers) and search_file (to pick snippet source).
fn scan_field_qualifiers(query_str: &str) -> (bool, bool) {
    let (mut has_fm, mut has_filename) = (false, false);
    let chars: Vec<char> = query_str.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i].is_whitespace() { i += 1; continue; }
        if chars[i] == '+' || chars[i] == '-' { i += 1; }
        let start = i;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
            i += 1;
        }
        if i < len && chars[i] == ':' && i > start {
            let field: String = chars[start..i].iter().collect();
            if field.starts_with("frontmatter.") { has_fm = true; }
            if field == "filename" { has_filename = true; }
        }
        while i < len && !chars[i].is_whitespace() { i += 1; }
    }
    (has_fm, has_filename)
}

fn query_targets_frontmatter(query_str: &str) -> bool {
    scan_field_qualifiers(query_str).0
}

fn query_targets_filename(query_str: &str) -> bool {
    scan_field_qualifiers(query_str).1
}

/// Format the stored frontmatter JSON object as a readable key:value snippet.
/// The key matching the query's `frontmatter.<key>:` qualifier is shown first.
fn frontmatter_to_snippet(
    doc: &tantivy::TantivyDocument,
    fm_field: tantivy::schema::Field,
    query_str: &str,
) -> String {
    use tantivy::schema::Value;

    // Extract the target key from the first "frontmatter.<key>:" qualifier in the query.
    let target_key: Option<String> = {
        let chars: Vec<char> = query_str.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut found = None;
        while i < len && found.is_none() {
            if chars[i].is_whitespace() { i += 1; continue; }
            if chars[i] == '+' || chars[i] == '-' { i += 1; }
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            if i < len && chars[i] == ':' && i > start {
                let field: String = chars[start..i].iter().collect();
                if let Some(key) = field.strip_prefix("frontmatter.") {
                    found = Some(key.to_string());
                }
            }
            while i < len && !chars[i].is_whitespace() { i += 1; }
        }
        found
    };

    let fm_val = match doc.get_first(fm_field) {
        Some(v) => v,
        None => return String::new(),
    };

    // Collect all kv pairs from the object iterator.
    let pairs: Vec<(String, String)> = fm_val
        .as_object()
        .map(|iter| {
            iter.map(|(k, v)| {
                let val_str = v.as_str().unwrap_or("").to_string();
                (k.to_string(), val_str)
            })
            .collect()
        })
        .unwrap_or_default();

    // Target key first, then the rest.
    let mut parts: Vec<String> = Vec::new();
    if let Some(ref key) = target_key {
        if let Some((_, val)) = pairs.iter().find(|(k, _)| k == key) {
            parts.push(format!("{}:{}", key, val));
        }
    }
    for (k, v) in &pairs {
        if target_key.as_deref() == Some(k.as_str()) { continue; }
        parts.push(format!("{}:{}", k, v));
    }
    parts.join("  ")
}

/// Rewrite the user-visible query string into a pair of tantivy query strings — one for the
/// japanese analyzer fields and one for the bigram fields — so that field qualifiers like
/// `body:foo` or `frontmatter.author:bar` are expanded to the correct physical field names.
///
/// Field names that are not recognised logical names are passed through unchanged so that
/// ordinary tantivy field qualifiers (`body_japanese:foo`) still work.
fn rewrite_query(query_str: &str) -> (String, String) {
    // Regex: match "word:" at the start of a token, including dotted paths like
    // "frontmatter.author:". The colon must NOT be escaped (i.e. not preceded by \).
    // We use a simple hand-rolled scan to avoid a regex dependency.
    let mut japanese_out = String::with_capacity(query_str.len() + 16);
    let mut bigram_out = String::with_capacity(query_str.len() + 32);

    let chars: Vec<char> = query_str.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace — copy verbatim to both outputs.
        if chars[i].is_whitespace() {
            japanese_out.push(chars[i]);
            bigram_out.push(chars[i]);
            i += 1;
            continue;
        }

        // Detect leading "+" / "-" / "+" operators.
        let mut prefix = String::new();
        if chars[i] == '+' || chars[i] == '-' {
            prefix.push(chars[i]);
            i += 1;
        }

        // Collect a potential field name (alphanumeric, underscore, dot).
        let field_start = i;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
            i += 1;
        }
        let field_candidate: String = chars[field_start..i].iter().collect();

        // Check whether it is followed by ':'.
        if i < len && chars[i] == ':' && !field_candidate.is_empty() {
            i += 1; // consume ':'

            // Determine physical field names.
            let (j_field, b_field) = if let Some((jf, bf)) = logical_to_physical(&field_candidate)
            {
                (jf.to_string(), bf.to_string())
            } else if field_candidate.starts_with("frontmatter.") {
                // e.g. "frontmatter.author" → bigram counterpart is "frontmatter_bigram.author"
                let suffix = &field_candidate["frontmatter.".len()..];
                (
                    field_candidate.clone(),
                    format!("frontmatter_bigram.{}", suffix),
                )
            } else {
                // Unknown field — pass through unchanged to both.
                let same = field_candidate.clone();
                (same.clone(), same)
            };

            japanese_out.push_str(&prefix);
            japanese_out.push_str(&j_field);
            japanese_out.push(':');

            bigram_out.push_str(&prefix);
            bigram_out.push_str(&b_field);
            bigram_out.push(':');
        } else {
            // Not a field qualifier — copy the token verbatim.
            japanese_out.push_str(&prefix);
            japanese_out.push_str(&field_candidate);
            bigram_out.push_str(&prefix);
            bigram_out.push_str(&field_candidate);
        }
    }

    (japanese_out, bigram_out)
}

fn build_query(
    index: &Index,
    schema: &Schema,
    query_str: &str,
    mode: IndexMode,
    frontmatter_keys: &std::collections::HashSet<String>,
) -> Result<Box<dyn Query>> {
    let body_japanese = schema.get_field("body_japanese").unwrap();
    let body_bigram = schema.get_field("body_bigram").unwrap();

    if mode == IndexMode::File {
        let filename_field = schema.get_field("filename").unwrap();
        let filename_bigram_field = schema.get_field("filename_bigram").unwrap();
        let frontmatter_field = schema.get_field("frontmatter").unwrap();
        let frontmatter_bigram_field = schema.get_field("frontmatter_bigram").unwrap();

        // When the query contains a field qualifier ("field:value"), route it only through the
        // frontmatter parser so the qualifier is honoured exactly. Mixing multiple parsers that
        // each cover different fields would cause the qualified term to be silently dropped by
        // parsers that don't know that field, or matched against the wrong field.
        let has_field_qualifier = query_str.contains(':');

        if has_field_qualifier {
            // Classify the field qualifier to decide which parsers to use.
            // frontmatter.* values are indexed with bigram only — the japanese morphological
            // analyser treats ASCII-only values (e.g. "notebook", "report") as a single
            // catch-all token "*", which would match every document regardless of value.
            // body: / filename: qualifiers still benefit from both analysers.
            let targets_frontmatter = query_targets_frontmatter(query_str);

            let (j_query, b_query) = rewrite_query(query_str);

            // bigram parser always handles the rewritten b_query.
            let bigram_parser = QueryParser::for_index(
                index,
                vec![body_bigram, filename_bigram_field, frontmatter_bigram_field],
            );
            let bigram_q = bigram_parser
                .parse_query(&b_query)
                .map_err(|e| anyhow::anyhow!("Query parse error (bigram): {}", e))?;

            if targets_frontmatter {
                // For frontmatter field qualifiers, skip the japanese parser entirely:
                // its morphological analyser mangles ASCII values into "*", causing false
                // positives across all documents.
                return Ok(bigram_q);
            }

            // For body:/filename: qualifiers, also run the japanese parser.
            let japanese_parser = QueryParser::for_index(
                index,
                vec![body_japanese, filename_field],
            );
            let japanese_q = japanese_parser
                .parse_query(&j_query)
                .map_err(|e| anyhow::anyhow!("Query parse error (japanese): {}", e))?;
            let combined: Box<dyn Query> = Box::new(BooleanQuery::new(vec![
                (Occur::Should, japanese_q),
                (Occur::Should, bigram_q),
            ]));
            return Ok(combined);
        }

        // Plain query: search body, filename, and every known frontmatter sub-key.
        // Japanese and bigram parsers are combined as Should so both analyzers apply.
        let japanese_parser = QueryParser::for_index(index, vec![body_japanese, filename_field]);
        let bigram_parser =
            QueryParser::for_index(index, vec![body_bigram, filename_bigram_field]);
        let japanese_q = japanese_parser
            .parse_query(query_str)
            .map_err(|e| anyhow::anyhow!("Query parse error (japanese): {}", e))?;
        let bigram_q = bigram_parser
            .parse_query(query_str)
            .map_err(|e| anyhow::anyhow!("Query parse error (bigram): {}", e))?;

        // For each known frontmatter key, build a field-qualified query "frontmatter.<key>:<query>"
        // so that tantivy generates terms with the correct json_path. Plain queries against a JSON
        // field always use json_path="" and never match sub-key terms, so we must qualify explicitly.
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
            (Occur::Should, japanese_q),
            (Occur::Should, bigram_q),
        ];
        let fm_japanese_parser = QueryParser::for_index(index, vec![frontmatter_field]);
        let fm_bigram_parser = QueryParser::for_index(index, vec![frontmatter_bigram_field]);
        for key in frontmatter_keys {
            let qualified_j = format!("frontmatter.{}:({})", key, query_str);
            let qualified_b = format!("frontmatter_bigram.{}:({})", key, query_str);
            if let Ok(q) = fm_japanese_parser.parse_query(&qualified_j) {
                clauses.push((Occur::Should, q));
            }
            if let Ok(q) = fm_bigram_parser.parse_query(&qualified_b) {
                clauses.push((Occur::Should, q));
            }
        }

        let combined: Box<dyn Query> = Box::new(BooleanQuery::new(clauses));
        return Ok(combined);
    }

    let japanese_parser = QueryParser::for_index(index, vec![body_japanese]);
    let bigram_parser = QueryParser::for_index(index, vec![body_bigram]);
    // Use lenient parsing so that field qualifiers referencing fields that don't exist in the
    // line-mode schema (e.g. "filename:", "frontmatter.*:") are silently ignored rather than
    // causing a hard error. The user may reasonably type such qualifiers without --by-file.
    let (japanese_q, _) = japanese_parser.parse_query_lenient(query_str);
    let (bigram_q, _) = bigram_parser.parse_query_lenient(query_str);
    let combined: Box<dyn Query> = Box::new(BooleanQuery::new(vec![
        (Occur::Should, japanese_q),
        (Occur::Should, bigram_q),
    ]));
    Ok(combined)
}

/// Build the Japanese analyzer token set for a query (used for base-form highlight matching).
fn build_highlight_tokens(index: &Index, query_str: &str) -> HashSet<String> {
    let manager = index.tokenizers();
    let mut japanese = match manager.get(crate::analyzer::TOKENIZER_JAPANESE) {
        Some(a) => a,
        None => return HashSet::new(),
    };
    japanese_query_tokens(&mut japanese, query_str)
}

// ---- Line-mode search ----

pub fn search_line(
    index: &Index,
    query_str: &str,
    limit: usize,
    _sort: SortOrder,
) -> Result<Vec<LineResult>> {
    let schema = index.schema();
    let reader = index
        .reader()
        .map_err(|e| anyhow::anyhow!("Failed to open reader: {}", e))?;
    let searcher = reader.searcher();
    let query = build_query(index, &schema, query_str, IndexMode::Line, &Default::default())?;

    let top_docs = searcher
        .search(&query, &TopDocs::with_limit(limit))
        .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

    let path_field = schema.get_field("path").unwrap();
    let line_num_field = schema.get_field("line_number").unwrap();
    let body_japanese = schema.get_field("body_japanese").unwrap();

    let mut results = Vec::new();
    for (_score, addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(addr)?;
        let path = doc
            .get_first(path_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let line_number = doc
            .get_first(line_num_field)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let line = doc
            .get_first(body_japanese)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        results.push(LineResult {
            path,
            line_number,
            lines: vec![line],
        });
    }

    Ok(results)
}

// ---- File-mode search ----

pub fn search_file(
    index: &Index,
    query_str: &str,
    limit: usize,
    _sort: SortOrder,
    frontmatter_keys: &std::collections::HashSet<String>,
) -> Result<Vec<FileResult>> {
    let schema = index.schema();
    let reader = index
        .reader()
        .map_err(|e| anyhow::anyhow!("Failed to open reader: {}", e))?;
    let searcher = reader.searcher();
    let query = build_query(index, &schema, query_str, IndexMode::File, frontmatter_keys)?;

    let top_docs = searcher
        .search(&query, &TopDocs::with_limit(limit))
        .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

    let path_field = schema.get_field("path").unwrap();
    let body_japanese = schema.get_field("body_japanese").unwrap();
    let filename_field = schema.get_field("filename").unwrap();
    let frontmatter_field = schema.get_field("frontmatter").unwrap();

    // Determine what the query is targeting so we can choose the right snippet source.
    let has_qualifier = query_str.contains(':');
    let targets_fm = has_qualifier && query_targets_frontmatter(query_str);
    let targets_filename = has_qualifier && query_targets_filename(query_str);

    let mut results = Vec::new();
    for (_score, addr) in top_docs {
        let doc: TantivyDocument = searcher.doc(addr)?;
        let path = doc
            .get_first(path_field)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let snippet = if targets_fm {
            // Show frontmatter values as the snippet so the user can see why this file matched.
            let fm_str = frontmatter_to_snippet(&doc, frontmatter_field, query_str);
            if fm_str.is_empty() {
                doc.get_first(body_japanese)
                    .and_then(|v| v.as_str())
                    .map(|s| extract_snippet(s, query_str))
                    .unwrap_or_default()
            } else {
                fm_str
            }
        } else if targets_filename {
            // Show the filename as the snippet.
            doc.get_first(filename_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            doc.get_first(body_japanese)
                .and_then(|v| v.as_str())
                .map(|s| extract_snippet(s, query_str))
                .unwrap_or_default()
        };

        results.push(FileResult {
            path,
            snippets: vec![snippet],
        });
    }

    Ok(results)
}

// ---- Snippet extraction ----

/// Extract a window of text around the first match of `query_str` in `body`.
/// Uses the same normalization as the search index (NFKC, lowercase, kata→hira, long-vowel
/// removal), so e.g. "もでる" correctly locates "モデル" in the body.
/// Falls back to the beginning of the text if no match is found.
fn extract_snippet(body: &str, query_str: &str) -> String {
    const WINDOW_CHARS: usize = 120;
    const CONTEXT_CHARS: usize = 40;

    let raw = crate::highlight::strip_query_syntax(query_str);
    let raw_norm = crate::analyzer::normalize(raw);

    // Build alignment: for each char in the normalized body, record which original char
    // index it came from. This mirrors the approach in highlight::highlight().
    let mut norm_to_orig: Vec<usize> = Vec::new();
    for (orig_idx, orig_char) in body.chars().enumerate() {
        let frag = crate::analyzer::normalize(&orig_char.to_string());
        for _ in frag.chars() {
            norm_to_orig.push(orig_idx);
        }
    }
    let norm_body: String = body
        .chars()
        .flat_map(|c| crate::analyzer::normalize(&c.to_string()).chars().collect::<Vec<_>>())
        .collect();

    // Find the first match in the normalized body and map back to an orig char index.
    let match_orig_char = if !raw_norm.is_empty() {
        norm_body.find(raw_norm.as_str()).and_then(|byte_pos| {
            let norm_char_idx = norm_body[..byte_pos].chars().count();
            norm_to_orig.get(norm_char_idx).copied()
        })
    } else {
        None
    };

    // Collect orig char → byte offset table (with sentinel).
    let char_to_byte: Vec<usize> = body
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(body.len()))
        .collect();

    let start_char = match match_orig_char {
        Some(c) => c.saturating_sub(CONTEXT_CHARS),
        None => 0,
    };

    let chars: Vec<(usize, char)> = body
        .char_indices()
        .skip(start_char)
        .take(WINDOW_CHARS + 1)
        .collect();
    if chars.is_empty() {
        return body.to_string();
    }

    let truncated = chars.len() > WINDOW_CHARS;
    let start_byte = char_to_byte[start_char];
    let end_byte = if truncated { chars[WINDOW_CHARS].0 } else { body.len() };

    let mut s = String::new();
    if start_char > 0 {
        s.push_str("…");
    }
    s.push_str(&body[start_byte..end_byte]);
    if truncated {
        s.push_str("…");
    }
    s
}

// ---- Output formatting ----

pub fn print_line_results(index: &Index, results: &[LineResult], query_str: &str, json: bool) -> bool {
    if json {
        println!("{}", serde_json::to_string(results).unwrap_or_default());
    } else {
        let tokens = build_highlight_tokens(index, query_str);
        let manager = index.tokenizers();
        let mut japanese = manager.get(crate::analyzer::TOKENIZER_JAPANESE);
        for r in results {
            for line in &r.lines {
                let hl = match japanese.as_mut() {
                    Some(m) => highlight(line, &tokens, query_str, m),
                    None => line.clone(),
                };
                println!("{}:{}:{}", r.path, r.line_number, hl);
            }
        }
    }
    !results.is_empty()
}

pub fn print_file_results(index: &Index, results: &[FileResult], query_str: &str, json: bool) -> bool {
    if json {
        println!("{}", serde_json::to_string(results).unwrap_or_default());
    } else {
        let tokens = build_highlight_tokens(index, query_str);
        let manager = index.tokenizers();
        let mut japanese = manager.get(crate::analyzer::TOKENIZER_JAPANESE);
        for r in results {
            for snippet in &r.snippets {
                let hl = match japanese.as_mut() {
                    Some(m) => highlight(snippet, &tokens, query_str, m),
                    None => snippet.clone(),
                };
                println!("{}:{}", r.path, hl);
            }
        }
    }
    !results.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{build_ram_index, build_ram_index_stdin, tests::fixtures_dir};
    use crate::meta::IndexMode;

    #[test]
    fn test_search_line_english() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("english_doc.txt")];
        let (index, _) = build_ram_index(&files, IndexMode::Line).unwrap();
        let results = search_line(&index, "search", 10, SortOrder::Score).unwrap();
        assert!(!results.is_empty(), "expected results for 'search'");
        assert!(results.iter().any(|r| r.lines.iter().any(|l| l.to_lowercase().contains("search"))));
    }

    #[test]
    fn test_search_line_japanese() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("search_engine.txt")];
        let (index, _) = build_ram_index(&files, IndexMode::Line).unwrap();
        let results = search_line(&index, "検索", 10, SortOrder::Score).unwrap();
        assert!(!results.is_empty(), "expected results for '検索'");
    }

    #[test]
    fn test_search_line_no_results() {
        let content = "This is a test line with known words.";
        let (index, _) = build_ram_index_stdin(content, IndexMode::Line).unwrap();
        let results = search_line(&index, "絶対ヒットしない", 10, SortOrder::Score).unwrap();
        assert!(results.is_empty(), "expected no results for Japanese query against ASCII doc");
    }

    #[test]
    fn test_search_file_mode() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("rust_intro.md")];
        let (index, fm_keys) = build_ram_index(&files, IndexMode::File).unwrap();
        let results = search_file(&index, "Rust", 10, SortOrder::Score, &fm_keys).unwrap();
        assert!(!results.is_empty(), "expected results for 'Rust' in file mode");
    }

    #[test]
    fn test_search_line_base_form() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("notes.md")];
        let (index, _) = build_ram_index(&files, IndexMode::Line).unwrap();
        let results = search_line(&index, "走る", 10, SortOrder::Score).unwrap();
        assert!(!results.is_empty(), "expected results for '走る' (base form search)");
    }

    #[test]
    fn test_search_stdin_line_mode() {
        let content = "Rustはシステムプログラミング言語です。\nメモリ安全性を保証します。";
        let (index, _) = build_ram_index_stdin(content, IndexMode::Line).unwrap();
        let results = search_line(&index, "メモリ", 10, SortOrder::Score).unwrap();
        assert!(!results.is_empty(), "expected results from stdin");
        assert_eq!(results[0].path, "(standard input)");
    }

    #[test]
    fn test_sort_order_parse() {
        assert_eq!(SortOrder::parse("score"), Some(SortOrder::Score));
        assert_eq!(SortOrder::parse("path"), Some(SortOrder::Path));
        assert_eq!(SortOrder::parse("mtime"), Some(SortOrder::Mtime));
        assert_eq!(SortOrder::parse("invalid"), None);
    }
}
