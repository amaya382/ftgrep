use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tantivy::doc;
use tantivy::schema::OwnedValue;
use tantivy::{Index, IndexWriter, TantivyDocument};
use walkdir::WalkDir;

use crate::analyzer::register_analyzers;
use crate::meta::{FileRecord, IndexMeta, IndexMode};
use crate::schema::{file_schema, line_schema};

const WRITER_HEAP: usize = 50_000_000;
const BINARY_PROBE: usize = 8192;

/// Open or create a tantivy index at `index_dir` for the given mode.
pub fn open_or_create_index(index_dir: &Path, mode: IndexMode) -> Result<Index> {
    std::fs::create_dir_all(index_dir)
        .with_context(|| format!("Failed to create index dir {}", index_dir.display()))?;

    let schema = match mode {
        IndexMode::Line => line_schema(),
        IndexMode::File => file_schema(),
    };

    let index = if index_dir.join("meta.json").exists() {
        let existing = Index::open_in_dir(index_dir)
            .with_context(|| format!("Failed to open index at {}", index_dir.display()))?;
        // If the on-disk schema doesn't match the current schema (e.g. new fields were added),
        // wipe the index directory and recreate it so the caller can re-sync from scratch.
        if existing.schema() != schema {
            drop(existing);
            for entry in std::fs::read_dir(index_dir)? {
                let entry = entry?;
                let p = entry.path();
                // Preserve ftgrep_meta.json so directory/file tracking survives the rebuild.
                if p.file_name().and_then(|n| n.to_str()) == Some("ftgrep_meta.json") {
                    continue;
                }
                if p.is_dir() {
                    std::fs::remove_dir_all(&p)?;
                } else {
                    std::fs::remove_file(&p)?;
                }
            }
            Index::create_in_dir(index_dir, schema)
                .with_context(|| format!("Failed to recreate index at {}", index_dir.display()))?
        } else {
            existing
        }
    } else {
        Index::create_in_dir(index_dir, schema)
            .with_context(|| format!("Failed to create index at {}", index_dir.display()))?
    };

    register_analyzers(&index)?;
    Ok(index)
}

fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; BINARY_PROBE];
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0)
}

fn file_mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

fn matches_patterns(
    path: &Path,
    include: Option<&str>,
    exclude: Option<&str>,
) -> bool {
    let name = path.to_string_lossy();
    if let Some(pat) = include {
        if !glob_match(pat, &name) {
            return false;
        }
    }
    if let Some(pat) = exclude {
        if glob_match(pat, &name) {
            return false;
        }
    }
    true
}

