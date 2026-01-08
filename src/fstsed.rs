use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use fst::raw::Fst;
use memmap2::Mmap;
use microtemplate::{render, Context};
use regex::bytes::Regex;
use serde_json::Value;
use std::borrow::Cow;
use std::fs::File;
use std::iter::Peekable;
use std::sync::LazyLock;
use termcolor::ColorChoice;

const SENTINEL: u8 = 0;

// RE_START and RE_NONWORD are used to find candidate positions
// to evaluate for fst keyword matches
// Note how these disable unicode matching (?i-u). key perf improvement
static RE_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i-u)^").unwrap());
static RE_NONWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)(?i-u)[, \t\a\n:="]"#).unwrap());

// is_word_char checks if a byte is a word character (per Unicode)
// This is used to validate match boundaries without a full regex engine call.
#[inline]
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

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
    skip: usize,
    // chain the two regexes iters together to ensure we can search for matches at the beginning of
    // a line as well as when word boundaries occur at beginning of line. both might match at pos
    // 0, but operate in different modes
    // TODO: would be better to use a Generic here...
    reiter:
        Peekable<std::iter::Chain<regex::bytes::Matches<'f, 'a>, regex::bytes::Matches<'f, 'a>>>,
}

impl<'f, 'c, 'a> FstMatches<'f, 'c, 'a> {
    pub fn new(fstsed: &'f FstSed, haystack: &'a [u8], cache: &'c mut FstCache) -> Self {
        Self {
            fstsed,
            haystack,
            cache,
            skip: 0,
            reiter: RE_START
                .find_iter(haystack)
                .chain(RE_NONWORD.find_iter(haystack))
                .peekable(),
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
}

// ideally this iterator would return a custom Match object with the true start offset of the text
// match, plus the text of the match itself, but the constructor is private. Could not
// overcome lifetime issues with returning a FstMatch directly from this.
// ideally this iterator would return a custom Match object with the true start offset of the text
// match, plus the text of the match itself, but the constructor is private. Could not
// overcome lifetime issues with returning a FstMatch directly from this.
impl Iterator for FstMatches<'_, '_, '_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while let Some(m) = self.reiter.next() {
            // self.skip will be 0 only for the very first iteration (RE_START match at pos 0).
            // for all other iterations, we are looking for word boundaries from RE_NONWORD
            // and thus want to test if the NEXT byte is in the fst.
            let start_pos = m.start() + self.skip;
            // avoid branching but ensure skip is 1 for all subsequent iterations
            self.skip = 1;

            if self
                .fstsed
                .longest_match_at(self.haystack, start_pos, self.cache)
                .is_some()
            {
                let match_start = self.cache.start;
                let match_end = match_start + self.cache.key.len();

                // when we have a match, we must advance the reiter position
                // past the point of the last match length before we can resume searching
                while self.reiter.peek().is_some()
                    && self.reiter.peek().unwrap().start() < match_end
                {
                    self.reiter.next();
                }

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
        let mut last_match = None;
        let value = &text[start..];

        for (i, &b) in value.iter().enumerate() {
            if let Some(trans_index) = node.find_input(b) {
                let t = node.transition(trans_index);
                node = self.fst.node(t.addr);

                if let Some(sentinel_index) = node.find_input(SENTINEL) {
                    // validate candidate match has nonword boundary char next
                    // or is at the end of the line. we dont want matches inside other strings,
                    // foo should not match inside foobar
                    let is_boundary = if i == value.len() - 1 {
                        true
                    } else {
                        // use bstr to decode the next character and check if it's a word char
                        use bstr::ByteSlice;
                        if let Some(next_char) = value[i + 1..].chars().next() {
                            !is_word_char(next_char)
                        } else {
                            true
                        }
                    };

                    if is_boundary {
                        // we have a match!
                        cache.key.clear();
                        cache.value.clear();
                        last_match = Some(i + 1);
                        cache.key.extend_from_slice(&value[..i + 1]);
                        cache.start = start;

                        // find the sentinel node, then read to the the final node
                        // to retrieve the "value"
                        let sentinel = node.transition(sentinel_index);
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
                    }
                }
            } else {
                return last_match;
            }
        }
        last_match
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

        // Should match "foo" at 0, "baz" at 4, and "京" at 15
        // "foobar" should NOT match "foo" because of word boundary
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

        // "apple" at 0 matches.
        // "apple pie" at 6 matches (longest match).
        // "applepie" at 16 does NOT match because of word boundary at the end.
        // "apple" at 0 matches.
        // "apple pie" at 6 matches (longest match).
        // "applepie" at 16 does NOT match because of word boundary at the end.
        assert_eq!(matches, vec![0, 6]);

        let _ = std::fs::remove_file(fst_path);
    }
}
