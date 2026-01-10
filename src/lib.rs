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
//! use fstsed::fstsed::{FstSed, FstCache};
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
//! - [`fstsed::FstSed`] - The main search engine wrapping an FST database
//! - [`fstsed::FstCache`] - Reusable buffers for search operations
//! - [`fstsed::FstMatch`] - A single match with key, value, and template rendering
//! - [`build::build_fstsed`] - Function to build FST databases from JSON input
//! - [`sink::FstSedSink`] - Integration with `grep-searcher` for line-by-line processing
//!
//! ## Building an FST Database
//!
//! FST databases are built from newline-delimited JSON (NDJSON) files where each line
//! contains a JSON object with at least a key field:
//!
//! ```json
//! {"key": "hello", "translation": "world", "category": "greeting"}
//! {"key": "goodbye", "translation": "farewell", "category": "parting"}
//! ```
//!
//! The key field becomes the searchable term, and the entire JSON line is stored
//! (zstd-compressed) as the value, allowing template access to any field.

pub mod build;
pub mod cli;
pub mod error;
pub mod fstsed;
pub mod jsonquotes;
pub mod sink;

/// Default buffer size for I/O operations (64 KiB).
pub const BUFFERSIZE: usize = 64 * 1024;

/// Sentinel byte used to separate keys from values in the FST.
///
/// This must be 0 to ensure proper lexicographic ordering, as the sentinel
/// marks the boundary between the searchable key and the compressed value
/// payload stored in the FST.
pub const SENTINEL: u8 = 0;

/// Compile-time lookup table for ASCII word characters.
///
/// Word characters are: `a-z`, `A-Z`, `0-9`, `_`
///
/// This matches ripgrep's `utf8::is_word_byte` implementation for consistency
/// with familiar regex `\w` semantics.
const IS_WORD_BYTE_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    // underscore
    table[b'_' as usize] = true;
    // 0-9
    let mut b = b'0';
    while b <= b'9' {
        table[b as usize] = true;
        b += 1;
    }
    // A-Z
    b = b'A';
    while b <= b'Z' {
        table[b as usize] = true;
        b += 1;
    }
    // a-z
    b = b'a';
    while b <= b'z' {
        table[b as usize] = true;
        b += 1;
    }
    table
};

/// Returns `true` if the given byte is an ASCII word character.
///
/// Word characters are: `a-z`, `A-Z`, `0-9`, `_`
///
/// This is the hot-path function used in the main search loop for
/// word boundary detection. It uses a compile-time lookup table for
/// O(1) performance.
///
/// # Examples
///
/// ```
/// use fstsed::is_word_byte;
///
/// assert!(is_word_byte(b'a'));
/// assert!(is_word_byte(b'Z'));
/// assert!(is_word_byte(b'5'));
/// assert!(is_word_byte(b'_'));
///
/// assert!(!is_word_byte(b' '));
/// assert!(!is_word_byte(b'.'));
/// assert!(!is_word_byte(b'-'));
/// ```
#[inline]
#[must_use]
pub fn is_word_byte(b: u8) -> bool {
    IS_WORD_BYTE_TABLE[b as usize]
}

/// Returns `true` if the given character is a word character.
///
/// For ASCII characters (code points < 128), this uses the fast lookup table
/// via [`is_word_byte`]. For Unicode characters, it falls back to
/// [`char::is_alphanumeric`].
///
/// # Examples
///
/// ```
/// use fstsed::is_word_char;
///
/// // ASCII word characters
/// assert!(is_word_char('a'));
/// assert!(is_word_char('Z'));
/// assert!(is_word_char('0'));
/// assert!(is_word_char('_'));
///
/// // Unicode alphanumeric characters
/// assert!(is_word_char('é'));
/// assert!(is_word_char('京'));
/// assert!(is_word_char('α'));
///
/// // Non-word characters
/// assert!(!is_word_char(' '));
/// assert!(!is_word_char('!'));
/// assert!(!is_word_char('.'));
/// assert!(!is_word_char('-'));
/// ```
#[inline]
#[must_use]
pub fn is_word_char(c: char) -> bool {
    if (c as u32) < 128 {
        IS_WORD_BYTE_TABLE[c as usize]
    } else {
        c.is_alphanumeric()
    }
}
