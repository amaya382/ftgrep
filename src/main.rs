use ftgrep::cli;
use ftgrep::cmd;
use ftgrep::indexer;
use ftgrep::meta;
use ftgrep::registry;
use ftgrep::searcher;

use anyhow::{bail, Result};
use clap::Parser;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use cli::Cli;
use cmd::{
    index_cmd::{run_index, IndexOptions},
    info_cmd::run_info,
    list_cmd::run_list,
    sync_cmd::run_sync,
};
use indexer::{build_ram_index, build_ram_index_stdin, open_or_create_index, sync_index};
use meta::{IndexMeta, IndexMode};
use registry::Registry;
use searcher::{print_file_results, print_line_results, search_file, search_line, SortOrder};

fn main() {
    let result = run();
    match result {
        Ok(had_results) => {
            if had_results {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {:#}", e);
            process::exit(2);
        }
    }
}

/// Returns Ok(true) when at least one result was found, Ok(false) for no results.
fn run() -> Result<bool> {
    let cli = Cli::parse();

    let mode = if cli.by_file {
        IndexMode::File
    } else if cli.by_line {
        IndexMode::Line
    } else if let Some(ref q) = cli.query {
        // Auto-detect file mode when the query uses file-mode-only field qualifiers
        // (filename: or frontmatter.*:) and neither --by-line nor --by-file is given.
        if query_implies_file_mode(q) {
            IndexMode::File
        } else {
            IndexMode::Line
        }
    } else {
        IndexMode::Line
    };

    let sort = SortOrder::parse(&cli.sort).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid sort order: '{}'. Use score, path, or mtime.",
            cli.sort
        )
    })?;

    // ---- Index management mode ----
    if let Some(ref name) = cli.index {
        // In management mode, query slot holds the first positional arg (first dir)
        let mut dirs: Vec<PathBuf> = cli.query.iter().map(PathBuf::from).collect();
        dirs.extend(cli.paths.iter().map(PathBuf::from));

        // When neither --by-line nor --by-file is given, build both modes.
        let modes: &[IndexMode] = if cli.by_file {
            &[IndexMode::File]
        } else if cli.by_line {
            &[IndexMode::Line]
        } else {
            &[IndexMode::Line, IndexMode::File]
        };

        for &m in modes {
            run_index(IndexOptions {
                name,
                dirs: dirs.clone(),
                mode: m,
                index_path: cli.index_path.as_ref().map(|p| {
                    // When building both modes, append the mode suffix to avoid collision.
                    if modes.len() > 1 {
                        PathBuf::from(format!("{}-{}", p, m))
                    } else {
                        PathBuf::from(p)
                    }
                }),
                no_register: cli.no_register,
                no_index: cli.no_index,
                include: cli.include.clone(),
                exclude: cli.exclude.clone(),
            })?;
        }
        return Ok(true);
    }

    if let Some(ref name) = cli.sync {
        run_sync(name, cli.by_line, cli.by_file)?;
        return Ok(true);
    }

    if cli.list {
        run_list(cli.by_line, cli.by_file)?;
        return Ok(true);
    }

    if let Some(ref name) = cli.info {
        run_info(name, cli.by_line, cli.by_file)?;
        return Ok(true);
    }

    // ---- Search mode ----
    if cli.include.is_some() || cli.exclude.is_some() {
        bail!("--include and --exclude cannot be used during search. They are only valid with --index.");
    }

    let query = match cli.query {
        Some(ref q) => q.as_str(),
        None => bail!(
            "A query is required for search. Use management flags (--index, --sync, --list, --info) without a query."
        ),
    };

    // Check if stdin has data (non-tty)
    let stdin_is_pipe = !atty_check();

    if stdin_is_pipe && cli.paths.is_empty() {
        // stdin mode
        let mut content = String::new();
        io::stdin().read_to_string(&mut content)?;
        let (index, fm_keys) = build_ram_index_stdin(&content, mode)?;
        return do_search(&index, query, mode, cli.limit, sort, cli.json, &fm_keys);
    }

    // Classify paths into files and directories
    let paths: Vec<PathBuf> = if cli.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        cli.paths.iter().map(PathBuf::from).collect()
    };

    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut dir_paths: Vec<PathBuf> = Vec::new();

    for p in &paths {
        if !p.exists() {
            bail!("Path does not exist: {}", p.display());
        }
        if p.is_dir() {
            dir_paths.push(p.clone());
        } else {
            file_paths.push(p.clone());
        }
    }

    let mut had_results = false;

    // File paths: ad-hoc RAM index
    if !file_paths.is_empty() {
        let (index, fm_keys) = build_ram_index(&file_paths, mode)?;
        if do_search(&index, query, mode, cli.limit, sort, cli.json, &fm_keys)? {
            had_results = true;
        }
    }

    // Directory paths: use registry index with auto-sync
    if !dir_paths.is_empty() {
        let (index, fm_keys) = resolve_and_sync_dir_index(&dir_paths, mode)?;
        if do_search(&index, query, mode, cli.limit, sort, cli.json, &fm_keys)? {
            had_results = true;
        }
    }

    Ok(had_results)
}

