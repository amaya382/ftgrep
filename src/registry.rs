use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Registry {
    pub indices: HashMap<String, String>,
}

impl Registry {
    pub fn path() -> PathBuf {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                PathBuf::from(home).join(".config")
            });
        config_dir.join("ftgrep").join("registry.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read registry at {}", path.display()))?;
        serde_json::from_str(&content).context("Failed to parse registry JSON")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create registry dir {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write registry at {}", path.display()))
    }

    pub fn register(&mut self, name: &str, index_path: &Path) -> Result<()> {
        self.indices
            .insert(name.to_string(), index_path.to_string_lossy().to_string());
        self.save()
    }

    pub fn resolve(&self, name_or_path: &str) -> Result<PathBuf> {
        if name_or_path.starts_with('/') || name_or_path.starts_with('.') {
            return Ok(PathBuf::from(name_or_path));
        }
        self.indices
            .get(name_or_path)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("Index '{}' not found in registry", name_or_path))
    }

    pub fn default_index_path(name: &str, by_file: bool) -> PathBuf {
        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                PathBuf::from(home).join(".local").join("share")
            });
        let mode = if by_file { "file" } else { "line" };
        data_dir.join("ftgrep").join("indices").join(mode).join(name)
    }
}
