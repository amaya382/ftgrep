use anyhow::{bail, Result};
use std::path::PathBuf;

use crate::indexer::build_index;
use crate::meta::{IndexMeta, IndexMode};
use crate::registry::Registry;

pub struct IndexOptions<'a> {
    pub name: &'a str,
    pub dirs: Vec<PathBuf>,
    pub mode: IndexMode,
    pub index_path: Option<PathBuf>,
    pub no_register: bool,
    pub no_index: bool,
    pub include: Option<String>,
    pub exclude: Option<String>,
}

pub fn run_index(opts: IndexOptions<'_>) -> Result<()> {
    if opts.dirs.is_empty() {
        bail!("--index requires at least one directory argument");
    }

    // Resolve index path
    let index_dir = if let Some(p) = opts.index_path.clone() {
        p
    } else if opts.name.starts_with('/') || opts.name.starts_with('.') {
        PathBuf::from(opts.name)
    } else {
        Registry::default_index_path(opts.name, opts.mode == IndexMode::File)
    };

    // Load or create metadata
    let meta_path = crate::meta::IndexMeta::meta_path(&index_dir);
    let mut meta = if meta_path.exists() {
        IndexMeta::load(&index_dir)?
    } else {
        IndexMeta::new(opts.mode)
    };

    if meta.mode != opts.mode {
        bail!(
            "Index '{}' exists with mode '{}' but you specified '{}'",
            opts.name,
            meta.mode,
            opts.mode
        );
    }

    // Apply include/exclude overrides
    if let Some(ref pat) = opts.include {
        meta.include = Some(pat.clone());
    }
    if let Some(ref pat) = opts.exclude {
        meta.exclude = Some(pat.clone());
    }

    // Add new directories (skip already registered)
    let mut added = 0usize;
    for dir in opts.dirs {
        let abs = dir
            .canonicalize()
            .unwrap_or_else(|_| dir.clone());
        if meta.add_dir(abs.clone()) {
            eprintln!("Added directory: {}", abs.display());
            added += 1;
        } else {
            eprintln!("Directory already registered, skipping: {}", abs.display());
        }
    }

    // Register in registry unless suppressed
    if !opts.no_register && !(opts.name.starts_with('/') || opts.name.starts_with('.')) {
        let mut registry = Registry::load()?;
        registry.register(opts.name, &index_dir)?;
    }

    // Save metadata before indexing
    std::fs::create_dir_all(&index_dir)?;
    meta.save(&index_dir)?;

    if opts.no_index {
        eprintln!("Skipping indexing (--no-index specified).");
        return Ok(());
    }

    if added > 0 || meta.files.is_empty() {
        eprintln!("Indexing {} directory/ies...", meta.dirs.len());
        build_index(&index_dir, &mut meta)?;
        meta.save(&index_dir)?;
        eprintln!(
            "Done. {} documents indexed.",
            meta.doc_count()
        );
    } else {
        eprintln!("No new directories added. Running sync instead.");
        crate::indexer::sync_index(&index_dir, &mut meta)?;
        meta.save(&index_dir)?;
    }

    Ok(())
}
