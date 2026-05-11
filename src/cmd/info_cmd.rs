use anyhow::Result;

use crate::meta::{IndexMeta, IndexMode};
use crate::registry::Registry;

pub fn run_info(name: &str, by_line: bool, by_file: bool) -> Result<()> {
    let registry = Registry::load()?;
    let index_dir = registry.resolve(name)?;

    let filter_mode: Option<IndexMode> = if by_line && !by_file {
        Some(IndexMode::Line)
    } else if by_file && !by_line {
        Some(IndexMode::File)
    } else {
        None
    };

    let meta_path = IndexMeta::meta_path(&index_dir);
    if !meta_path.exists() {
        anyhow::bail!("No index found at {}", index_dir.display());
    }

    let meta = IndexMeta::load(&index_dir)?;

    if let Some(mode) = filter_mode {
        if meta.mode != mode {
            println!("Index '{}' does not have a {} mode index.", name, mode);
            return Ok(());
        }
    }

    println!("Name:       {}", name);
    println!("Path:       {}", index_dir.display());
    println!("Mode:       {}", meta.mode);
    println!("Documents:  {}", meta.doc_count());
    println!(
        "Last sync:  {}",
        meta.last_sync
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "(never)".to_string())
    );
    println!(
        "Include:    {}",
        meta.include.as_deref().unwrap_or("(none)")
    );
    println!(
        "Exclude:    {}",
        meta.exclude.as_deref().unwrap_or("(none)")
    );
    println!("Directories ({}):", meta.dirs.len());
    for dir in &meta.dirs {
        println!("  {}", dir.display());
    }

    Ok(())
}
