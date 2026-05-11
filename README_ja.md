<p align="right"><a href="README.md">English</a> / <strong>日本語</strong></p>

<h1 align="center">📃🔍 ftgrep</h1>

<p align="center"><b>インデキシングが自動な grep ライク即時全文検索ツール</b></p>

> [!WARNING]
> このプロジェクトは実験的な実装です。バグや未実装の機能が多数存在する可能性があります。

ftgrep は [tantivy](https://github.com/quickwit-oss/tantivy) ベースのターミナル向け全文検索ツールです。行単位・ファイル単位の検索、永続インデックスの自動差分 sync、日本語形態素解析を含む多言語対応に対応しています。

## なぜ ftgrep？

- **自動インデキシング** — 即時全文検索可能。mtime ベースの差分 indexing を自動実行
- **デフォルトで多言語対応** — 日本語形態素解析（IPADIC）と bigram フォールバックによる幅広いカバレッジ
- **本文だけでなく frontmatter やファイル名での検索も** — セットアップなしで frontmatter やファイル名での markdown ドキュメント検索も可能
- **grep 互換の出力形式** — `path:行番号:内容`、TTY でのカラーハイライト、`--json` によるスクリプト連携
- **行モード・ファイルモード** — 行モードは grep 風結果、ファイルモードは frontmatter 対応のドキュメント検索
- **柔軟なターゲット指定** — ファイル・ディレクトリ・stdin を自由に混在指定

## 🚀 クイックスタート

### 1. インストール

```bash
# Homebrew (macOS/Linux)
brew install amaya382/tap/ftgrep
```

```bash
# GitHub Release（バイナリをダウンロード）
# Linux (x86_64) の例
curl -L https://github.com/amaya382/ftgrep/releases/latest/download/ftgrep_Linux_x86_64.tar.gz | tar xz
sudo mv ftgrep_Linux_x86_64/ftgrep /usr/local/bin/

# macOS (Apple Silicon) の例
curl -L https://github.com/amaya382/ftgrep/releases/latest/download/ftgrep_Darwin_arm64.tar.gz | tar xz
sudo mv ftgrep_Darwin_arm64/ftgrep /usr/local/bin/
```

利用可能な全バイナリは [Releases](https://github.com/amaya382/ftgrep/releases) を参照。

### オンデマンドインデックスで即時検索

```bash
# ファイルを直接検索
ftgrep "走る" ./notes.md

# ディレクトリ検索（初回はインデックスを自動作成、以降は差分 sync）
ftgrep "rust" ./src

# stdin を検索
cat README.md | ftgrep "search"
```

### 永続インデックスを作成して検索

```bash
# ディレクトリをインデックス登録（デフォルトで行・ファイル両モード）
ftgrep --index my-notes ~/notes

# モードを指定して登録
ftgrep --index my-notes --by-line ~/notes
ftgrep --index my-notes --by-file ~/notes

# インデックスを使って検索
ftgrep "形態素" ~/notes
ftgrep "形態素" --by-file ~/notes
```

## 📖 使い方

```
ftgrep [OPTIONS] [QUERY] [PATHS]...
```

`QUERY` は検索クエリ。`PATHS` にはファイル・ディレクトリを混在指定可能（省略時はカレントディレクトリ）。stdin にデータが渡されパス指定がない場合は stdin を検索。

### 検索オプション

| オプション                  | 説明                                         |
| --------------------------- | -------------------------------------------- |
| `--by-line`                 | 行モード — 1ドキュメント = 1行（デフォルト） |
| `--by-file`                 | ファイルモード — 1ドキュメント = 1ファイル   |
| `-m N`, `--limit N`         | 最大件数（デフォルト: 100）                  |
| `--sort score\|path\|mtime` | ソート順（デフォルト: `score`）              |
| `--json`                    | JSON 形式で出力                              |

### 終了コード

| コード | 意味          |
| ------ | ------------- |
| `0`    | 1件以上ヒット |
| `1`    | ヒットなし    |
| `2+`   | エラー        |

## 📂 インデックス管理

### `--index <name> <dir>...`

名前付きインデックスを作成・更新する。デフォルトでは行モード・ファイルモードの両方のインデックスを作成する。`--by-line` または `--by-file` を指定するとそのモードのみ作成する。

```bash
ftgrep --index my-notes ~/notes ~/drafts   # 両モード
ftgrep --index my-notes --by-line ~/notes  # 行モードのみ
ftgrep --index my-notes --by-file ~/notes  # ファイルモードのみ

# インデキシング対象ファイルを制御
ftgrep --index src-index --include "*.rs" --exclude "target/**" .

# ディレクトリ登録のみ（インデキシングは後で）
ftgrep --index my-notes --no-index ~/notes

# カスタムパスに保存（レジストリ登録なし）
ftgrep --index my-notes --index-path /tmp/my-idx ~/notes --no-register
```

インデックスの保存先:
- 行モード: `$XDG_DATA_HOME/ftgrep/indices/line/<name>/`（デフォルト: `~/.local/share/ftgrep/indices/line/<name>/`）
- ファイルモード: `$XDG_DATA_HOME/ftgrep/indices/file/<name>/`

### `--sync <name>`

インデックスを手動で sync する（追加・変更・削除を検出して反映）。

```bash
ftgrep --sync my-notes           # 全モード sync
ftgrep --sync my-notes --by-line # 行モードのみ sync
```

> ディレクトリ検索時は毎回自動で差分 sync が実行される。

### `--list`

登録済みインデックスの一覧を表示する。

```bash
ftgrep --list
```

```
NAME        MODE  DIRS                        LAST SYNC
my-notes    line  ~/notes, ~/drafts           2026-05-10 12:34
my-docs     file  ~/docs                      2026-05-09 08:00
```

### `--info <name>`

インデックスの詳細情報を表示する。

```bash
ftgrep --info my-notes
```

## 🔍 クエリ構文

ftgrep は [tantivy のクエリ構文](https://docs.rs/tantivy/latest/tantivy/query/struct.QueryParser.html) を使用する。

| 構文     | 例                   | マッチ                        |
| -------- | -------------------- | ----------------------------- |
| 単語     | `走る`               | 走る, 走った, 走っていた      |
| フレーズ | `"full text search"` | 完全一致フレーズ              |
| AND      | `rust +search`       | 両方の語を含む                |
| OR       | `rust search`        | どちらかの語を含む            |
| NOT      | `rust -python`       | rust を含み python を含まない |

## 🌐 言語サポート

2種類の analyzer を並行して適用し、結果をマージする。

**Japanese analyzer** — lindera + IPADIC による形態素解析。原形への lemmatization（走っていた → 走る）と機能語除去を行う。日本語テキスト専用。

**Bigram analyzer** — 2文字 n-gram による部分一致。未知語・記号・複合語・全言語の部分一致に対応。

**正規化**（両 analyzer 共通）:

1. Unicode NFKC — 全角英数字→半角、合成文字の正規化
2. 小文字化 — 英字の大文字・小文字を統一
3. カタカナ→ひらがな変換 — コンピュータ ≡ こんぴゅーた
4. 長音符の正規化 — コンピューター ≡ コンピュータ

## 📁 ファイル処理

- **バイナリ判定**: 先頭 8KB に NUL バイト（`\0`）が含まれるファイルはスキップ
- **frontmatter**（ファイルモードのみ）: `.md`/`.markdown` の YAML frontmatter を検索可能フィールド（`frontmatter.*`）としてインデックス
- **並列ディレクトリ走査**: rayon による高速走査

### 🗂 レジストリ

インデックス名とパスのマッピングを保持する。

```
$XDG_CONFIG_HOME/ftgrep/registry.json
（デフォルト: ~/.config/ftgrep/registry.json）
```

```json
{
  "indices": {
    "my-notes": "/home/user/.local/share/ftgrep/indices/line/my-notes",
    "my-docs": "/path/to/custom/docs-index"
  }
}
```

`/` または `.` で始まる名前はパスとして扱われ、レジストリをバイパスする。

## 🛠 開発

### ソースからビルド

```bash
cargo build --release
# バイナリ: target/release/ftgrep
```

### テスト

#### テストを実行する

```bash
# unit test + e2e test をまとめて実行
cargo test

# unit test のみ（高速・バイナリ不要）
cargo test --lib

# e2e test のみ（先にバイナリをビルドしてから実行）
cargo test --test e2e
```

#### テスト構成

| 種別       | 場所                                                   | 内容                                                                |
| ---------- | ------------------------------------------------------ | ------------------------------------------------------------------- |
| Unit tests | `src/analyzer.rs`, `src/indexer.rs`, `src/searcher.rs` | 正規化・glob マッチング・インデックス構築・検索などの関数単体テスト |
| e2e tests  | `tests/e2e.rs`                                         | コンパイルした `ftgrep` バイナリをエンドツーエンドで呼び出すテスト  |
| Fixtures   | `tests/fixtures/docs/`                                 | テストで使用するサンプルドキュメント（日本語・英語）                |

PR ごとに CI が unit test・e2e test の両方を Linux と macOS で自動実行します。

### 🔧 技術スタック

- [tantivy](https://github.com/quickwit-oss/tantivy) — 全文検索エンジン
- [lindera](https://github.com/lindera-morphology/lindera) + IPADIC — 日本語形態素解析
- [rayon](https://github.com/rayon-rs/rayon) — 並列ディレクトリ走査
- [clap](https://github.com/clap-rs/clap) — CLI 引数解析
- [serde](https://github.com/serde-rs/serde) / serde_json — シリアライズ
- [walkdir](https://github.com/BurntSushi/walkdir) — ディレクトリ走査
- [unicode-normalization](https://github.com/unicode-rs/unicode-normalization) — NFKC 正規化
