//! FST-based search engine for text replacement.
//!
//! This module provides the core [`FstSed`] type for performing efficient
//! multi-pattern matching using Finite State Transducers.

use crate::error::Result;
use crate::template::{render_fmt, Context};
use camino::Utf8Path;
use fst::raw::Fst;
use memmap2::Mmap;
use serde_json::Value;
use std::borrow::Cow;
use termcolor::{Color, ColorChoice, ColorSpec, WriteColor};

use bstr::ByteSlice;

use crate::json::jsonquotes_range_iter;
use crate::util::{is_word_byte, is_word_char, mmap_fst, BUFFERSIZE, SENTINEL};
use bstr::io::BufReadExt;
use std::io::{self, BufRead, Write};

/// Represents a single match of an FST key in a haystack
/// with its corresponding value from the FST.
pub struct FstMatch<'a> {
    key: Cow<'a, str>,
    value: Cow<'a, str>,
    template: &'a str,
    jsonvalue: Option<Value>,
}

impl FstMatch<'_> {
    /// Returns a displayable object that renders the match.
    ///
    /// This uses `std::fmt::from_fn` to allow rendering directly to a formatter
    /// without intermediate allocation.
    #[must_use]
    pub fn display(&self) -> impl std::fmt::Display + '_ {
        std::fmt::from_fn(move |f| {
            // fast path for default template
            if self.template == "<{key}|{value}>" {
                write!(f, "<{}|{}>", self.key, self.value)
            } else {
                render_fmt(self.template, self, f)
            }
        })
    }
}

impl Context for &FstMatch<'_> {
    fn get_field(&self, field_name: &str) -> &str {
        match field_name {
            "key" => self.key.as_ref(),
            "value" => self.value.as_ref(),
            _ => {
                self.jsonvalue
                    .as_ref()
                    .and_then(|jv| {
                        if field_name.starts_with('/') {
                            // Field name starts with "/", use JSON pointer
                            jv.pointer(field_name)
                        } else {
                            // Just regular field retrieval
                            jv.get(field_name)
                        }
                    })
                    .and_then(Value::as_str)
                    .unwrap_or("")
            }
        }
    }
}

pub struct FstMatches<'f, 'c, 'a> {
    fstsed: &'f FstSed,
    haystack: &'a [u8],
    cache: &'c mut FstCache,
    /// Current scan position in the haystack
    pos: usize,
}

impl<'f, 'c, 'a> FstMatches<'f, 'c, 'a> {
    pub fn new(fstsed: &'f FstSed, haystack: &'a [u8], cache: &'c mut FstCache) -> Self {
        Self {
            fstsed,
            haystack,
            cache,
            pos: 0,
        }
    }

    /// Returns the current match with its key, value, and template.
    ///
    /// This method decompresses the stored value and constructs an [`FstMatch`]
    /// that can be used for rendering.
    ///
    /// # Note
    ///
    /// If decompression fails, the value will be empty. This can happen if
    /// the FST database is corrupted.
    pub fn get_match(&mut self) -> FstMatch<'_> {
        // Decompress the value into scratch buffer
        self.cache.scratch.clear();
        if let Err(e) =
            zstd::stream::copy_decode(self.cache.value.as_slice(), &mut self.cache.scratch)
        {
            eprintln!(
                "Warning: failed to decompress value for key '{}': {e}",
                String::from_utf8_lossy(&self.cache.key)
            );
        }

        FstMatch {
            key: String::from_utf8_lossy(&self.cache.key),
            value: String::from_utf8_lossy(&self.cache.scratch),
            template: &self.fstsed.template,
            jsonvalue: if self.fstsed.has_json_keys {
                Some(
                    serde_json::from_slice(&self.cache.scratch)
                        .unwrap_or_else(|_| Value::default()),
                )
            } else {
                None
            },
        }
    }

    /// Returns the length of the matched key in bytes.
    #[must_use]
    pub fn get_match_len(&self) -> usize {
        self.cache.key.len()
    }

    /// Check if position `pos` is a valid word-start boundary.
    ///
    /// Returns `true` if:
    /// - `pos == 0` (start of input), OR
    /// - the character at `pos-1` is NOT a word character
    #[inline]
    fn is_word_start_boundary(&self, pos: usize) -> bool {
        if pos == 0 {
            return true;
        }
        let prev = self.haystack[pos - 1];
        if prev < 128 {
            // ASCII fast path
            !is_word_byte(prev)
        } else {
            // Unicode fallback: decode the previous character
            self.haystack[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word_char(c))
        }
    }
}

