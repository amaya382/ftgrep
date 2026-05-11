pub mod analyzer;
pub mod highlight;
pub mod indexer;
pub mod meta;
pub mod registry;
pub mod schema;
pub mod searcher;

// cli と cmd は main.rs 側でのみ使う
pub mod cli;
pub mod cmd;
