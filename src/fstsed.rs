use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use fst::raw::{Fst, Output};
use lazy_static::lazy_static;
use memmap2::Mmap;
use microtemplate::{render, Context};
use regex::bytes::Regex;
use serde_json::Value;
use std::cell::RefCell;
use std::fs::File;
use termcolor::ColorChoice;

const SENTINEL: u8 = 0;

// RE_START and RE_NONWORD are used to find candidate positions
// to evaluate for fst keyword matches
// Note how these disable unicode matching (?i-u). key perf improvement
lazy_static! {
    static ref RE_START: Regex = Regex::new(r"(?i-u)^").unwrap();
}
lazy_static! {
    static ref RE_NONWORD: Regex = Regex::new(r#"(?m)(?i-u)[, \t\a\n:="]"#).unwrap();
}
// RE_UNICODE_BOUNDARY is used within the fstmatch algorithm to validate
// that the end of the match is a boundary and therefore we are not inside
// a word
lazy_static! {
    static ref RE_UNICODE_BOUNDARY: Regex = Regex::new(r"^\W").unwrap();
}

/// FstMatch represents a single match of a fst key in a haystack
/// with its corresponding value from the fst.
///
/// The lifetime parameter `'a` refers to the lifetime of the haystack text.
/// The lifetime parameter `'f` refers to the lifetime of the fstsed object holding cached matches.
pub struct FstMatch<'f> {
    //start: usize,
    key: String,
    value: String,
    template: &'f str,
    jsonvalue: Option<Value>,
}

impl<'f> FstMatch<'f> {
    pub fn render(&self) -> String {
        render(self.template, self)
    }
}

impl Context for &FstMatch<'_> {
    fn get_field(&self, field_name: &str) -> &str {
        match field_name {
            "key" => self.key.as_str(),
            "value" => self.value.as_str(),
            _ => self
                .jsonvalue
                .as_ref()
                .and_then(|jv| jv.get(field_name))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        }
    }
}

pub struct FstMatches<'f, 'a> {
    fstsed: &'f FstSed,
    haystack: &'a [u8],
    skip: usize,
    last_matchlen: usize,
    // chain the two regexes iters together to ensure we can search for matches at the beginning of
    // a line as well as when word boundaries occur at beginning of line. both might match at pos
    // 0, but operate in different modes
    // TODO: would be better to use a Generic here...
    reiter: std::iter::Chain<regex::bytes::Matches<'f, 'a>, regex::bytes::Matches<'f, 'a>>,
}

impl<'f, 'a> FstMatches<'f, 'a> {
    pub fn new(fstsed: &'f FstSed, haystack: &'a [u8]) -> Self {
        Self {
            fstsed,
            haystack,
            skip: 0,
            last_matchlen: 0,
            reiter: RE_START
                .find_iter(haystack)
                .chain(RE_NONWORD.find_iter(haystack)),
        }
    }
}

// ideally this iterator would return a custom Match object with the true start offset of the text
// match, plus the text of the match itself, but the constructor is private. Could not
// overcome lifetime issues with returning a FstMatch directly from this.
impl<'f, 'a> Iterator for FstMatches<'f, 'a> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let mut m = self.reiter.next();
        // self.skip will be 0 only for the very first iteration. this is because matching at the
        // beginning of the line is a slightly different operation: we want to test that very first
        // byte if it is in the fst. for all other iterations, we are looking for word boundaries
        // and thus want to test if the NEXT byte is in the fst
        while m.is_some()
            && (
                // our new boundary candidate is within our previous match range
                (m.unwrap().start() + self.skip) < (self.fstsed.get_match_start() + self.fstsed.get_match_len())
                // or we dont have a fstsed match at all
                || self
                .fstsed
                .longest_match_at(self.haystack, m.unwrap().start() + self.skip)
                .is_none()
            )
        {
            // advance loop until we find a fstsed match or exhaust the iterator
            m = self.reiter.next();
            // avoid branching of testing "is this the first loop" and just set
            // to 1 over and over again.
            self.skip = 1;
        }

        // return just position of the match start
        // if m is None, the "and" will fail
        m.and(Some(self.fstsed.get_match_start()))
    }
}

