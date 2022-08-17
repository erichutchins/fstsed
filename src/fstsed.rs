use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use fst::raw::{Fst, Output};
use lazy_static::lazy_static;
use memmap2::Mmap;
use microtemplate::{render, Context};
use regex::bytes::Match;
use regex::bytes::Matches;
use regex::bytes::Regex;
use serde_json::Value;
use std::cell::RefCell;
use std::fs::File;
use termcolor::ColorChoice;

const SENTINEL: u8 = 0;

lazy_static! {
    static ref RE_NONWORD_OR_START: Regex = Regex::new(r"(?i-u)^|\W").unwrap();
}

/// FstMatch represents a single match of a fst key in a haystack
/// with its corresponding value from the fst.
///
/// The lifetime parameter `'a` refers to the lifetime of the haystack text.
/// The lifetime parameter `'f` refers to the lifetime of the fstsed object holding cached matches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FstMatch<'f> {
    //start: usize,
    key: &'f [u8],
    value: &'f [u8],
    template: &'f str,
    jsonvalue: Option<Value>,
}

impl<'f> FstMatch<'f> {
    pub fn new(key: &'f [u8], value: &'f [u8], template: &'f str, parse_value: bool) -> Self {
        Self {
            key,
            value,
            template,
            jsonvalue: if parse_value {
                Some(serde_json::from_slice(value).unwrap_or_else(|_| Value::default()))
            } else {
                None
            },
        }
    }

    pub fn render(&self) -> String {
        render(self.template, self)
    }
}

impl Context for &FstMatch<'_> {
    fn get_field(&self, field_name: &str) -> &str {
        match field_name {
            "key" => unsafe { std::str::from_utf8_unchecked(self.key) },
            "value" => unsafe { std::str::from_utf8_unchecked(self.value) },
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
    reiter: Matches<'f, 'a>,
}

impl<'f, 'a> FstMatches<'f, 'a> {
    pub fn new(fstsed: &'f FstSed, haystack: &'a [u8]) -> Self {
        Self {
            fstsed,
            haystack,
            reiter: RE_NONWORD_OR_START.find_iter(haystack),
        }
    }
}

impl<'f, 'a> Iterator for FstMatches<'f, 'a> {
    type Item = Match<'a>;
    //type Item = usize;

    //fn next(&mut self) -> Option<usize> {
    fn next(&mut self) -> Option<Match<'a>> {
        let mut m = self.reiter.next();
        while m.is_some()
            && self
                .fstsed
                .longest_match_at(self.haystack, m.unwrap().start())
                .is_none()
        {
            m = self.reiter.next();
        }
        m
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
            template = format!("\x1b[1;31m{}\x1b[0;0m", template);
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

    // #[inline]
    // pub fn get_match<'f> (&'f self) -> FstMatch<'f> {
    //     FstMatch::<'f>::new(
    //         self.keycache.borrow().as_slice(),
    //         self.valuecache.borrow().as_slice(),
    //         &self.template,
    //         self.has_json_keys,
    //     )
    // }

    #[inline]
    pub fn get_match_len(&self) -> usize {
        self.keycache.borrow().len()
    }

    #[inline]
    pub fn find_iter<'f>(&'f self, text: &'a [u8]) -> FstMatches<'f, 'a> {
        FstMatches::new(self, text)
    }

    // adapted from https://github.com/BurntSushi/fst/pull/104/files
    #[inline]
    pub fn longest_match_at(&self, text: &'a [u8], start: usize) -> Option<usize> {
        // self has to be borrowed mutable so we can keep internal cache up to date
        self.keycache.borrow_mut().clear();
        self.valuecache.borrow_mut().clear();
        *self.startcache.borrow_mut() = start;

        lazy_static! {
            static ref RE_NONWORD: Regex = Regex::new(r"(?i-u)\W").unwrap();
        }

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
                    if i == value.len() - 1 || RE_NONWORD.is_match(&value[i + 1..i + 2]) {
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
