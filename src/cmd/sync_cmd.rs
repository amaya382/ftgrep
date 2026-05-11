use anyhow::Result;

use crate::indexer::sync_index;
use crate::meta::{IndexMeta, IndexMode};
use crate::registry::Registry;

pub fn run_sync(name: &str, by_line: bool, by_file: bool) -> Result<()> {
    let registry = Registry::load()?;
    let index_dir = registry.resolve(name)?;

    // Determine which modes to sync
    let modes: Vec<IndexMode> = if by_line && !by_file {
        vec![IndexMode::Line]
    } else if by_file && !by_line {
        vec![IndexMode::File]
    } else {
        vec![IndexMode::Line, IndexMode::File]
    };

    for mode in modes {
        // Check if index exists for this mode
        let meta_path = IndexMeta::meta_path(&index_dir);
        if !meta_path.exists() {
            eprintln!(
                "No index found at {}; skipping mode '{}'.",
                index_dir.display(),
                mode
            );
            continue;
        }

        let mut meta = IndexMeta::load(&index_dir)?;
        if meta.mode != mode {
            continue;
        }

        eprintln!("Syncing index '{}' (mode: {})...", name, mode);
        sync_index(&index_dir, &mut meta)?;
        meta.save(&index_dir)?;
        eprintln!(
            "Sync complete. {} documents.",
            meta.doc_count()
        );
    }

    Ok(())
}