pub struct FstSed {
    fst: Fst<Mmap>,
    pub color: ColorChoice,
    pub template: String,
    keycache: RefCell<Vec<u8>>,
    valuecache: RefCell<Vec<u8>>,
    startcache: RefCell<usize>,
    has_json_keys: bool,
}

// from https://github.com/BurntSushi/fst/blob/master/fst-bin/src/util.rs
#[inline]
unsafe fn mmap_fst(path: Utf8PathBuf) -> Result<Fst<Mmap>, Error> {
    let mmap = Mmap::map(&File::open(path)?)?;
    let fst = Fst::new(mmap)?;
    Ok(fst)
}

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
            keycache: RefCell::new(Vec::with_capacity(256)),
            valuecache: RefCell::new(Vec::with_capacity(2048)),
            startcache: RefCell::new(0),
            has_json_keys,
        }
    }

    #[inline]
    pub fn get_match(&self) -> FstMatch {
        // instantiate object directly. i tried using a new constructor, but had lifetime/scoping
        // issues passing references created in this function
        FstMatch {
            key: std::str::from_utf8(self.keycache.borrow().as_slice())
                .unwrap_or("<keyerror>")
                .to_string(),
            value: std::str::from_utf8(self.valuecache.borrow().as_slice())
                .unwrap_or("<valueerror>")
                .to_string(),
            template: &self.template,
            jsonvalue: if self.has_json_keys {
                Some(
                    serde_json::from_slice(self.valuecache.borrow().as_slice())
                        .unwrap_or_else(|_| Value::default()),
                )
            } else {
                None
            },
        }
    }

    #[inline]
    pub fn get_match_len(&self) -> usize {
        self.keycache.borrow().len()
    }

    #[inline]
    pub fn get_match_start(&self) -> usize {
        *self.startcache.borrow()
    }

    #[inline]
    pub fn find_iter<'f>(&'f self, text: &'a [u8]) -> FstMatches<'f, 'a> {
        FstMatches::new(self, text)
    }

    // adapted from https://github.com/BurntSushi/fst/pull/104/files
    #[inline]
    pub fn longest_match_at(&self, text: &'a [u8], start: usize) -> Option<usize> {
        self.keycache.borrow_mut().clear();
        self.valuecache.borrow_mut().clear();

        let mut node = self.fst.root();
        let mut out = Output::zero();
        let mut last_match = None;
        let value = &text[start..];

        for (i, &b) in value.iter().enumerate() {
            if let Some(trans_index) = node.find_input(b) {
                let t = node.transition(trans_index);
                node = self.fst.node(t.addr);
                out = out.cat(t.out);

                if let Some(sentinel_index) = node.find_input(SENTINEL) {
                    // validate candidate match has nonword boundary char next
                    // or is at the end of the line. we dont want matches inside other strings,
                    // foo should not match inside foobar
                    if i == value.len() - 1 || RE_UNICODE_BOUNDARY.is_match(&value[i + 1..]) {
                        let sentinel = node.transition(sentinel_index);
                        let mut snode = self.fst.node(sentinel.addr);
                        while !snode.is_final() {
                            if let Some(t) = snode.transitions().next() {
                                // after the sentinel, we should not have any more
                                // branching in the fst, so we just grab the first transition
                                self.valuecache.borrow_mut().push(t.inp);
                                snode = self.fst.node(t.addr);
                            } else {
                                // somehow ran out of nodes!
                                break;
                            }
                        }

                        last_match = Some(i + 1);
                        self.keycache
                            .borrow_mut()
                            .extend_from_slice(&value[..i + 1]);
                        *self.startcache.borrow_mut() = start;
                    }
                }
            } else {
                return last_match;
            }
        }
        last_match
    }

    #[inline]
    pub fn longest_match(&self, text: &'a [u8]) -> Option<usize> {
        self.longest_match_at(text, 0)
    }
}
