use std::path::PathBuf;
use std::process::Command;

fn bin_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ftgrep")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("docs")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin_path())
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to execute ftgrep binary")
}

fn run_with_stdin(args: &[&str], input: &str) -> std::process::Output {
    use std::io::Write;
    let mut child = Command::new(bin_path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("NO_COLOR", "1")
        .spawn()
        .expect("failed to spawn ftgrep binary");

    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        stdin.write_all(input.as_bytes()).ok();
    }

    child.wait_with_output().expect("failed to wait on child")
}

// ---- File-mode search (no persistent index) ----

#[test]
fn test_file_search_english_hit() {
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["search", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "expected exit 0 (results found)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("search"), "output should contain 'search': {}", stdout);
}

#[test]
fn test_file_search_japanese_hit() {
    let file = fixtures_dir().join("search_engine.txt");
    let out = run(&["検索", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "expected exit 0 (results found)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.is_empty(), "expected non-empty output");
}

#[test]
fn test_file_search_no_results() {
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["絶対ヒットしない", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "expected exit 1 (no results)");
}

#[test]
fn test_file_search_by_file_mode() {
    let file = fixtures_dir().join("rust_intro.md");
    let out = run(&["--by-file", "Rust", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rust_intro.md"), "output path should contain 'rust_intro.md': {}", stdout);
}

#[test]
fn test_file_search_json_output() {
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["--json", "search", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("output should be valid JSON");
    assert!(parsed.is_array(), "JSON output should be an array");
    assert!(!parsed.as_array().unwrap().is_empty(), "JSON array should not be empty");
}

#[test]
fn test_file_search_line_number_in_output() {
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["search", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Line mode output format: path:line_number:content
    let has_line_numbers = stdout.lines().any(|l| {
        let parts: Vec<&str> = l.splitn(3, ':').collect();
        parts.len() == 3 && parts[1].parse::<u64>().is_ok()
    });
    assert!(has_line_numbers, "expected line numbers in output: {}", stdout);
}

// ---- File mode: output format, JSON schema, frontmatter, stdin ----

#[test]
fn test_file_mode_output_format_is_path_colon_snippet() {
    // File mode output: "path:snippet" — exactly 2 colon-separated fields (path may contain colons
    // on Windows but on Unix the first colon separates path from snippet).
    let file = fixtures_dir().join("search_engine.txt");
    let out = run(&["--by-file", "検索", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().expect("expected at least one output line");
    // Must NOT have a numeric second field (which would indicate line mode format path:N:content)
    let parts: Vec<&str> = line.splitn(3, ':').collect();
    assert!(
        parts.len() >= 2 && parts[1].parse::<u64>().is_err(),
        "file mode output should be 'path:snippet', not 'path:N:content': {:?}", line
    );
}

#[test]
fn test_file_mode_json_schema() {
    // File mode JSON: array of objects with "path" and "snippets" (NOT "line_number"/"lines")
    let file = fixtures_dir().join("rust_intro.md");
    let out = run(&["--by-file", "--json", "Rust", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let arr: serde_json::Value = serde_json::from_str(&stdout).expect("expected valid JSON");
    let obj = arr.as_array().unwrap().first().expect("expected at least one result");
    assert!(obj.get("path").is_some(), "file mode JSON should have 'path' field");
    assert!(obj.get("snippets").is_some(), "file mode JSON should have 'snippets' field");
    assert!(obj.get("line_number").is_none(), "file mode JSON must NOT have 'line_number'");
    assert!(obj.get("lines").is_none(), "file mode JSON must NOT have 'lines'");
}

#[test]
fn test_line_mode_json_schema() {
    // Line mode JSON: array of objects with "path", "line_number", "lines" (NOT "snippets")
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["--json", "search", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let arr: serde_json::Value = serde_json::from_str(&stdout).expect("expected valid JSON");
    let obj = arr.as_array().unwrap().first().expect("expected at least one result");
    assert!(obj.get("path").is_some(), "line mode JSON should have 'path' field");
    assert!(obj.get("line_number").is_some(), "line mode JSON should have 'line_number' field");
    assert!(obj.get("lines").is_some(), "line mode JSON should have 'lines' field");
    assert!(obj.get("snippets").is_none(), "line mode JSON must NOT have 'snippets'");
}

#[test]
fn test_file_mode_frontmatter_search() {
    // File mode indexes frontmatter as "frontmatter.*" dynamic fields.
    // Querying with field qualifier "frontmatter.author:テスト太郎" should hit the file.
    // rust_intro.md has frontmatter: title: Rust入門, author: テスト太郎
    let file = fixtures_dir().join("rust_intro.md");
    let out = run(&["--by-file", "frontmatter.author:テスト太郎", file.to_str().unwrap()]);
    assert_eq!(
        out.status.code(), Some(0),
        "file mode should find documents by frontmatter field qualifier"
    );
}

#[test]
fn test_file_mode_frontmatter_search_without_qualifier() {
    // Frontmatter values are also indexed into body_japanese/body_bigram, so a plain query
    // (no field qualifier) should match frontmatter-only content.
    // "テスト太郎" appears only in the frontmatter author field of rust_intro.md.
    let file = fixtures_dir().join("rust_intro.md");
    let out = run(&["--by-file", "テスト太郎", file.to_str().unwrap()]);
    assert_eq!(
        out.status.code(), Some(0),
        "file mode should find documents by frontmatter value even without field qualifier"
    );
}

#[test]
fn test_line_mode_ignores_frontmatter() {
    // Line mode strips frontmatter; the author field must NOT be searchable as a body line.
    // "テスト太郎" only appears in the frontmatter of rust_intro.md.
    let file = fixtures_dir().join("rust_intro.md");
    let out = run(&["テスト太郎", file.to_str().unwrap()]);
    assert_eq!(
        out.status.code(), Some(1),
        "line mode should NOT match frontmatter-only content"
    );
}

#[test]
fn test_file_mode_stdin() {
    // --by-file on stdin: one document = the whole stdin content; path should be "(standard input)"
    let out = run_with_stdin(
        &["--by-file", "形態素"],
        "形態素解析による全文検索の実装。\n日本語テキストを適切に処理します。\n",
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("(standard input)"),
        "file mode stdin path should be '(standard input)': {}", stdout
    );
}

#[test]
fn test_file_mode_highlight_colored() {
    // File mode output ("path:snippet") should also be highlighted with ANSI when FORCE_COLOR is set.
    let file = fixtures_dir().join("search_engine.txt");
    let out = run_with_force_color(&["--by-file", "検索", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ANSI_BOLD_RED),
        "file mode output should contain ANSI highlight: {:?}", stdout
    );
}

// ---- Base-form (morphological) search ----

#[test]
fn test_base_form_search() {
    let file = fixtures_dir().join("notes.md");
    // "走る" should match "走っていた" in the document (base form lemmatization)
    let out = run(&["走る", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "base form '走る' should match '走っていた'");
}

// ---- stdin search ----

#[test]
fn test_stdin_search_hit() {
    let out = run_with_stdin(
        &["メモリ"],
        "Rustはシステムプログラミング言語です。\nメモリ安全性を保証します。\n",
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("(standard input)"), "stdin path should appear: {}", stdout);
}

#[test]
fn test_stdin_search_no_results() {
    let out = run_with_stdin(&["絶対ヒットしない"], "This is plain English content.\n");
    assert_eq!(out.status.code(), Some(1), "expected exit 1 for no results");
}

// ---- Directory search with persistent index ----

#[test]
fn test_directory_search_auto_index() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("test.txt"), "全文検索エンジンのテストです。\nインデックスを構築します。\n").unwrap();

    let out = Command::new(bin_path())
        .args(["検索", docs.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", tmpdir.path().join("data").to_str().unwrap())
        .env("XDG_CONFIG_HOME", tmpdir.path().join("config").to_str().unwrap())
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0), "directory search should find '検索'");
}

#[test]
fn test_index_and_search() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("doc1.txt"), "形態素解析による日本語全文検索。\n").unwrap();
    std::fs::write(docs.join("doc2.txt"), "Rustで実装された高速検索エンジン。\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    // Create named index
    let index_out = Command::new(bin_path())
        .args(["--index", "test-idx", "--by-line", docs.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(index_out.status.code(), Some(0), "ftgrep --index should succeed");

    // Search via the directory (uses the registered index)
    let search_out = Command::new(bin_path())
        .args(["検索", docs.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(search_out.status.code(), Some(0), "search should find '検索'");
}

#[test]
fn test_list_command() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("note.txt"), "テストデータ\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    Command::new(bin_path())
        .args(["--index", "my-idx", "--by-line", docs.to_str().unwrap()])
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();

    let list_out = Command::new(bin_path())
        .args(["--list"])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(list_out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(stdout.contains("my-idx"), "list output should show 'my-idx': {}", stdout);
}

#[test]
fn test_sync_command() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("initial.txt"), "初期ドキュメント\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    Command::new(bin_path())
        .args(["--index", "sync-idx", "--by-line", docs.to_str().unwrap()])
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();

    // Add a new file and sync
    std::fs::write(docs.join("added.txt"), "追加されたドキュメント\n").unwrap();

    let sync_out = Command::new(bin_path())
        .args(["--sync", "sync-idx", "--by-line"])
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap();
    assert_eq!(sync_out.status.code(), Some(0), "sync should succeed");
}

// ---- Normalization: NFKC (full-width → half-width) ----

#[test]
fn test_norm_nfkc_query_matches_halfwidth_content() {
    // Document contains half-width "RUST"; query uses full-width "ＲＵＳＴ".
    // NFKC normalization makes them equivalent.
    let out = run_with_stdin(&["ＲＵＳＴ"], "RUST is a systems programming language.\n");
    assert_eq!(out.status.code(), Some(0), "full-width query should match half-width content via NFKC");
}

#[test]
fn test_norm_nfkc_content_matches_halfwidth_query() {
    // Document contains full-width "ＲＵＳＴ"; query uses half-width "RUST".
    let file = fixtures_dir().join("normalization.txt");
    let out = run(&["RUST", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "half-width query should match full-width content via NFKC");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("全角"),
        "matched line should be the full-width sample line: {}", stdout
    );
}

// ---- Normalization: case-insensitive (lowercase) ----

#[test]
fn test_norm_lowercase_uppercase_query_matches_lowercase_content() {
    // Document has "rust" (lowercase); query is "RUST" (uppercase).
    let out = run_with_stdin(&["RUST"], "rust is a language.\n");
    assert_eq!(out.status.code(), Some(0), "uppercase query should match lowercase content");
}

#[test]
fn test_norm_lowercase_lowercase_query_matches_uppercase_content() {
    // Document has "Rust" (title-case); query is "rust" (lowercase).
    let file = fixtures_dir().join("normalization.txt");
    let out = run(&["rust", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "lowercase query should match title-case content");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both "Rust" and "rust" lines should appear
    let hit_count = stdout.lines().count();
    assert!(hit_count >= 2, "expected at least 2 matching lines, got {}: {}", hit_count, stdout);
}

// ---- Normalization: katakana → hiragana ----

#[test]
fn test_norm_kata_to_hira_katakana_query_matches_hiragana_content() {
    // Document contains hiragana "もでる"; query uses katakana "モデル".
    let out = run_with_stdin(&["モデル"], "もでるは仮名の変換テストです。\n");
    assert_eq!(out.status.code(), Some(0), "katakana query should match hiragana content");
}

#[test]
fn test_norm_kata_to_hira_hiragana_query_matches_katakana_content() {
    // Document contains katakana "モデル"; query uses hiragana "もでる".
    let out = run_with_stdin(&["もでる"], "モデルは変換テストです。\n");
    assert_eq!(out.status.code(), Some(0), "hiragana query should match katakana content");
}

// ---- Normalization: long vowel (ー) removal ----

#[test]
fn test_norm_long_vowel_with_macron_matches_without() {
    // Query has ー (long vowel mark); document has short form without ー.
    let file = fixtures_dir().join("normalization.txt");
    let out = run(&["コンピューター", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "long-vowel query should match short-form content");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both the ー line and the non-ー line should be found
    assert!(
        stdout.lines().count() >= 2,
        "both コンピューター and コンピュータ lines should match: {}", stdout
    );
}

#[test]
fn test_norm_long_vowel_without_macron_matches_with() {
    // Query has no ー; document has ー form. They should still match.
    let out = run_with_stdin(&["コンピュータ"], "コンピューターは便利です。\n");
    assert_eq!(out.status.code(), Some(0), "short-form query should match long-vowel content");
}

// ---- Highlight: ANSI escape codes ----

fn run_with_force_color(args: &[&str]) -> std::process::Output {
    Command::new(bin_path())
        .args(args)
        .env("FORCE_COLOR", "1")
        .env_remove("NO_COLOR")
        .output()
        .expect("failed to execute ftgrep binary")
}

fn run_stdin_with_force_color(args: &[&str], input: &str) -> std::process::Output {
    use std::io::Write;
    let mut child = Command::new(bin_path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("FORCE_COLOR", "1")
        .env_remove("NO_COLOR")
        .spawn()
        .expect("failed to spawn ftgrep binary");

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).ok();
    }

    child.wait_with_output().expect("failed to wait on child")
}

const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_RESET: &str = "\x1b[0m";

#[test]
fn test_highlight_ascii_term_is_colored() {
    // "search" appears in english_doc.txt — with FORCE_COLOR the matched word should be wrapped
    // in ANSI bold-red escape codes.
    let file = fixtures_dir().join("english_doc.txt");
    let out = run_with_force_color(&["search", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ANSI_BOLD_RED),
        "expected ANSI bold-red escape in output: {:?}", stdout
    );
    assert!(
        stdout.contains(ANSI_RESET),
        "expected ANSI reset escape in output: {:?}", stdout
    );
}

#[test]
fn test_highlight_japanese_term_is_colored() {
    let file = fixtures_dir().join("search_engine.txt");
    let out = run_with_force_color(&["検索", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ANSI_BOLD_RED),
        "expected ANSI bold-red escape for Japanese query: {:?}", stdout
    );
}

#[test]
fn test_highlight_no_color_env_disables_ansi() {
    // NO_COLOR must suppress all ANSI output even when stdout might look like a tty.
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["search", file.to_str().unwrap()]); // run() already sets NO_COLOR=1
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(ANSI_BOLD_RED),
        "NO_COLOR should suppress ANSI escapes: {:?}", stdout
    );
}

#[test]
fn test_highlight_only_matched_region_colored() {
    // The colored region must wrap only the matched token, not the whole line.
    // We verify by checking that there is text both before the escape and after the reset
    // on the same output line.
    let out = run_stdin_with_force_color(
        &["search"],
        "full-text search engine\n",
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().expect("expected at least one output line");

    // The line must contain text BEFORE the highlight starts (e.g. "full-text ")
    let esc_pos = line.find(ANSI_BOLD_RED).expect("expected ANSI escape");
    assert!(esc_pos > 0, "expected non-highlighted prefix before the match: {:?}", line);

    // And text AFTER the reset (e.g. " engine")
    let reset_pos = line.find(ANSI_RESET).expect("expected ANSI reset");
    let after_reset = &line[reset_pos + ANSI_RESET.len()..];
    // Strip trailing path:lineno: prefix — the content portion is what matters.
    // The raw output line is "path:N:...full-text \e[1;31msearch\e[0m engine"
    // so after_reset will include " engine"
    assert!(!after_reset.trim().is_empty(), "expected non-highlighted suffix after the match: {:?}", line);
}

#[test]
fn test_highlight_stdin_base_form_colored() {
    // "走る" (base form) should match "走っていた" and that surface form should be highlighted.
    let out = run_stdin_with_force_color(
        &["走る"],
        "猫が走っていた。\n",
    );
    assert_eq!(out.status.code(), Some(0), "base form '走る' should match '走っていた'");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ANSI_BOLD_RED),
        "matched surface form should be highlighted: {:?}", stdout
    );
    // The surface form "走っていた" should appear somewhere in the colored region
    assert!(
        stdout.contains("走"),
        "output should contain the matched kanji: {:?}", stdout
    );
}

#[test]
fn test_highlight_normalization_katakana_query_colors_katakana_content() {
    // Query "もでる" (hiragana) matches "モデル" (katakana) in content.
    // The original katakana surface form should be highlighted, not the normalized form.
    let out = run_stdin_with_force_color(&["もでる"], "モデルの評価を行います。\n");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(ANSI_BOLD_RED),
        "katakana content should be highlighted when matched via hiragana query: {:?}", stdout
    );
    assert!(
        stdout.contains("モデル") || stdout.contains("モ"),
        "original katakana surface should appear in output: {:?}", stdout
    );
}

// ---- Re-search after sync: differential sync correctness ----

fn make_search_cmd(
    query: &str,
    dir: &std::path::Path,
    data_home: &std::path::Path,
    config_home: &std::path::Path,
) -> std::process::Output {
    Command::new(bin_path())
        .args([query, dir.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap()
}

fn make_index_cmd(
    idx_name: &str,
    dir: &std::path::Path,
    data_home: &std::path::Path,
    config_home: &std::path::Path,
) -> std::process::Output {
    Command::new(bin_path())
        .args(["--index", idx_name, "--by-line", dir.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap()
}

fn make_sync_cmd(
    idx_name: &str,
    data_home: &std::path::Path,
    config_home: &std::path::Path,
) -> std::process::Output {
    Command::new(bin_path())
        .args(["--sync", idx_name, "--by-line"])
        .env("NO_COLOR", "1")
        .env("XDG_DATA_HOME", data_home.to_str().unwrap())
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .output()
        .unwrap()
}

/// sync後に新規追加ファイルが再検索で見つかることを確認する
#[test]
fn test_resync_added_file_is_searchable() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("original.txt"), "元のドキュメントです。\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    // 初期インデックス作成
    let idx_out = make_index_cmd("resync-add", &docs, &data_home, &config_home);
    assert_eq!(idx_out.status.code(), Some(0), "initial index should succeed");

    // 初期状態：新規ファイルの内容はまだヒットしない
    let before = make_search_cmd("追加コンテンツ", &docs, &data_home, &config_home);
    assert_eq!(before.status.code(), Some(1), "added content should not be found before sync");

    // 新規ファイルを追加
    std::fs::write(docs.join("added.txt"), "追加コンテンツが含まれています。\n").unwrap();

    // sync実行
    let sync_out = make_sync_cmd("resync-add", &data_home, &config_home);
    assert_eq!(sync_out.status.code(), Some(0), "sync should succeed");

    // sync後：新規ファイルの内容がヒットする
    let after = make_search_cmd("追加コンテンツ", &docs, &data_home, &config_home);
    assert_eq!(after.status.code(), Some(0), "added file content should be found after sync");
    let stdout = String::from_utf8_lossy(&after.stdout);
    assert!(
        stdout.contains("added.txt"),
        "search result should reference added.txt: {}", stdout
    );
}

/// sync後に削除されたファイルの内容が再検索でヒットしないことを確認する
#[test]
fn test_resync_deleted_file_is_not_searchable() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("keep.txt"), "残留するドキュメント。\n").unwrap();
    std::fs::write(docs.join("delete_me.txt"), "削除予定のコンテンツ。\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    // 初期インデックス作成（削除対象ファイルも含む）
    let idx_out = make_index_cmd("resync-del", &docs, &data_home, &config_home);
    assert_eq!(idx_out.status.code(), Some(0), "initial index should succeed");

    // 初期状態：削除予定コンテンツがヒットする
    let before = make_search_cmd("削除予定", &docs, &data_home, &config_home);
    assert_eq!(before.status.code(), Some(0), "content should be found before deletion");

    // ファイル削除
    std::fs::remove_file(docs.join("delete_me.txt")).unwrap();

    // sync実行
    let sync_out = make_sync_cmd("resync-del", &data_home, &config_home);
    assert_eq!(sync_out.status.code(), Some(0), "sync should succeed after file deletion");

    // sync後：削除済みファイルのコンテンツはヒットしない
    let after = make_search_cmd("削除予定", &docs, &data_home, &config_home);
    assert_eq!(after.status.code(), Some(1), "deleted file content should NOT be found after sync");
}

/// sync後にファイル内容を変更した場合、旧内容がヒットせず新内容がヒットすることを確認する
#[test]
fn test_resync_modified_file_reflects_new_content() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let target = docs.join("mutable.txt");
    std::fs::write(&target, "変更前のテキスト内容。\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    // 初期インデックス作成
    let idx_out = make_index_cmd("resync-mod", &docs, &data_home, &config_home);
    assert_eq!(idx_out.status.code(), Some(0), "initial index should succeed");

    // 変更前内容がヒットする
    let before = make_search_cmd("変更前", &docs, &data_home, &config_home);
    assert_eq!(before.status.code(), Some(0), "old content should be found before modification");

    // mtimeが変わるよう1秒待ってからファイル内容を変更する
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(&target, "変更後のテキスト内容。\n").unwrap();

    // sync実行
    let sync_out = make_sync_cmd("resync-mod", &data_home, &config_home);
    assert_eq!(sync_out.status.code(), Some(0), "sync should succeed after modification");

    // sync後：旧内容はヒットしない
    let old_content = make_search_cmd("変更前", &docs, &data_home, &config_home);
    assert_eq!(old_content.status.code(), Some(1), "old content should NOT be found after sync");

    // sync後：新内容がヒットする
    let new_content = make_search_cmd("変更後", &docs, &data_home, &config_home);
    assert_eq!(new_content.status.code(), Some(0), "new content should be found after sync");
    let stdout = String::from_utf8_lossy(&new_content.stdout);
    assert!(
        stdout.contains("mutable.txt"),
        "result should reference mutable.txt: {}", stdout
    );
}

/// 複数回syncしても正しく差分が適用されることを確認する
#[test]
fn test_resync_multiple_rounds_correct() {
    let tmpdir = tempdir();
    let docs = tmpdir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("base.txt"), "ベースドキュメント。\n").unwrap();

    let data_home = tmpdir.path().join("data");
    let config_home = tmpdir.path().join("config");

    // 初期インデックス作成
    make_index_cmd("resync-multi", &docs, &data_home, &config_home);

    // 1回目: ファイル追加 → sync
    std::fs::write(docs.join("round1.txt"), "第一ラウンドのコンテンツ。\n").unwrap();
    let sync1 = make_sync_cmd("resync-multi", &data_home, &config_home);
    assert_eq!(sync1.status.code(), Some(0), "first sync should succeed");

    let r1 = make_search_cmd("第一ラウンド", &docs, &data_home, &config_home);
    assert_eq!(r1.status.code(), Some(0), "round1 content should be found after first sync");

    // 2回目: 別ファイル追加 → sync
    std::fs::write(docs.join("round2.txt"), "第二ラウンドのコンテンツ。\n").unwrap();
    let sync2 = make_sync_cmd("resync-multi", &data_home, &config_home);
    assert_eq!(sync2.status.code(), Some(0), "second sync should succeed");

    let r2 = make_search_cmd("第二ラウンド", &docs, &data_home, &config_home);
    assert_eq!(r2.status.code(), Some(0), "round2 content should be found after second sync");

    // 1回目のコンテンツも引き続きヒットする
    let r1_still = make_search_cmd("第一ラウンド", &docs, &data_home, &config_home);
    assert_eq!(r1_still.status.code(), Some(0), "round1 content should still be found after second sync");

    // 3回目: round1を削除 → sync
    std::fs::remove_file(docs.join("round1.txt")).unwrap();
    let sync3 = make_sync_cmd("resync-multi", &data_home, &config_home);
    assert_eq!(sync3.status.code(), Some(0), "third sync should succeed");

    let r1_gone = make_search_cmd("第一ラウンド", &docs, &data_home, &config_home);
    assert_eq!(r1_gone.status.code(), Some(1), "round1 content should NOT be found after deletion sync");

    // round2は残っている
    let r2_remain = make_search_cmd("第二ラウンド", &docs, &data_home, &config_home);
    assert_eq!(r2_remain.status.code(), Some(0), "round2 content should still be found");
}

// ---- Error cases ----

#[test]
fn test_nonexistent_path_error() {
    let out = run(&["query", "/nonexistent/path/that/does/not/exist"]);
    assert!(out.status.code().unwrap_or(0) >= 2, "should exit with error code >= 2");
}

#[test]
fn test_invalid_sort_order_error() {
    let file = fixtures_dir().join("english_doc.txt");
    let out = run(&["--sort", "invalid", "search", file.to_str().unwrap()]);
    assert!(out.status.code().unwrap_or(0) >= 2, "invalid sort should exit with error");
}

// ---- Helper ----

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir() -> TempDir {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("ftgrep_e2e_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&path).unwrap();
    TempDir { path }
}
