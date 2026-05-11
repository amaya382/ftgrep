use clap::{ArgAction, Parser};

#[derive(Parser, Debug)]
#[command(
    name = "ftgrep",
    about = "Full-text search tool with Japanese morphological analysis support",
    long_about = None,
    disable_help_flag = false,
)]
pub struct Cli {
    /// Search query (omit for index management mode)
    pub query: Option<String>,

    /// Paths to search (files or directories; defaults to current directory)
    pub paths: Vec<String>,

    // --- Search mode ---
    /// Search in line mode (default)
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "by_file")]
    pub by_line: bool,

    /// Search in file mode
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "by_line")]
    pub by_file: bool,

    /// Maximum number of results (default: 100)
    #[arg(long, short = 'm', default_value_t = 100)]
    pub limit: usize,

    /// Sort order: score, path, or mtime
    #[arg(long, default_value = "score")]
    pub sort: String,

    /// Output in JSON format
    #[arg(long, action = ArgAction::SetTrue)]
    pub json: bool,

    // --- Index management ---
    /// Create or update an index: --index <name> <dir>...
    #[arg(long, value_name = "NAME", conflicts_with_all = ["sync", "list", "info"])]
    pub index: Option<String>,

    /// Sync an index: --sync <name>
    #[arg(long, value_name = "NAME", conflicts_with_all = ["index", "list", "info"])]
    pub sync: Option<String>,

    /// List all registered indices
    #[arg(long, action = ArgAction::SetTrue, conflicts_with_all = ["index", "sync", "info"])]
    pub list: bool,

    /// Show details of an index: --info <name>
    #[arg(long, value_name = "NAME", conflicts_with_all = ["index", "sync", "list"])]
    pub info: Option<String>,

    // --- Index creation options ---
    /// Custom path for the index (used with --index)
    #[arg(long, value_name = "PATH")]
    pub index_path: Option<String>,

    /// Do not register the index in the registry (used with --index)
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_register: bool,

    /// Register dirs/patterns without indexing (used with --index)
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_index: bool,

    /// Include glob pattern for files (used with --index)
    #[arg(long, value_name = "PATTERN")]
    pub include: Option<String>,

    /// Exclude glob pattern for files (used with --index)
    #[arg(long, value_name = "PATTERN")]
    pub exclude: Option<String>,
}
