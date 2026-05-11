use anyhow::Result;

use crate::meta::{IndexMeta, IndexMode};
use crate::registry::Registry;

pub fn run_list(by_line: bool, by_file: bool) -> Result<()> {
    let registry = Registry::load()?;

    if registry.indices.is_empty() {
        println!("No indices registered.");
        return Ok(());
    }

    let filter_mode: Option<IndexMode> = if by_line && !by_file {
        Some(IndexMode::Line)
    } else if by_file && !by_line {
        Some(IndexMode::File)
    } else {
        None
    };

    println!(
        "{:<20} {:<6} {:<40} {}",
        "NAME", "MODE", "DIRS", "LAST SYNC"
    );
    println!("{}", "-".repeat(80));

    let mut entries: Vec<(String, String)> = registry.indices.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, path) in entries {
        let index_dir = std::path::PathBuf::from(&path);
        let meta_path = IndexMeta::meta_path(&index_dir);

        if !meta_path.exists() {
            println!("{:<20} {:<6} {:<40} {}", name, "?", path, "(no metadata)");
            continue;
        }

        let meta = match IndexMeta::load(&index_dir) {
            Ok(m) => m,
            Err(_) => {
                println!("{:<20} {:<6} {:<40} {}", name, "?", path, "(error)");
                continue;
            }
        };

        if let Some(mode) = filter_mode {
            if meta.mode != mode {
                continue;
            }
        }

        let dirs_str = meta
            .dirs
            .iter()
            .map(|d| {
                let home = std::env::var("HOME").unwrap_or_default();
                let s = d.to_string_lossy().to_string();
                if !home.is_empty() && s.starts_with(&home) {
                    s.replacen(&home, "~", 1)
                } else {
                    s
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let last_sync = meta
            .last_sync
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "(never)".to_string());

        println!(
            "{:<20} {:<6} {:<40} {}",
            name, meta.mode, dirs_str, last_sync
        );
    }

    Ok(())
}
