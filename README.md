<p align="right"><strong>English</strong> / <a href="README_ja.md">日本語</a></p>

<h1 align="center">📃🔍 ftgrep</h1>

<p align="center"><b>grep-like instant full-text search with automated indexing</b></p>

> [!WARNING]
> This is an experimental implementation. Expect bugs and missing features.

ftgrep is a full-text search tool for the terminal, built on [tantivy](https://github.com/quickwit-oss/tantivy). It supports line-level and file-level search, persistent indexes with automatic incremental sync, and multiple languages including Japanese (via morphological analysis).

## Why ftgrep?

- **Automated indexing** — full-text search instantly; mtime-based incremental indexing runs automatically before each search
- **Multi-language support by default** — morphological analysis (Japanese/IPADIC) and bigram fallback for broad coverage
- **Search not just by body, but also by frontmatter / filename** — no setup required for markdown document search
- **grep-compatible interface** — `path:line:content` format, colorized matches in TTY, `--json` for scripting
- **Line and file modes** — line mode for grep-style results, file mode for document search with frontmatter support
- **Flexible targeting** — files, directories, or stdin; mix freely in one command

## 🚀 Quick Start

### 1. Install

```bash
# Homebrew (macOS/Linux)
brew install amaya382/tap/ftgrep
```

```bash
# GitHub Release (download binary)
# Example for Linux (x86_64)
curl -L https://github.com/amaya382/ftgrep/releases/latest/download/ftgrep_Linux_x86_64.tar.gz | tar xz
sudo mv ftgrep_Linux_x86_64/ftgrep /usr/local/bin/

# Example for macOS (Apple Silicon)
curl -L https://github.com/amaya382/ftgrep/releases/latest/download/ftgrep_Darwin_arm64.tar.gz | tar xz
sudo mv ftgrep_Darwin_arm64/ftgrep /usr/local/bin/
```

See [Releases](https://github.com/amaya382/ftgrep/releases) for all available binaries.

### Search with on-demand indexing

```bash
# Search a file directly
ftgrep "走る" ./notes.md

# Search a directory (indexes on first run, syncs on subsequent runs)
ftgrep "rust" ./src

# Search stdin
cat README.md | ftgrep "search"
```

### Build a persistent index

```bash
# Index a directory (both line and file mode by default)
ftgrep --index my-notes ~/notes

# Index in a specific mode only
ftgrep --index my-notes --by-line ~/notes
ftgrep --index my-notes --by-file ~/notes

# Search using the index
ftgrep "形態素" ~/notes
ftgrep "形態素" --by-file ~/notes
```

## 📖 Usage

```
ftgrep [OPTIONS] [QUERY] [PATHS]...
```

`QUERY` is the search term. `PATHS` can be files, directories, or omitted (defaults to current directory). When stdin is piped and no paths are given, stdin is searched.

### Search Options

| Option                      | Description                                 |
| --------------------------- | ------------------------------------------- |
| `--by-line`                 | Line mode — one document per line (default) |
| `--by-file`                 | File mode — one document per file           |
| `-m N`, `--limit N`         | Maximum results (default: 100)              |
| `--sort score\|path\|mtime` | Sort order (default: `score`)               |
| `--json`                    | JSON output                                 |

### Exit Codes

| Code | Meaning                   |
| ---- | ------------------------- |
| `0`  | One or more results found |
| `1`  | No results                |
| `2+` | Error                     |

## 📂 Index Management

### `--index <name> <dir>...`

Create or update a named index over one or more directories. By default, both line and file mode indexes are created. Use `--by-line` or `--by-file` to build only one.

```bash
ftgrep --index my-notes ~/notes ~/drafts   # both modes
ftgrep --index my-notes --by-line ~/notes  # line mode only
ftgrep --index my-notes --by-file ~/notes  # file mode only

# Control which files are indexed
ftgrep --index src-index --include "*.rs" --exclude "target/**" .

# Register without indexing yet
ftgrep --index my-notes --no-index ~/notes

# Use a custom index path (not registered globally)
ftgrep --index my-notes --index-path /tmp/my-idx ~/notes --no-register
```

Indexes are stored at:
- Line mode: `$XDG_DATA_HOME/ftgrep/indices/line/<name>/` (default: `~/.local/share/ftgrep/indices/line/<name>/`)
- File mode: `$XDG_DATA_HOME/ftgrep/indices/file/<name>/`

### `--sync <name>`

Manually sync an index (detect added, modified, and deleted files).

```bash
ftgrep --sync my-notes           # Sync all modes
ftgrep --sync my-notes --by-line # Sync line mode only
```

> Sync also runs automatically before every directory search.

### `--list`

List all registered indexes.

```bash
ftgrep --list
```

```
NAME        MODE  DIRS                        LAST SYNC
my-notes    line  ~/notes, ~/drafts           2026-05-10 12:34
my-docs     file  ~/docs                      2026-05-09 08:00
```

### `--info <name>`

Show index details.

```bash
ftgrep --info my-notes
```

## 🔍 Query Syntax

ftgrep uses [tantivy's query syntax](https://docs.rs/tantivy/latest/tantivy/query/struct.QueryParser.html).

| Syntax     | Example              | Matches                  |
| ---------- | -------------------- | ------------------------ |
| Plain term | `走る`               | 走る, 走った, 走っていた |
| Phrase     | `"full text search"` | exact phrase             |
| AND        | `rust +search`       | both terms required      |
| OR         | `rust search`        | either term              |
| NOT        | `rust -python`       | rust without python      |

## 🌐 Language Support

ftgrep applies two analyzers in parallel and merges results:

**Japanese analyzer** — morphological analysis via lindera + IPADIC. Lemmatizes to base form (走っていた → 走る), removes function words. Optimized for Japanese text.

**Bigram analyzer** — overlapping 2-character n-grams. Covers unknown words, symbols, compound nouns, and partial matches across all languages.

**Normalization** (applied to both):

1. Unicode NFKC — full-width to half-width, composed characters
2. Lowercase — case-insensitive ASCII search
3. Katakana → hiragana — コンピュータ matches こんぴゅーた
4. Long vowel removal — コンピューター matches コンピュータ

## 📁 File Handling

- **Binary detection**: files with a NUL byte (`\0`) in the first 8 KB are skipped
- **Frontmatter** (file mode only): YAML frontmatter in `.md`/`.markdown` files is indexed as searchable fields (`frontmatter.*`)
- **Parallel directory traversal**: powered by rayon

### 🗂 Registry

The registry maps index names to their paths:

```
$XDG_CONFIG_HOME/ftgrep/registry.json
(default: ~/.config/ftgrep/registry.json)
```

```json
{
  "indices": {
    "my-notes": "/home/user/.local/share/ftgrep/indices/line/my-notes",
    "my-docs": "/path/to/custom/docs-index"
  }
}
```

Index names that begin with `/` or `.` are treated as paths and bypass the registry.

## 🛠 Development

### Build from source

```bash
cargo build --release
# Binary: target/release/ftgrep
```

### Testing

#### Run all tests

```bash
# Unit tests + e2e tests
cargo test

# Unit tests only (fast, no binary required)
cargo test --lib

# e2e tests only (builds the binary first)
cargo test --test e2e
```

#### Test structure

| Type       | Location                                               | Description                                                                    |
| ---------- | ------------------------------------------------------ | ------------------------------------------------------------------------------ |
| Unit tests | `src/analyzer.rs`, `src/indexer.rs`, `src/searcher.rs` | Tests for individual functions: normalization, glob matching, indexing, search |
| e2e tests  | `tests/e2e.rs`                                         | Tests that invoke the compiled `ftgrep` binary end-to-end                      |
| Fixtures   | `tests/fixtures/docs/`                                 | Sample documents (Japanese and English) used by tests                          |

CI runs both test suites on every pull request (Linux and macOS).

### 🔧 Technical Stack

- [tantivy](https://github.com/quickwit-oss/tantivy) — full-text search engine
- [lindera](https://github.com/lindera-morphology/lindera) + IPADIC — Japanese morphological analysis
- [rayon](https://github.com/rayon-rs/rayon) — parallel directory traversal
- [clap](https://github.com/clap-rs/clap) — CLI argument parsing
- [serde](https://github.com/serde-rs/serde) / serde_json — serialization
- [walkdir](https://github.com/BurntSushi/walkdir) — directory traversal
- [unicode-normalization](https://github.com/unicode-rs/unicode-normalization) — NFKC normalization