/// Minimal glob matching: supports `*` (any chars, no path sep) and `**` (any).
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_parts(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_parts(pat: &[u8], text: &[u8]) -> bool {
    match (pat, text) {
        ([], []) => true,
        ([], _) => false,
        ([b'*', b'*', rest @ ..], t) => {
            if glob_match_parts(rest, t) {
                return true;
            }
            for i in 0..=t.len() {
                if glob_match_parts(rest, &t[i..]) {
                    return true;
                }
            }
            false
        }
        ([b'*', rest @ ..], t) => {
            for i in 0..=t.len() {
                if t[..i].contains(&b'/') {
                    break;
                }
                if glob_match_parts(rest, &t[i..]) {
                    return true;
                }
            }
            false
        }
        ([p, rest_p @ ..], [t, rest_t @ ..]) => *p == *t && glob_match_parts(rest_p, rest_t),
        _ => false,
    }
}

/// Collect all text files under dirs respecting include/exclude patterns.
/// Uses rayon for parallel directory traversal.
pub fn collect_files(
    dirs: &[PathBuf],
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<PathBuf> {
    dirs.par_iter()
        .flat_map(|dir| {
            WalkDir::new(dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
                .filter(|p| matches_patterns(p, include, exclude))
                .filter(|p| !is_binary(p))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- Line-mode indexing ----

fn index_file_line(
    writer: &IndexWriter,
    schema: &tantivy::schema::Schema,
    path: &Path,
) -> Result<usize> {
    let content = std::fs::read_to_string(path)?;
    let path_field = schema.get_field("path").unwrap();
    let line_num_field = schema.get_field("line_number").unwrap();
    let body_japanese = schema.get_field("body_japanese").unwrap();
    let body_bigram = schema.get_field("body_bigram").unwrap();

    let path_str = path.to_string_lossy().to_string();
    let mut count = 0usize;

    // Remove frontmatter for line mode
    let body = strip_frontmatter(&content);

    for (i, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ln = (i + 1) as u64;
        writer.add_document(doc!(
            path_field => path_str.clone(),
            line_num_field => ln,
            body_japanese => line.to_string(),
            body_bigram => line.to_string(),
        ))?;
        count += 1;
    }
    Ok(count)
}

fn strip_frontmatter(content: &str) -> &str {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("\n---") {
            let after = end + 3 + 4; // skip past closing ---
            return content.get(after..).unwrap_or(content);
        }
    }
    content
}

// ---- File-mode indexing ----

fn parse_frontmatter(content: &str) -> (serde_json::Value, &str) {
    if content.starts_with("---") {
        if let Some(end_off) = content[3..].find("\n---") {
            let fm_yaml = &content[3..end_off + 3];
            let rest_start = end_off + 3 + 4;
            let body = content.get(rest_start..).unwrap_or("");
            if let Ok(val) = serde_yaml_to_json(fm_yaml) {
                return (val, body);
            }
        }
    }
    (serde_json::Value::Object(serde_json::Map::new()), content)
}

fn serde_yaml_to_json(yaml: &str) -> Result<serde_json::Value> {
    // Parse YAML frontmatter using gray_matter's built-in YAML logic.
    // We do it manually with a simple line-by-line key:value parser
    // to avoid adding a yaml dependency.
    let mut map = serde_json::Map::new();
    for line in yaml.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if !key.is_empty() {
                map.insert(key, serde_json::Value::String(val));
            }
        }
    }
    Ok(serde_json::Value::Object(map))
}

/// Convert a serde_json::Value object to BTreeMap<String, OwnedValue> for tantivy.
fn json_to_btree(val: &serde_json::Value) -> BTreeMap<String, OwnedValue> {
    match val {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| (k.clone(), json_val_to_owned(v)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn json_val_to_owned(v: &serde_json::Value) -> OwnedValue {
    match v {
        serde_json::Value::String(s) => OwnedValue::from(s.as_str()),
        serde_json::Value::Bool(b) => OwnedValue::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                OwnedValue::from(i)
            } else if let Some(f) = n.as_f64() {
                OwnedValue::from(f)
            } else {
                OwnedValue::from(n.to_string().as_str())
            }
        }
        serde_json::Value::Null => OwnedValue::Null,
        serde_json::Value::Array(arr) => {
            OwnedValue::Array(arr.iter().map(json_val_to_owned).collect())
        }
        serde_json::Value::Object(map) => OwnedValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_val_to_owned(v)))
                .collect(),
        ),
    }
}

fn index_file_file(
    writer: &IndexWriter,
    schema: &tantivy::schema::Schema,
    path: &Path,
    seen_keys: &mut HashSet<String>,
) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let path_field = schema.get_field("path").unwrap();
    let filename_field = schema.get_field("filename").unwrap();
    let filename_bigram_field = schema.get_field("filename_bigram").unwrap();
    let body_japanese = schema.get_field("body_japanese").unwrap();
    let body_bigram = schema.get_field("body_bigram").unwrap();
    let frontmatter_field = schema.get_field("frontmatter").unwrap();
    let frontmatter_bigram_field = schema.get_field("frontmatter_bigram").unwrap();

    let path_str = path.to_string_lossy().to_string();
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let (fm, body) = parse_frontmatter(&content);
    if let Some(obj) = fm.as_object() {
        for k in obj.keys() {
            seen_keys.insert(k.clone());
        }
    }

    let fm_btree = json_to_btree(&fm);
    let mut doc = TantivyDocument::default();
    doc.add_text(path_field, &path_str);
    doc.add_text(filename_field, &filename);
    doc.add_text(filename_bigram_field, &filename);
    doc.add_text(body_japanese, body);
    doc.add_text(body_bigram, body);
    doc.add_object(frontmatter_field, fm_btree.clone());
    doc.add_object(frontmatter_bigram_field, fm_btree);
    writer.add_document(doc)?;
    Ok(())
}

// ---- Delete by path ----

fn delete_file_docs(writer: &IndexWriter, schema: &tantivy::schema::Schema, path_str: &str) {
    let path_field = schema.get_field("path").unwrap();
    let term = tantivy::Term::from_field_text(path_field, path_str);
    writer.delete_term(term);
}

// ---- Full index build ----

