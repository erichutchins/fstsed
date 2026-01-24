//! Match results from FST searches.
//!
//! This module provides the [`FstMatch`] and [`FstMatches`] types, which
//! represent the results of a search operation.

use crate::search::{FstCache, FstSed};
use crate::template::{Context, Template};
use crate::util::{is_word_byte, is_word_char};
use bstr::ByteSlice;
use serde_json::Value;
use std::borrow::Cow;

/// Represents a single match of an FST key in a haystack
/// with its corresponding value from the FST.
pub struct FstMatch<'a> {
    key: Cow<'a, str>,
    value: Cow<'a, str>,
    template: &'a Template,
    jsonvalue: Option<Value>,
}

impl<'a> FstMatch<'a> {
    /// Creates a new `FstMatch` for testing or benchmarking.
    pub fn new(
        key: Cow<'a, str>,
        value: Cow<'a, str>,
        template: &'a Template,
        jsonvalue: Option<Value>,
    ) -> Self {
        Self {
            key,
            value,
            template,
            jsonvalue,
        }
    }

    /// Returns a displayable object that renders the match.
    ///
    /// This uses `std::fmt::from_fn` to allow rendering directly to a formatter
    /// without intermediate allocation (new in Rust v1.93).
    #[must_use]
    pub fn display(&self) -> impl std::fmt::Display + '_ {
        std::fmt::from_fn(move |f| self.template.render(self, f))
    }
}

impl Context for FstMatch<'_> {
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
            // ASCII fast path: direct byte check
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