impl Iterator for FstMatches<'_, '_, '_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let haystack = self.haystack;
        let len = haystack.len();

        while self.pos < len {
            let start_pos = self.pos;
            let current_byte = haystack[start_pos];
            self.pos += 1;

            // FAST: Single array lookup (256 bytes easily fits in L1 cache).
            // Reject bytes that can never be the start of any key in the FST.
            // This eliminates ~90% of positions without any FST traversal.
            if !self.fstsed.start_byte_table[current_byte as usize] {
                continue;
            }

            // MEDIUM: Boundary check (only for bytes that could start a key).
            // A word can start here if:
            // 1. We're at position 0, OR
            // 2. The previous byte is not a word character
            if !self.is_word_start_boundary(start_pos) {
                continue;
            }

            // SLOW: Full FST walk (only for potential matches at word boundaries).
            if self
                .fstsed
                .longest_match_at(haystack, start_pos, self.cache)
                .is_some()
            {
                let match_start = self.cache.start;
                let match_end = match_start + self.cache.key.len();

                // Skip past the match to avoid overlapping matches
                self.pos = match_end;

                return Some(match_start);
            }
        }
        None
    }
}

/// Holds reusable buffers for search operations.
///
/// Creating an `FstCache` once and reusing it across multiple searches
/// avoids repeated memory allocations.
pub struct FstCache {
    key: Vec<u8>,
    value: Vec<u8>,
    scratch: Vec<u8>,
    start: usize,
}

impl FstCache {
    /// Creates a new cache with pre-allocated buffers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: Vec::with_capacity(256),
            value: Vec::with_capacity(2048),
            scratch: Vec::with_capacity(2048),
            start: 0,
        }
    }
}

impl Default for FstCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The main FST-based search engine.
///
/// `FstSed` wraps a memory-mapped FST database and provides methods for
/// finding matches in text with word boundary detection.
pub struct FstSed {
    fst: Fst<Mmap>,
    /// Color choice for output formatting.
    pub color: ColorChoice,
    /// Template string for rendering matches.
    pub template: String,
    has_json_keys: bool,
    /// Lookup table for valid starting bytes. If `start_byte_table[b]` is false,
    /// byte `b` cannot be the first byte of any key in the FST.
    start_byte_table: [bool; 256],
    /// Color specification for highlighting matches.
    color_spec: ColorSpec,
}

/// Determines if a template string requires JSON deserialization.
///
/// Returns `true` if the template contains template variables beyond
/// `{key}` and `{value}`, indicating that JSON field access is needed.
fn test_for_json_keys(template: &str) -> bool {
    template
        .split('{')
        .skip(1)
        .any(|c| !(c.starts_with("key}") || c.starts_with("value}")))
}

