use tantivy::schema::{
    IndexRecordOption, JsonObjectOptions, Schema, SchemaBuilder, TextFieldIndexing, TextOptions,
    STORED, STRING,
};

use crate::analyzer::{TOKENIZER_BIGRAM, TOKENIZER_JAPANESE};

fn text_field_indexed(tokenizer: &str) -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

fn text_field_stored(tokenizer: &str) -> TextOptions {
    TextOptions::default()
        .set_stored()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(tokenizer)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
}

/// Build tantivy schema for line mode.
pub fn line_schema() -> Schema {
    let mut builder = SchemaBuilder::new();
    builder.add_text_field("path", STRING | STORED);
    builder.add_u64_field("line_number", tantivy::schema::STORED);
    // body_japanese is stored so we can retrieve the original line text for display
    builder.add_text_field("body_japanese", text_field_stored(TOKENIZER_JAPANESE));
    builder.add_text_field("body_bigram", text_field_indexed(TOKENIZER_BIGRAM));
    builder.build()
}

/// Build tantivy schema for file mode.
/// `frontmatter` is a dynamic JSON field whose sub-keys are indexed individually.
/// Both plain queries and field-qualified queries (`frontmatter.title:...`) work because
/// QueryParser treats a JSON field in default_fields as matching all sub-keys.
pub fn file_schema() -> Schema {
    let mut builder = SchemaBuilder::new();
    builder.add_text_field("path", STRING | STORED);
    builder.add_text_field("filename", STRING | STORED);
    // body_japanese is stored so we can retrieve snippet text for display
    builder.add_text_field("body_japanese", text_field_stored(TOKENIZER_JAPANESE));
    builder.add_text_field("body_bigram", text_field_indexed(TOKENIZER_BIGRAM));
    // filename_bigram enables partial/bigram matching on the filename
    builder.add_text_field("filename_bigram", text_field_indexed(TOKENIZER_BIGRAM));
    // frontmatter.* — japanese analyzer variant (stored for retrieval)
    let json_opts_japanese = JsonObjectOptions::default()
        .set_stored()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(TOKENIZER_JAPANESE)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
    builder.add_json_field("frontmatter", json_opts_japanese);
    // frontmatter_bigram — bigram variant of frontmatter (not stored; index-only)
    let json_opts_bigram = JsonObjectOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER_BIGRAM)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    builder.add_json_field("frontmatter_bigram", json_opts_bigram);
    builder.build()
}
