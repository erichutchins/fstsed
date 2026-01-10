//! Processing utilities for `FstSed` text transformation.
//!
//! This module provides [`FstSedSink`], which handles the transformation of
//! text lines using the FST matcher, writing results to a buffer.

use std::io::{self, Write};

use termcolor::Buffer;

use crate::fstsed::{FstCache, FstSed};
use crate::jsonquotes::jsonquotes_range_iter;
use crate::BUFFERSIZE;

/// `FstSedSink` handles text transformation using `FstSed`.
///
/// It processes input lines, applies FST matching and template rendering,
/// and writes the transformed output to the provided buffer.
pub struct FstSedSink<'a, 'b> {
    fsed: &'a FstSed,
    cache: &'b mut FstCache,
    buf: &'b mut Buffer,
    only_matching: bool,
    /// Scratch buffer for JSON string handling.
    temp_buf: Vec<u8>,
}

impl<'a, 'b> FstSedSink<'a, 'b> {
    /// Creates a new `FstSedSink`.
    ///
    /// # Arguments
    ///
    /// * `fsed` - Reference to the `FstSed` instance for pattern matching
    /// * `cache` - Mutable reference to the FST cache for reuse
    /// * `buf` - Mutable reference to the output buffer
    /// * `only_matching` - If true, only output matching portions
    pub fn new(
        fsed: &'a FstSed,
        cache: &'b mut FstCache,
        buf: &'b mut Buffer,
        only_matching: bool,
    ) -> Self {
        Self {
            fsed,
            cache,
            buf,
            only_matching,
            temp_buf: Vec::with_capacity(BUFFERSIZE),
        }
    }

    /// Processes plain text input, replacing matches with rendered templates.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output buffer fails.
    #[inline]
    pub fn process_text(&mut self, input: &[u8]) -> Result<(), io::Error> {
        let mut lastpos: usize = 0;
        let mut matcher = self.fsed.find_iter(input, self.cache);

        while let Some(m) = matcher.next() {
            if !self.only_matching {
                self.buf.write_all(&input[lastpos..m])?;
            }
            // print rendered match
            self.buf
                .write_all(matcher.get_match().render().as_bytes())?;
            if self.only_matching {
                self.buf.write_all(b"\n")?;
            }
            lastpos = m + matcher.get_match_len();
        }

        if !self.only_matching {
            // print remainder
            self.buf.write_all(&input[lastpos..])?;
        }

        Ok(())
    }

    /// Processes JSON input, searching within string values.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output buffer fails.
    #[inline]
    pub fn process_json(&mut self, line: &[u8]) -> Result<(), io::Error> {
        let mut lastpos: usize = 0;
        for (start, end, has_esc) in jsonquotes_range_iter(line) {
            // print from last spot to new start
            if !self.only_matching {
                self.buf.write_all(&line[lastpos..start])?;
            }

            // CONTENT: line[start+1..end-1]
            let raw_content = &line[start + 1..end - 1];

            if has_esc {
                // SLOW PATH: Use Cow to avoid allocation if possible,
                // though has_esc=true means we'll likely allocate once.
                match serde_json::from_slice::<std::borrow::Cow<str>>(&line[start..end]) {
                    Ok(s) => self.process_content(s.as_bytes())?,
                    Err(_) => {
                        if !self.only_matching {
                            self.buf.write_all(&line[start..end])?;
                        }
                    }
                }
            } else {
                // FAST PATH: No escapes to handle!
                // We can run FST directly on the raw bytes from the line.
                self.process_content(raw_content)?;
            }
            lastpos = end;
        }
        if !self.only_matching {
            self.buf.write_all(&line[lastpos..])?;
        }
        Ok(())
    }

    /// Processes content within a JSON string value.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output buffer fails.
    ///
    /// # Panics
    ///
    /// Panics if the processed content is not valid UTF-8. This should not
    /// occur in practice since JSON strings are required to be valid UTF-8.
    #[inline]
    pub fn process_content(&mut self, input: &[u8]) -> Result<(), io::Error> {
        self.temp_buf.clear();
        let mut inner_lastpos = 0;
        let mut matcher = self.fsed.find_iter(input, self.cache);

        while let Some(m) = matcher.next() {
            self.temp_buf.write_all(&input[inner_lastpos..m])?;
            self.temp_buf
                .write_all(matcher.get_match().render().as_bytes())?;
            inner_lastpos = m + matcher.get_match_len();
        }
        self.temp_buf.write_all(&input[inner_lastpos..])?;

        serde_json::to_writer(&mut self.buf, std::str::from_utf8(&self.temp_buf).unwrap())?;
        Ok(())
    }
}