impl<'a> FstSed {
    /// Creates a new `FstSed` instance from an FST file path.
    ///
    /// # Arguments
    ///
    /// * `fstpath` - Path to the FST database file
    /// * `user_template` - Optional template string for rendering matches
    /// * `color` - Color choice for output formatting
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The FST file cannot be opened
    /// - The FST file is invalid or corrupted
    ///
    /// # Safety
    ///
    /// This function uses memory-mapped I/O internally. The caller must ensure
    /// that the FST file is not modified while the `FstSed` instance is in use.
    pub fn new(
        fstpath: impl AsRef<Utf8Path>,
        user_template: Option<String>,
        color: ColorChoice,
    ) -> Result<Self> {
        let fstpath = fstpath.as_ref();
        let template = user_template.unwrap_or_else(|| "<{key}|{value}>".to_string());
        let has_json_keys = test_for_json_keys(&template);

        let mut color_spec = ColorSpec::new();
        if color != ColorChoice::Never {
            color_spec.set_fg(Some(Color::Red)).set_bold(true);
        }

        // SAFETY: Memory-mapped FST files must not be modified while in use.
        // The caller is responsible for ensuring file stability.
        let fst = unsafe { mmap_fst(fstpath) }?;

        // Build the starting byte lookup table from the FST root node.
        // This allows us to quickly reject bytes that can never start a key.
        let mut start_byte_table = [false; 256];
        let root = fst.root();
        for trans in root.transitions() {
            start_byte_table[trans.inp as usize] = true;
        }

        Ok(Self {
            fst,
            color,
            template,
            has_json_keys,
            start_byte_table,
            color_spec,
        })
    }

    /// Returns an iterator over all matches in the given text.
    ///
    /// Matches are found at word boundaries only, and the longest match
    /// at each position is returned.
    #[inline]
    pub fn find_iter<'f, 'c>(
        &'f self,
        text: &'a [u8],
        cache: &'c mut FstCache,
    ) -> FstMatches<'f, 'c, 'a> {
        FstMatches::new(self, text, cache)
    }

    /// Finds the longest match starting at the given position.
    ///
    /// Returns `Some(length)` if a match was found, storing the match
    /// details in the provided cache. Returns `None` if no match exists.
    ///
    /// Adapted from <https://github.com/BurntSushi/fst/pull/104/files>
    #[inline]
    pub fn longest_match_at(
        &self,
        text: &'a [u8],
        start: usize,
        cache: &mut FstCache,
    ) -> Option<usize> {
        let mut node = self.fst.root();
        let mut last_match_info = None;
        let value = &text[start..];

        // ══════════════════════════════════════════════════════════════════════════════
        // FST Traversal Loop: Find the longest matching key starting at `start`
        // ══════════════════════════════════════════════════════════════════════════════
        //
        // We traverse the FST byte-by-byte, following transitions that match our input.
        // The FST is structured so that each key ends with a SENTINEL byte (0x00),
        // which marks a complete key. When we encounter a sentinel transition, we've
        // found a potential match.
        //
        // Since we want the LONGEST match (e.g., "New York City" over "New York"),
        // we don't return immediately upon finding a match. Instead, we record it
        // and continue traversing to see if a longer key exists in the FST.
        //
        // Example FST structure for keys "app" and "apple":
        //
        //   root ─a─> ─p─> ─p─> ─SENTINEL─> (value for "app")
        //                    └─l─> ─e─> ─SENTINEL─> (value for "apple")
        //
        // When matching "apple pie", we'll find both "app" and "apple" as potential
        // matches, but we'll return "apple" as it's longer. Note that "applesauce"
        // would NOT match "apple" because the word boundary check (below) would
        // fail—the 's' following "apple" is a word character.
        //
        for (i, &b) in value.iter().enumerate() {
            // Try to find a transition from the current node that matches byte `b`.
            //
            // In FST terminology, a "transition" is a labeled edge between nodes.
            // Each transition has:
            //   - An input byte (the label on the edge)
            //   - A target node address (where the edge leads)
            //   - An optional output value (used for storing associated data)
            //
            // For example, if we're at a node after matching "ap" and the next byte
            // is 'p', we look for a transition labeled 'p' to continue to the "app"
            // state. If no such transition exists, the FST has no keys with this prefix.
            let Some(trans_index) = node.find_input(b) else {
                break;
            };

            // Follow the transition to the next node in the FST.
            let t = node.transition(trans_index);
            node = self.fst.node(t.addr);

            // Check if this node has a SENTINEL transition, indicating a complete key.
            // The presence of a sentinel means the bytes we've consumed so far form
            // a valid key in our dictionary.
            if let Some(sentinel_index) = node.find_input(SENTINEL) {
                // ──────────────────────────────────────────────────────────────────
                // Word Boundary Validation (Trailing Edge)
                // ──────────────────────────────────────────────────────────────────
                //
                // To prevent partial word matches, we verify that we're at a word
                // boundary. A match is valid if:
                //   1. We're at the end of the input text, OR
                //   2. The boundary between matched text and following text is a
                //      word boundary (at least one side is non-word character)
                //
                // This prevents matching "apple" inside "applesauce" or "pineapple".
                // The logic mirrors regex word boundaries (\b): a boundary exists
                // where a word character meets a non-word character.
                //
                // Word characters: alphanumeric (Unicode-aware) and underscore
                // Non-word characters: whitespace, punctuation, etc.
                //
                let is_boundary = i == value.len() - 1 || {
                    let next_byte = value[i + 1];

                    if b < 128 && next_byte < 128 {
                        // ┌─────────────────────────────────────────────────────────┐
                        // │ ASCII Fast Path                                         │
                        // │                                                         │
                        // │ When both the last matched byte and the next byte are   │
                        // │ ASCII (< 128), we can check word boundaries directly    │
                        // │ without UTF-8 decoding overhead. This covers the vast   │
                        // │ majority of English text processing.                    │
                        // └─────────────────────────────────────────────────────────┘
                        !is_word_char(b as char) || !is_word_char(next_byte as char)
                    } else {
                        // ┌─────────────────────────────────────────────────────────┐
                        // │ Unicode Fallback Path                                   │
                        // │                                                         │
                        // │ For multi-byte UTF-8 characters, we must decode to get  │
                        // │ the actual Unicode code points. This handles languages  │
                        // │ like Chinese, Japanese, Arabic, Burmese, etc.           │
                        // │                                                         │
                        // │ We extract:                                             │
                        // │   - last_char: final character of the matched portion   │
                        // │   - next_char: first character after the match          │
                        // └─────────────────────────────────────────────────────────┘
                        let last_char = value[..=i].chars().next_back();
                        let next_char = value[i + 1..].chars().next();

                        match (last_char, next_char) {
                            (Some(last), Some(next)) => !is_word_char(last) || !is_word_char(next),
                            // If we can't decode either character, conservatively
                            // treat it as a valid boundary to avoid missing matches.
                            _ => true,
                        }
                    }
                };

                if is_boundary {
                    // Record this as a candidate for the longest match. We store the
                    // node and sentinel index rather than retrieving the value now,
                    // avoiding unnecessary work if we find a longer match later.
                    // The value will be retrieved after the loop completes.
                    last_match_info = Some((node, sentinel_index, i));
                }
            }
        }

        if let Some((match_node, sentinel_index, i)) = last_match_info {
            // We found at least one match, retrieve the one with the longest key.
            cache.key.clear();
            cache.value.clear();
            cache.key.extend_from_slice(&value[..=i]);
            cache.start = start;

            // find the sentinel node, then read to the final node
            // to retrieve the compressed "value" string.
            let sentinel = match_node.transition(sentinel_index);
            let mut snode = self.fst.node(sentinel.addr);
            while !snode.is_final() {
                if let Some(t) = snode.transitions().next() {
                    // after the sentinel, we should not have any more
                    // branching in the fst, so we just grab the first transition
                    cache.value.push(t.inp);
                    snode = self.fst.node(t.addr);
                } else {
                    // somehow ran out of nodes!
                    break;
                }
            }
            return Some(i + 1);
        }
        None
    }

    /// Finds the longest match starting at position 0.
    ///
    /// This is a convenience method equivalent to `longest_match_at(text, 0, cache)`.
    #[inline]
    pub fn longest_match(&self, text: &'a [u8], cache: &mut FstCache) -> Option<usize> {
        self.longest_match_at(text, 0, cache)
    }

    /// Process a stream line-by-line, writing results to output.
    ///
    /// This mirrors aho-corasick's stream_replace_all pattern but
    /// optimized for line-based text processing.
    #[inline]
    pub fn stream_find<R: BufRead, W: WriteColor>(
        &self,
        reader: &mut R,
        writer: &mut W,
        cache: &mut FstCache,
        only_matching: bool,
        json_mode: bool,
    ) -> io::Result<()> {
        let mut json_buf = if json_mode {
            Vec::with_capacity(BUFFERSIZE)
        } else {
            Vec::new()
        };

        reader.for_byte_line_with_terminator(|line| {
            if json_mode {
                self.process_json_line(line, writer, cache, &mut json_buf)?;
            } else if only_matching {
                self.process_matching_only(line, writer, cache)?;
            } else {
                self.process_text_line(line, writer, cache)?;
            }
            Ok(true)
        })
    }

    /// Process a single text line, replacing matches.
    #[inline]
    fn process_text_line(
        &self,
        line: &[u8],
        writer: &mut impl WriteColor,
        cache: &mut FstCache,
    ) -> io::Result<()> {
        let mut last_end = 0;
        let mut matches = self.find_iter(line, cache);

        while let Some(match_start) = matches.next() {
            // Write text before match
            writer.write_all(&line[last_end..match_start])?;

            // Write replacement
            write!(writer, "{}", matches.get_match().display())?;
            writer.reset()?;

            last_end = match_start + matches.get_match_len();
        }

        // Write remaining text
        writer.write_all(&line[last_end..])?;
        Ok(())
    }

    /// Process a line in matching-only mode.
    #[inline]
    fn process_matching_only(
        &self,
        line: &[u8],
        writer: &mut impl WriteColor,
        cache: &mut FstCache,
    ) -> io::Result<()> {
        let mut matches = self.find_iter(line, cache);

        while let Some(_) = matches.next() {
            write!(writer, "{}", matches.get_match().display())?;
            writer.reset()?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Process a JSON line.
    #[inline]
    fn process_json_line(
        &self,
        line: &[u8],
        writer: &mut impl WriteColor,
        cache: &mut FstCache,
        json_buf: &mut Vec<u8>,
    ) -> io::Result<()> {
        let mut lastpos: usize = 0;
        for (start, end, has_esc) in jsonquotes_range_iter(line) {
            // print from last spot to new start
            writer.write_all(&line[lastpos..start])?;

            // CONTENT: line[start+1..end-1]
            let raw_content = &line[start + 1..end - 1];

            if has_esc {
                // SLOW PATH: Use Cow to avoid allocation if possible,
                // though has_esc=true means we'll likely allocate once.
                match serde_json::from_slice::<std::borrow::Cow<str>>(&line[start..end]) {
                    Ok(s) => self.process_json_content(s.as_bytes(), writer, cache, json_buf)?,
                    Err(_) => {
                        writer.write_all(&line[start..end])?;
                    }
                }
            } else {
                // FAST PATH: No escapes to handle!
                // We can run FST directly on the raw bytes from the line.
                self.process_json_content(raw_content, writer, cache, json_buf)?;
            }
            lastpos = end;
        }
        writer.write_all(&line[lastpos..])?;
        Ok(())
    }

    #[inline]
    fn process_json_content(
        &self,
        input: &[u8],
        writer: &mut impl WriteColor,
        cache: &mut FstCache,
        json_buf: &mut Vec<u8>,
    ) -> io::Result<()> {
        writer.write_all(b"\"")?; // Opening quote

        let mut last_end = 0;
        let mut matcher = self.find_iter(input, cache);

        while let Some(m) = matcher.next() {
            // Text before match
            self.write_json_str_fragment(writer, &input[last_end..m], json_buf)?;

            // The match
            // We need to render to a string first for JSON encoding
            let replacement = matcher.get_match().display().to_string();
            writer.set_color(&self.color_spec)?;
            self.write_json_str_fragment(writer, replacement.as_bytes(), json_buf)?;
            writer.reset()?;

            last_end = m + matcher.get_match_len();
        }

        // Remaining text
        self.write_json_str_fragment(writer, &input[last_end..], json_buf)?;
        writer.write_all(b"\"")?; // Closing quote
        Ok(())
    }

    /// Writes a byte slice to the writer, escaping it for use inside a JSON string.
    /// Reuses json_buf to avoid allocations.
    fn write_json_str_fragment(
        &self,
        writer: &mut impl Write,
        chunk: &[u8],
        json_buf: &mut Vec<u8>,
    ) -> io::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        let s = String::from_utf8_lossy(chunk);
        json_buf.clear();
        serde_json::to_writer(&mut *json_buf, &s).map_err(io::Error::other)?;
        // Trim the leading and trailing quotes added by to_writer
        writer.write_all(&json_buf[1..json_buf.len() - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn test_word_char() {
        assert!(is_word_char('a'));
        assert!(is_word_char('Z'));
        assert!(is_word_char('0'));
        assert!(is_word_char('_'));
        assert!(is_word_char('é'));
        assert!(is_word_char('京'));
        assert!(!is_word_char(' '));
        assert!(!is_word_char('!'));
        assert!(!is_word_char('.'));
    }

    #[test]
    fn test_json_keys_check() {
        assert!(!test_for_json_keys("<{key}|{value}>"));
        assert!(test_for_json_keys("<{key}|{value}|{other}>"));
        assert!(test_for_json_keys("{/path/to/json}"));
        assert!(!test_for_json_keys("pure text"));
    }

    fn create_test_fst(name: &str, pairs: &[(&str, &str)]) -> Utf8PathBuf {
        let dir = std::env::temp_dir();
        let path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!(
            "test_fstsed_{}_{}.fst",
            name,
            std::process::id()
        ));
        {
            let wtr = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            let mut build = fst::SetBuilder::new(wtr).unwrap();

            let mut data: Vec<Vec<u8>> = pairs
                .iter()
                .map(|(k, v)| {
                    let mut row = k.as_bytes().to_vec();
                    row.push(SENTINEL);
                    row.extend_from_slice(
                        zstd::stream::encode_all(v.as_bytes(), 3)
                            .unwrap()
                            .as_slice(),
                    );
                    row
                })
                .collect();
            data.sort();
            for row in data {
                build.insert(row).unwrap();
            }
            build.finish().unwrap();
        }
        path
    }

    #[test]
    fn test_fstsed_search() {
        let fst_path =
            create_test_fst("search", &[("foo", "bar"), ("baz", "qux"), ("京", "tokyo")]);
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();

        let haystack = "foo baz foobar 京 text".as_bytes();
        let mut cache = FstCache::new();
        let matches: Vec<usize> = fsed.find_iter(haystack, &mut cache).collect();

        assert_eq!(matches, vec![0, 4, 15]);

        let _ = std::fs::remove_file(fst_path);
    }

    #[test]
    fn test_fstsed_json_color() {
        let fst_path = create_test_fst("json_color", &[("foo", "bar")]);
        // Use Always to ensure color spec is applied
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Always).unwrap();

        let haystack = "{\"msg\": \"hello foo world\"}".as_bytes();
        let mut cache = FstCache::new();

        // Use Ansi wrapper to capture raw escapes for inspection
        use termcolor::Ansi;
        let mut out = Ansi::new(vec![]);

        fsed.stream_find(&mut &haystack[..], &mut out, &mut cache, false, true)
            .unwrap();

        let result = String::from_utf8(out.into_inner()).unwrap();
        // The result should contain the raw ANSI red escape code
        // and NOT the escaped version "\\u001b"
        assert!(result.contains("\x1b[31m"));
        assert!(result.contains("\x1b[1m"));
        assert!(result.contains("<foo|bar>"));
        assert!(result.contains("\x1b[0m"));

        let _ = std::fs::remove_file(fst_path);
    }

    #[test]
    fn test_fstsed_overlap_and_none() {
        let fst_path = create_test_fst("overlap", &[("apple", "A"), ("apple pie", "B")]);
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();

        let haystack = "apple apple pie applepie".as_bytes();
        let mut cache = FstCache::new();
        let matches: Vec<usize> = fsed.find_iter(haystack, &mut cache).collect();

        assert_eq!(matches, vec![0, 6]);

        let _ = std::fs::remove_file(fst_path);
    }
}