fn do_search(
    index: &tantivy::Index,
    query: &str,
    mode: IndexMode,
    limit: usize,
    sort: SortOrder,
    json: bool,
    frontmatter_keys: &std::collections::HashSet<String>,
) -> Result<bool> {
    match mode {
        IndexMode::Line => {
            let results = search_line(index, query, limit, sort)?;
            Ok(print_line_results(index, &results, query, json))
        }
        IndexMode::File => {
            let results = search_file(index, query, limit, sort, frontmatter_keys)?;
            Ok(print_file_results(index, &results, query, json))
        }
    }
}

/// Resolve (or create) and sync the persistent index for the given directories.
/// Returns the index and the frontmatter keys recorded in its metadata.
fn resolve_and_sync_dir_index(
    dirs: &[PathBuf],
    mode: IndexMode,
) -> Result<(tantivy::Index, std::collections::HashSet<String>)> {
    let abs_dirs: Vec<PathBuf> = dirs
        .iter()
        .map(|d| d.canonicalize().unwrap_or_else(|_| d.clone()))
        .collect();

    let registry = Registry::load()?;

    // Index selection algorithm
    let index_dir = select_index(&registry, &abs_dirs, mode)?;

    let mut meta = if IndexMeta::meta_path(&index_dir).exists() {
        IndexMeta::load(&index_dir)?
    } else {
        // Create new index covering these dirs
        let mut m = IndexMeta::new(mode);
        for d in &abs_dirs {
            m.add_dir(d.clone());
        }
        std::fs::create_dir_all(&index_dir)?;
        m.save(&index_dir)?;
        m
    };

    // Differential sync before search
    sync_index(&index_dir, &mut meta)?;
    meta.save(&index_dir)?;

    let fm_keys = meta.frontmatter_keys.clone();
    let tantivy_index = open_or_create_index(&index_dir, mode)?;
    Ok((tantivy_index, fm_keys))
}

/// Select the best matching index from the registry, or return a path for a new auto index.
fn select_index(registry: &Registry, target_dirs: &[PathBuf], mode: IndexMode) -> Result<PathBuf> {
    let mut candidates: Vec<(PathBuf, IndexMeta)> = Vec::new();

    for (_name, path_str) in &registry.indices {
        let index_dir = PathBuf::from(path_str);
        let meta_path = IndexMeta::meta_path(&index_dir);
        if !meta_path.exists() {
            continue;
        }
        let meta = match IndexMeta::load(&index_dir) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.mode != mode {
            continue;
        }
        // Check if this index covers all target dirs
        let covers_all = target_dirs
            .iter()
            .all(|td| meta.dirs.iter().any(|id| td.starts_with(id) || td == id));
        if covers_all {
            candidates.push((index_dir, meta));
        }
    }

    if candidates.is_empty() {
        return Ok(auto_index_path(target_dirs, mode));
    }

    // Sort: prefer smallest coverage, then newest sync, then lexicographic path
    candidates.sort_by(|a, b| {
        let size_a = a.1.dirs.len();
        let size_b = b.1.dirs.len();
        size_a
            .cmp(&size_b)
            .then_with(|| b.1.last_sync.cmp(&a.1.last_sync))
            .then_with(|| a.0.cmp(&b.0))
    });

    Ok(candidates.remove(0).0)
}

fn auto_index_path(dirs: &[PathBuf], mode: IndexMode) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for d in dirs {
        d.hash(&mut hasher);
    }
    let hash = hasher.finish();
    let mode_str = if mode == IndexMode::File { "file" } else { "line" };

    let data_dir = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local").join("share")
        });
    data_dir
        .join("ftgrep")
        .join("indices")
        .join(mode_str)
        .join(format!("auto_{:x}", hash))
}

/// Return true when the query contains a field qualifier that only makes sense in file mode
/// ("filename:" or "frontmatter.<key>:"). Used to auto-switch to file mode when neither
/// --by-line nor --by-file is explicitly given.
fn query_implies_file_mode(query: &str) -> bool {
    let chars: Vec<char> = query.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        // Skip whitespace
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        // Skip leading +/- operators
        if chars[i] == '+' || chars[i] == '-' {
            i += 1;
        }
        // Collect a potential field name token
        let start = i;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
            i += 1;
        }
        if i < len && chars[i] == ':' && i > start {
            let field: String = chars[start..i].iter().collect();
            if field == "filename" || field.starts_with("frontmatter.") {
                return true;
            }
        }
        // Skip to end of current token
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    false
}

fn atty_check() -> bool {
    use std::os::unix::io::AsRawFd;
    unsafe { libc::isatty(io::stdin().as_raw_fd()) != 0 }
}