pub fn build_index(index_dir: &Path, meta: &mut IndexMeta) -> Result<()> {
    let index = open_or_create_index(index_dir, meta.mode)?;
    let schema = index.schema();
    let mut writer = index.writer(WRITER_HEAP)?;

    let files = collect_files(&meta.dirs, meta.include.as_deref(), meta.exclude.as_deref());

    for path in &files {
        let mtime = file_mtime(path);
        let path_str = path.to_string_lossy().to_string();

        match meta.mode {
            IndexMode::Line => {
                let _ = index_file_line(&writer, &schema, path);
            }
            IndexMode::File => {
                let _ = index_file_file(&writer, &schema, path, &mut meta.frontmatter_keys);
            }
        }

        meta.files.insert(path_str, FileRecord { mtime });
    }

    writer.commit()?;
    meta.touch_sync();
    Ok(())
}

// ---- Differential sync ----

pub fn sync_index(index_dir: &Path, meta: &mut IndexMeta) -> Result<()> {
    let index = open_or_create_index(index_dir, meta.mode)?;
    let schema = index.schema();
    let mut writer = index.writer(WRITER_HEAP)?;

    // Collect current files from registered dirs (only dirs that still exist)
    let valid_dirs: Vec<PathBuf> = meta
        .dirs
        .iter()
        .filter(|d| {
            if d.exists() {
                true
            } else {
                eprintln!(
                    "warning: registered directory no longer exists, skipping: {}",
                    d.display()
                );
                false
            }
        })
        .cloned()
        .collect();

    let current_files: Vec<PathBuf> =
        collect_files(&valid_dirs, meta.include.as_deref(), meta.exclude.as_deref());

    let current_set: HashSet<String> = current_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    // Detect deleted files
    let deleted: Vec<String> = meta
        .files
        .keys()
        .filter(|p| !current_set.contains(*p))
        .cloned()
        .collect();

    for path_str in &deleted {
        delete_file_docs(&writer, &schema, path_str);
        meta.files.remove(path_str);
    }

    // Detect added/modified files (parallel mtime scan)
    let to_update: Vec<PathBuf> = current_files
        .par_iter()
        .filter(|p| {
            let path_str = p.to_string_lossy().to_string();
            let mtime = file_mtime(p);
            match meta.files.get(&path_str) {
                None => true,
                Some(rec) => rec.mtime != mtime,
            }
        })
        .cloned()
        .collect();

    for path in &to_update {
        let path_str = path.to_string_lossy().to_string();
        // Remove stale docs first
        delete_file_docs(&writer, &schema, &path_str);

        match meta.mode {
            IndexMode::Line => {
                let _ = index_file_line(&writer, &schema, path);
            }
            IndexMode::File => {
                let _ = index_file_file(&writer, &schema, path, &mut meta.frontmatter_keys);
            }
        }

        let mtime = file_mtime(path);
        meta.files.insert(path_str, FileRecord { mtime });
    }

    writer.commit()?;
    meta.touch_sync();
    Ok(())
}

// ---- Ad-hoc (no persistent index) indexing for file/stdin inputs ----

