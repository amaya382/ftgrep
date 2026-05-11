use tantivy::tokenizer::{
    BoxTokenStream, LowerCaser, NgramTokenizer, TextAnalyzer, Token, TokenFilter, TokenStream,
    Tokenizer,
};
use unicode_normalization::UnicodeNormalization;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::token_filter::japanese_base_form::JapaneseBaseFormTokenFilter;
use lindera::token_filter::japanese_stop_tags::JapaneseStopTagsTokenFilter;
use lindera_tantivy::tokenizer::LinderaTokenizer;

pub const TOKENIZER_JAPANESE: &str = "ftgrep_japanese";
pub const TOKENIZER_BIGRAM: &str = "ftgrep_bigram";

/// Normalize text: NFKC → lowercase → katakana-to-hiragana → long vowel removal.
pub fn normalize(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    let lower = nfkc.to_lowercase();
    let hira = kata_to_hira(&lower);
    normalize_long_vowel(&hira)
}

fn kata_to_hira(s: &str) -> String {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            // Katakana full-width U+30A1–U+30F6 → Hiragana U+3041–U+3096
            if (0x30A1..=0x30F6).contains(&cp) {
                char::from_u32(cp - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Remove long vowel marks (ー U+30FC) to normalize e.g. コンピューター ≡ コンピュータ.
fn normalize_long_vowel(s: &str) -> String {
    s.chars().filter(|&c| c != '\u{30FC}').collect()
}

// ---- NormalizeFilter token filter ----

/// Token filter that applies our normalization to each token's text.
#[derive(Clone)]
pub struct NormalizeFilter;

impl TokenFilter for NormalizeFilter {
    type Tokenizer<T: Tokenizer> = NormalizeFilterWrapper<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> Self::Tokenizer<T> {
        NormalizeFilterWrapper { inner: tokenizer }
    }
}

#[derive(Clone)]
pub struct NormalizeFilterWrapper<T> {
    inner: T,
}

impl<T: Tokenizer> Tokenizer for NormalizeFilterWrapper<T> {
    type TokenStream<'a> = NormalizeFilterStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        NormalizeFilterStream {
            tail: BoxTokenStream::new(self.inner.token_stream(text)),
        }
    }
}

pub struct NormalizeFilterStream<'a> {
    tail: BoxTokenStream<'a>,
}

impl<'a> TokenStream for NormalizeFilterStream<'a> {
    fn advance(&mut self) -> bool {
        if !self.tail.advance() {
            return false;
        }
        let text = self.tail.token().text.clone();
        self.tail.token_mut().text = normalize(&text);
        true
    }

    fn token(&self) -> &Token {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut Token {
        self.tail.token_mut()
    }
}

// ---- Analyzer builders ----

/// IPADIC part-of-speech tags to remove from the Japanese token stream.
/// Covers particles, auxiliary verbs, punctuation, and other function words
/// that add noise without contributing to search relevance.
const STOP_TAGS: &[&str] = &[
    "接続詞",
    "助詞",
    "助詞,格助詞",
    "助詞,格助詞,一般",
    "助詞,格助詞,引用",
    "助詞,格助詞,連語",
    "助詞,係助詞",
    "助詞,副助詞",
    "助詞,間投助詞",
    "助詞,並立助詞",
    "助詞,終助詞",
    "助詞,副助詞／並立助詞／終助詞",
    "助詞,連体化",
    "助詞,副詞化",
    "助詞,特殊",
    "助動詞",
    "記号",
    "記号,一般",
    "記号,読点",
    "記号,句点",
    "記号,空白",
    "記号,括弧閉",
    "その他,間投",
    "フィラー",
    "非言語音",
];

/// Build the Japanese analyzer: Lindera IPADIC + base form lemmatization + stop tags + NormalizeFilter + LowerCaser.
pub fn build_japanese_analyzer() -> anyhow::Result<TextAnalyzer> {
    let dictionary = load_dictionary("embedded://ipadic")
        .map_err(|e| anyhow::anyhow!("Failed to load IPADIC: {}", e))?;
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let mut tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    // Lemmatize verbs/adjectives to their dictionary base form (e.g. 走っていた → 走る).
    tokenizer.append_token_filter(JapaneseBaseFormTokenFilter::new().into());
    // Remove function words (particles, auxiliary verbs, punctuation) that don't carry search meaning.
    let stop_tags = STOP_TAGS.iter().map(|s| s.to_string()).collect();
    tokenizer.append_token_filter(JapaneseStopTagsTokenFilter::new(stop_tags).into());
    Ok(TextAnalyzer::builder(tokenizer)
        .filter(NormalizeFilter)
        .filter(LowerCaser)
        .build())
}

/// Build the bigram analyzer: NgramTokenizer(2,2) + NormalizeFilter + LowerCaser.
pub fn build_bigram_analyzer() -> anyhow::Result<TextAnalyzer> {
    let tokenizer = NgramTokenizer::all_ngrams(2, 2)
        .map_err(|e| anyhow::anyhow!("Failed to create NgramTokenizer: {}", e))?;
    Ok(TextAnalyzer::builder(tokenizer)
        .filter(NormalizeFilter)
        .filter(LowerCaser)
        .build())
}

/// Register both custom analyzers on the given index.
pub fn register_analyzers(index: &tantivy::Index) -> anyhow::Result<()> {
    index
        .tokenizers()
        .register(TOKENIZER_JAPANESE, build_japanese_analyzer()?);
    index
        .tokenizers()
        .register(TOKENIZER_BIGRAM, build_bigram_analyzer()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::TokenStream;

    fn tokenize(mut analyzer: tantivy::tokenizer::TextAnalyzer, text: &str) -> Vec<String> {
        let mut stream = analyzer.token_stream(text);
        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        tokens
    }

    #[test]
    fn test_normalize_nfkc() {
        assert_eq!(normalize("ＲＵＳＴ"), "rust");
    }

    #[test]
    fn test_normalize_lowercase() {
        assert_eq!(normalize("Rust"), "rust");
    }

    #[test]
    fn test_normalize_kata_to_hira() {
        assert_eq!(normalize("コンピュータ"), "こんぴゅた");
    }

    #[test]
    fn test_normalize_long_vowel() {
        // ー (U+30FC) is removed
        assert_eq!(normalize("コンピューター"), "こんぴゅた");
    }

    #[test]
    fn test_japanese_analyzer_base_form() {
        let analyzer = build_japanese_analyzer().unwrap();
        let tokens = tokenize(analyzer, "走っていた");
        assert!(tokens.contains(&"走る".to_string()), "expected base form '走る' in {:?}", tokens);
    }

    #[test]
    fn test_bigram_analyzer_basic() {
        let analyzer = build_bigram_analyzer().unwrap();
        let tokens = tokenize(analyzer, "全文検索");
        // bigram: 全文, 文検, 検索
        assert!(tokens.contains(&"全文".to_string()), "expected '全文' in {:?}", tokens);
        assert!(tokens.contains(&"検索".to_string()), "expected '検索' in {:?}", tokens);
    }

    #[test]
    fn test_bigram_analyzer_ascii() {
        let analyzer = build_bigram_analyzer().unwrap();
        let tokens = tokenize(analyzer, "rust");
        assert!(tokens.contains(&"ru".to_string()), "expected 'ru' in {:?}", tokens);
        assert!(tokens.contains(&"us".to_string()), "expected 'us' in {:?}", tokens);
    }

}
