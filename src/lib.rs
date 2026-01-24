//! # fstsed
//!
//! `fstsed` is a high-performance text enrichment tool that uses FST (Finite State Transducer)
//! data structures for efficient multi-pattern matching. It's designed for scenarios where you
//! need to perform many simultaneous string decorations across large amounts of text.
//!
//! ## Features
//!
//! - **FST-based matching**: Leverages the `fst` crate for memory-efficient, fast lookups
//! - **Word boundary detection**: Matches only occur at word boundaries (like `\b` in regex)
//! - **Unicode support**: Proper handling of Unicode word characters
//! - **Template rendering**: Customizable output using `{key}`, `{value}`, and JSON field templates
//! - **JSON mode**: Search within JSON string values without deserializing entire documents
//! - **Parallel processing**: Multi-threaded file walking for directory searches
//! - **Streaming builds**: Build FST databases from sorted input with O(1) memory
//!
//! ## Quick Start
//!
//! ```no_run
//! use fstsed::{FstSed, FstCache};
//! use termcolor::ColorChoice;
//! use camino::Utf8PathBuf;
//!
//! // Load an FST database
//! let fsed = FstSed::new(
//!     Utf8PathBuf::from("dictionary.fst"),
//!     Some("{key} -> {value}".to_string()),
//!     ColorChoice::Never,
//! ).expect("Failed to load FST");
//!
//! // Search text
//! let mut cache = FstCache::new();
//! let text = b"Hello world";
//! for match_pos in fsed.find_iter(text, &mut cache) {
//!     println!("Found match at position {}", match_pos);
//! }
//! ```
//!
//! ## Main Types
//!
//! - [`FstSed`] - The main search engine wrapping an FST database
//! - [`FstCache`] - Reusable buffers for search operations
//! - [`FstMatch`] - A single match with key, value, and template rendering
//! - [`builder::build_fstsed`] - Function to build FST databases from JSON input

pub mod builder;
pub mod cli;
pub mod error;
pub mod json;
pub mod matches;
pub mod search;
pub mod template;
mod util;

// Re-export public utilities
pub use matches::{FstMatch, FstMatches};
pub use search::{FstCache, FstSed};
pub use util::{is_word_byte, is_word_char, BUFFERSIZE, SENTINEL};