/// Index a list of files into a RAM index (for direct-file search).
/// Returns the index and the set of frontmatter keys seen (non-empty only for file mode).
pub fn build_ram_index(paths: &[PathBuf], mode: IndexMode) -> Result<(Index, HashSet<String>)> {
    let schema = match mode {
        IndexMode::Line => line_schema(),
        IndexMode::File => file_schema(),
    };
    let index = Index::create_in_ram(schema.clone());
    register_analyzers(&index)?;

    let mut writer = index.writer(WRITER_HEAP)?;
    let mut fm_keys = HashSet::new();

    for path in paths {
        if is_binary(path) {
            continue;
        }
        match mode {
            IndexMode::Line => {
                let _ = index_file_line(&writer, &schema, path);
            }
            IndexMode::File => {
                let _ = index_file_file(&writer, &schema, path, &mut fm_keys);
            }
        }
    }

    writer.commit()?;
    Ok((index, fm_keys))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::meta::IndexMode;

    pub fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("docs")
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.txt"));
        assert!(!glob_match("*.rs", "src/main.rs"));
    }

    #[test]
    fn test_glob_match_double_star() {
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "a/b/c/main.rs"));
        assert!(!glob_match("**/*.rs", "a/b/c/main.txt"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("Cargo.toml", "Cargo.toml"));
        assert!(!glob_match("Cargo.toml", "Cargo.lock"));
    }

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\ntitle: Test\n---\nBody text here.";
        let body = strip_frontmatter(content);
        assert!(body.contains("Body text here."));
        assert!(!body.contains("title: Test"));
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "No frontmatter here.";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn test_build_ram_index_line_mode() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("english_doc.txt")];
        let (index, _) = build_ram_index(&files, IndexMode::Line).unwrap();
        let schema = index.schema();
        assert!(schema.get_field("path").is_ok());
        assert!(schema.get_field("line_number").is_ok());
        assert!(schema.get_field("body_japanese").is_ok());
    }

    #[test]
    fn test_build_ram_index_file_mode() {
        let fixtures = fixtures_dir();
        let files = vec![fixtures.join("rust_intro.md")];
        let (index, fm_keys) = build_ram_index(&files, IndexMode::File).unwrap();
        let schema = index.schema();
        assert!(schema.get_field("path").is_ok());
        assert!(schema.get_field("filename").is_ok());
        assert!(schema.get_field("frontmatter").is_ok());
        assert!(!fm_keys.is_empty(), "expected frontmatter keys from rust_intro.md");
    }

    #[test]
    fn test_build_ram_index_stdin_line() {
        let content = "Rustはシステム言語です。\nメモリ安全性があります。";
        let (index, _) = build_ram_index_stdin(content, IndexMode::Line).unwrap();
        let schema = index.schema();
        assert!(schema.get_field("path").is_ok());
    }

    #[test]
    fn test_collect_files_basic() {
        let fixtures = fixtures_dir();
        let files = collect_files(&[fixtures], None, None);
        assert!(!files.is_empty());
    }

    #[test]
    fn test_collect_files_include_pattern() {
        let fixtures = fixtures_dir();
        let files = collect_files(&[fixtures], Some("**/*.md"), None);
        assert!(!files.is_empty(), "expected .md files");
        assert!(files.iter().all(|p| p.extension().map(|e| e == "md").unwrap_or(false)));
    }

    #[test]
    fn test_collect_files_exclude_pattern() {
        let fixtures = fixtures_dir();
        let all_files = collect_files(&[fixtures.clone()], None, None);
        let filtered = collect_files(&[fixtures], None, Some("**/*.md"));
        assert!(filtered.len() < all_files.len(), "excluding *.md should reduce file count");
        assert!(filtered.iter().all(|p| p.extension().map(|e| e != "md").unwrap_or(true)));
    }
}

/// Index stdin content into a RAM index.
/// Index stdin content into a RAM index.
/// Returns the index and the set of frontmatter keys seen (non-empty only for file mode).
pub fn build_ram_index_stdin(content: &str, mode: IndexMode) -> Result<(Index, HashSet<String>)> {
    let schema = match mode {
        IndexMode::Line => line_schema(),
        IndexMode::File => file_schema(),
    };
    let index = Index::create_in_ram(schema.clone());
    register_analyzers(&index)?;
    let mut writer = index.writer(WRITER_HEAP)?;
    let mut fm_keys = HashSet::new();

    let fake_path = "(standard input)";

    match mode {
        IndexMode::Line => {
            let path_field = schema.get_field("path").unwrap();
            let line_num_field = schema.get_field("line_number").unwrap();
            let body_japanese = schema.get_field("body_japanese").unwrap();
            let body_bigram = schema.get_field("body_bigram").unwrap();
            let body = strip_frontmatter(content);
            for (i, line) in body.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                writer.add_document(doc!(
                    path_field => fake_path.to_string(),
                    line_num_field => (i + 1) as u64,
                    body_japanese => line.to_string(),
                    body_bigram => line.to_string(),
                ))?;
            }
        }
        IndexMode::File => {
            let (fm, body) = parse_frontmatter(content);
            if let Some(obj) = fm.as_object() {
                for k in obj.keys() {
                    fm_keys.insert(k.clone());
                }
            }
            let path_field = schema.get_field("path").unwrap();
            let filename_field = schema.get_field("filename").unwrap();
            let filename_bigram_field = schema.get_field("filename_bigram").unwrap();
            let body_japanese = schema.get_field("body_japanese").unwrap();
            let body_bigram = schema.get_field("body_bigram").unwrap();
            let frontmatter_field = schema.get_field("frontmatter").unwrap();
            let frontmatter_bigram_field = schema.get_field("frontmatter_bigram").unwrap();
            let fm_btree = json_to_btree(&fm);
            let mut tdoc = TantivyDocument::default();
            tdoc.add_text(path_field, fake_path);
            tdoc.add_text(filename_field, "(stdin)");
            tdoc.add_text(filename_bigram_field, "(stdin)");
            tdoc.add_text(body_japanese, body);
            tdoc.add_text(body_bigram, body);
            tdoc.add_object(frontmatter_field, fm_btree.clone());
            tdoc.add_object(frontmatter_bigram_field, fm_btree);
            writer.add_document(tdoc)?;
        }
    }

    writer.commit()?;
    Ok((index, fm_keys))
}
