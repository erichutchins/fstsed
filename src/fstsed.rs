//! FST-based search engine for text replacement.
//!
//! This module provides the core [`FstSed`] type for performing efficient
//! multi-pattern matching using Finite State Transducers.

use crate::error::{FstSedError, Result};
use camino::Utf8Path;
use fst::raw::Fst;
use memmap2::Mmap;
use microtemplate::{render, Context};
use serde_json::Value;
use std::borrow::Cow;
use std::fs::File;
use termcolor::ColorChoice;

use bstr::ByteSlice;

use crate::jsonquotes::jsonquotes_range_iter;
use crate::{is_word_byte, is_word_char, BUFFERSIZE, SENTINEL};
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
    /// Renders the match using the configured template.
    ///
    /// Template variables like `{key}` and `{value}` are replaced with
    /// the actual matched key and its associated value.
    #[must_use]
    pub fn render(&self) -> String {
        render(self.template, self)
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
            self.pos += 1;

            // Quick check: is this position a potential word start?
            // A word can start here if:
            // 1. We're at position 0, OR
            // 2. The previous byte is not a word character
            //
            // We also only attempt FST matching if the current byte could
            // be the start of a key (any byte is valid, FST will reject quickly)

            if !self.is_word_start_boundary(start_pos) {
                continue;
            }

            // Try to match at this position
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
}

/// Memory-maps an FST file from disk.
///
/// # Safety
///
/// The caller must ensure the file is not modified while the returned
/// FST is in use. Modifying the underlying file while it's memory-mapped
/// leads to undefined behavior.
///
/// Adapted from <https://github.com/BurntSushi/fst/blob/master/fst-bin/src/util.rs>
#[inline]
unsafe fn mmap_fst(path: &Utf8Path) -> Result<Fst<Mmap>> {
    let mmap = Mmap::map(&File::open(path).map_err(FstSedError::FstMmap)?)
        .map_err(FstSedError::FstMmap)?;
    let fst = Fst::new(mmap).map_err(FstSedError::FstParse)?;
    Ok(fst)
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
        let mut template = user_template.unwrap_or_else(|| "<{key}|{value}>".to_string());
        let has_json_keys = test_for_json_keys(&template);

        if color == ColorChoice::Always {
            // if we are printing color, bookend the template with ansi red escapes
            template = format!("\x1b[1;31m{template}\x1b[0;0m");
        }

        // SAFETY: Memory-mapped FST files must not be modified while in use.
        // The caller is responsible for ensuring file stability.
        let fst = unsafe { mmap_fst(fstpath) }?;

        Ok(Self {
            fst,
            color,
            template,
            has_json_keys,
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

        for (i, &b) in value.iter().enumerate() {
            if let Some(trans_index) = node.find_input(b) {
                let t = node.transition(trans_index);
                node = self.fst.node(t.addr);

                if let Some(sentinel_index) = node.find_input(SENTINEL) {
                    // validation: if key ends with a word char, next char must not be a word char.
                    // Like the leading boundary check, this ensures we don't match 'apple'
                    // inside 'applesauce'.
                    let is_boundary = if i == value.len() - 1 {
                        true
                    } else {
                        let next_byte = value[i + 1];
                        if b < 128 && next_byte < 128 {
                            // ASCII fast path: used to check trailing word character boundaries
                            // without a full UTF-8 decode.
                            if is_word_char(b as char) {
                                !is_word_char(next_byte as char)
                            } else {
                                true
                            }
                        } else {
                            // Unicode Fallback
                            if let Some(next_char) = value[i + 1..].chars().next() {
                                // check last character of the current match
                                if let Some(last_char) = value[..=i].chars().next_back() {
                                    if is_word_char(last_char) {
                                        !is_word_char(next_char)
                                    } else {
                                        true
                                    }
                                } else {
                                    true
                                }
                            } else {
                                true
                            }
                        }
                    };

                    if is_boundary {
                        // Remember this as a potentially longest match. We don't retrieve the
                        // value yet to avoid unnecessary work if we find a longer match later.
                        last_match_info = Some((node, sentinel_index, i));
                    }
                }
            } else {
                break;
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
    pub fn stream_find<R: BufRead, W: Write>(
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
        writer: &mut impl Write,
        cache: &mut FstCache,
    ) -> io::Result<()> {
        let mut last_end = 0;
        let mut matches = self.find_iter(line, cache);

        while let Some(match_start) = matches.next() {
            // Write text before match
            writer.write_all(&line[last_end..match_start])?;

            // Write replacement
            let replacement = matches.get_match().render();
            writer.write_all(replacement.as_bytes())?;

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
        writer: &mut impl Write,
        cache: &mut FstCache,
    ) -> io::Result<()> {
        let mut matches = self.find_iter(line, cache);

        while let Some(_) = matches.next() {
            let replacement = matches.get_match().render();
            writer.write_all(replacement.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Process a JSON line.
    #[inline]
    fn process_json_line(
        &self,
        line: &[u8],
        writer: &mut impl Write,
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
        writer: &mut impl Write,
        cache: &mut FstCache,
        json_buf: &mut Vec<u8>,
    ) -> io::Result<()> {
        json_buf.clear();
        let mut inner_lastpos = 0;
        let mut matcher = self.find_iter(input, cache);

        while let Some(m) = matcher.next() {
            json_buf.write_all(&input[inner_lastpos..m])?;
            json_buf.write_all(matcher.get_match().render().as_bytes())?;
            inner_lastpos = m + matcher.get_match_len();
        }
        json_buf.write_all(&input[inner_lastpos..])?;

        serde_json::to_writer(writer, std::str::from_utf8(json_buf).unwrap())?;
        Ok(())
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
