use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use fst::raw::Fst;
use memmap2::Mmap;
use microtemplate::{render, Context};
use serde_json::Value;
use std::borrow::Cow;
use std::fs::File;
use termcolor::ColorChoice;

const SENTINEL: u8 = 0;
use bstr::ByteSlice;

use crate::{is_word_byte, is_word_char};

/// FstMatch represents a single match of a fst key in a haystack
/// with its corresponding value from the fst.
pub struct FstMatch<'a> {
    key: Cow<'a, str>,
    value: Cow<'a, str>,
    template: &'a str,
    jsonvalue: Option<Value>,
}

impl FstMatch<'_> {
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

    pub fn get_match(&mut self) -> FstMatch<'_> {
        // Decompress the value into scratch buffer
        self.cache.scratch.clear();
        let _ = zstd::stream::copy_decode(self.cache.value.as_slice(), &mut self.cache.scratch);

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

    pub fn get_match_len(&self) -> usize {
        self.cache.key.len()
    }

    /// Check if position `pos` is a valid word-start boundary.
    /// Returns true if:
    /// - pos == 0 (start of input), OR
    /// - the character at pos-1 is NOT a word character
    #[inline(always)]
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

/// FstCache holds reusable buffers for search operations
pub struct FstCache {
    key: Vec<u8>,
    value: Vec<u8>,
    scratch: Vec<u8>,
    start: usize,
}

impl FstCache {
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

pub struct FstSed {
    fst: Fst<Mmap>,
    pub color: ColorChoice,
    pub template: String,
    has_json_keys: bool,
}

// from https://github.com/BurntSushi/fst/blob/master/fst-bin/src/util.rs
#[inline]
unsafe fn mmap_fst(path: Utf8PathBuf) -> Result<Fst<Mmap>, Error> {
    let mmap = Mmap::map(&File::open(path)?)?;
    let fst = Fst::new(mmap)?;
    Ok(fst)
}

/// Helper function to determine if user-specified template string will require
/// json deserialization. Returns true if the template contains a {var} beyond
/// {key} and [value}
fn test_for_json_keys(template: &str) -> bool {
    template
        .split('{')
        .skip(1)
        .any(|c| !(c.starts_with("key}") || c.starts_with("value}")))
}

impl<'a> FstSed {
    pub fn new(fstpath: Utf8PathBuf, user_template: Option<String>, color: ColorChoice) -> Self {
        let mut template = user_template.unwrap_or_else(|| "<{key}|{value}>".to_string());
        let has_json_keys = test_for_json_keys(&template);

        if color == ColorChoice::Always {
            // if we are printing color, bookend the template with ansi red escapes
            template = format!("\x1b[1;31m{template}\x1b[0;0m");
        }

        let fst = unsafe { mmap_fst(fstpath).expect("Error opening fst database") };

        Self {
            fst,
            color,
            template,
            has_json_keys,
        }
    }

    #[inline]
    pub fn find_iter<'f, 'c>(
        &'f self,
        text: &'a [u8],
        cache: &'c mut FstCache,
    ) -> FstMatches<'f, 'c, 'a> {
        FstMatches::new(self, text, cache)
    }

    // adapted from https://github.com/BurntSushi/fst/pull/104/files
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
                                if let Some(last_char) = value[..i + 1].chars().next_back() {
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
            cache.key.extend_from_slice(&value[..i + 1]);
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

    #[inline]
    pub fn longest_match(&self, text: &'a [u8], cache: &mut FstCache) -> Option<usize> {
        self.longest_match_at(text, 0, cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let fsed = FstSed::new(fst_path.clone(), None, ColorChoice::Never);

        let haystack = "foo baz foobar 京 text".as_bytes();
        let mut cache = FstCache::new();
        let matches: Vec<usize> = fsed.find_iter(haystack, &mut cache).collect();

        assert_eq!(matches, vec![0, 4, 15]);

        let _ = std::fs::remove_file(fst_path);
    }

    #[test]
    fn test_fstsed_overlap_and_none() {
        let fst_path = create_test_fst("overlap", &[("apple", "A"), ("apple pie", "B")]);
        let fsed = FstSed::new(fst_path.clone(), None, ColorChoice::Never);

        let haystack = "apple apple pie applepie".as_bytes();
        let mut cache = FstCache::new();
        let matches: Vec<usize> = fsed.find_iter(haystack, &mut cache).collect();

        assert_eq!(matches, vec![0, 6]);

        let _ = std::fs::remove_file(fst_path);
    }
}
