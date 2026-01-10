//! Sink implementation for integrating FstSed with grep-searcher.
//!
//! This module provides [`FstSedSink`], which acts as the bridge between
//! `grep-searcher` and `fstsed`. It receives lines from the searcher,
//! processes them with the FST matcher, and writes results to a buffer.

use std::io::{self, Write};

use grep_searcher::{Searcher, Sink, SinkMatch};
use termcolor::Buffer;

use crate::fstsed::{FstCache, FstSed};
use crate::jsonquotes::jsonquotes_range_iter;
use crate::BUFFERSIZE;

/// `FstSedSink` acts as the bridge between `grep-searcher` and `fstsed`.
///
/// It receives lines from the searcher, processes them with fstsed,
/// and writes the result to the provided buffer.
pub struct FstSedSink<'a, 'b> {
    fsed: &'a FstSed,
    cache: &'b mut FstCache,
    buf: &'b mut Buffer,
    only_matching: bool,
    json: bool,
    /// Scratch buffer for JSON string handling.
    temp_buf: Vec<u8>,
}

impl<'a, 'b> FstSedSink<'a, 'b> {
    /// Creates a new `FstSedSink`.
    ///
    /// # Arguments
    ///
    /// * `fsed` - Reference to the FstSed instance for pattern matching
    /// * `cache` - Mutable reference to the FST cache for reuse
    /// * `buf` - Mutable reference to the output buffer
    /// * `only_matching` - If true, only output matching portions
    /// * `json` - If true, process input as JSON strings
    pub fn new(
        fsed: &'a FstSed,
        cache: &'b mut FstCache,
        buf: &'b mut Buffer,
        only_matching: bool,
        json: bool,
    ) -> Self {
        Self {
            fsed,
            cache,
            buf,
            only_matching,
            json,
            temp_buf: Vec::with_capacity(BUFFERSIZE),
        }
    }

    /// Processes plain text input, replacing matches with rendered templates.
    #[inline]
    pub fn process_text(&mut self, input: &[u8]) -> Result<bool, io::Error> {
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

        Ok(true)
    }

    /// Processes JSON input, searching within string values.
    #[inline]
    pub fn process_json(&mut self, line: &[u8]) -> Result<bool, io::Error> {
        let mut lastpos: usize = 0;
        for (start, end, has_esc) in jsonquotes_range_iter(line) {
            // print from last spot to new start
            if !self.only_matching {
                self.buf.write_all(&line[lastpos..start])?;
            }

            // CONTENT: line[start+1..end-1]
            let raw_content = &line[start + 1..end - 1];

            if !has_esc {
                // FAST PATH: No escapes to handle!
                // We can run FST directly on the raw bytes from the line.
                self.process_content(raw_content)?;
            } else {
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
            }
            lastpos = end;
        }
        if !self.only_matching {
            self.buf.write_all(&line[lastpos..])?;
        }
        Ok(true)
    }

    /// Processes content within a JSON string value.
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

impl Sink for FstSedSink<'_, '_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let line = mat.bytes();
        if self.json {
            self.process_json(line)
        } else {
            self.process_text(line)
        }
    }
}
