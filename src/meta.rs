use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    Line,
    File,
}

impl std::fmt::Display for IndexMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexMode::Line => write!(f, "line"),
            IndexMode::File => write!(f, "file"),
        }
    }
}

/// Per-file tracking record stored in index metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    /// Modification time (seconds since Unix epoch).
    pub mtime: u64,
}

/// Metadata stored alongside the tantivy index files.
#[derive(Debug, Serialize, Deserialize)]
pub struct IndexMeta {
    pub mode: IndexMode,
    /// Directories registered with this index.
    pub dirs: Vec<PathBuf>,
    /// Optional glob include pattern.
    pub include: Option<String>,
    /// Optional glob exclude pattern.
    pub exclude: Option<String>,
    /// Map from absolute file path string to its tracking record.
    pub files: HashMap<String, FileRecord>,
    /// Timestamp of the last successful sync.
    pub last_sync: Option<DateTime<Utc>>,
    /// All frontmatter keys seen across indexed files (file mode only).
    /// Used to build per-key QueryParser entries so plain queries match frontmatter content.
    #[serde(default)]
    pub frontmatter_keys: HashSet<String>,
}

impl IndexMeta {
    pub fn new(mode: IndexMode) -> Self {
        Self {
            mode,
            dirs: Vec::new(),
            include: None,
            exclude: None,
            files: HashMap::new(),
            last_sync: None,
            frontmatter_keys: HashSet::new(),
        }
    }

    pub fn meta_path(index_dir: &Path) -> PathBuf {
        index_dir.join("ftgrep_meta.json")
    }

    pub fn load(index_dir: &Path) -> Result<Self> {
        let path = Self::meta_path(index_dir);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read metadata at {}", path.display()))?;
        serde_json::from_str(&content).context("Failed to parse index metadata JSON")
    }

    pub fn save(&self, index_dir: &Path) -> Result<()> {
        let path = Self::meta_path(index_dir);
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write metadata at {}", path.display()))
    }

    pub fn add_dir(&mut self, dir: PathBuf) -> bool {
        if self.dirs.contains(&dir) {
            return false;
        }
        self.dirs.push(dir);
        true
    }

    pub fn touch_sync(&mut self) {
        self.last_sync = Some(Utc::now());
    }

    pub fn doc_count(&self) -> usize {
        self.files.len()
    }
}
