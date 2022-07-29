use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use fst::raw::{Fst, Output};
use lazy_static::lazy_static;
use memmap2::Mmap;
use microtemplate::{render, Context};
use regex::bytes::Regex;
use serde_json::Value;
use std::fs::File;
use termcolor::ColorChoice;

const SENTINEL: u8 = 0;

struct FstMatch<'a> {
    key: &'a [u8],
    value: &'a [u8],
    jsonvalue: Option<Value>,
}

impl<'a> FstMatch<'a> {
    pub fn new(key: &'a [u8], value: &'a [u8], parse_value: bool) -> Self {
        Self {
            key,
            value,
            jsonvalue: if parse_value {
                Some(serde_json::from_slice(value).unwrap_or_else(|_| Value::default()))
            } else {
                None
            },
        }
    }
}

impl Context for FstMatch<'_> {
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

pub struct FstSed {
    fst: Fst<Mmap>,
    pub color: ColorChoice,
    pub template: String,
    keycache: Vec<u8>,
    valuecache: Vec<u8>,
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

impl FstSed {
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
            keycache: Vec::with_capacity(256),
            valuecache: Vec::with_capacity(2048),
            has_json_keys,
        }
    }

    #[inline]
    pub fn render_match(&self) -> String {
        let lastmatch = FstMatch::new(
            self.keycache.as_slice(),
            self.valuecache.as_slice(),
            self.has_json_keys,
        );
        render(&self.template, lastmatch)
    }

    // adapted from https://github.com/BurntSushi/fst/pull/104/files
    #[inline]
    pub fn longest_match(&mut self, value: &[u8]) -> Option<usize> {
        // has to be borrowed mutable so we can keep internal cache up to date
        self.keycache.clear();
        self.valuecache.clear();

        lazy_static! {
            static ref RE_NONWORD: Regex = Regex::new(r"\W").unwrap();
        }

        let mut node = self.fst.root();
        let mut out = Output::zero();
        let mut last_match = None;
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
                                self.valuecache.push(t.inp);
                                snode = self.fst.node(t.addr);
                            } else {
                                // somehow ran out of nodes!
                                break;
                            }
                        }

                        last_match = Some(i + 1);
                        self.keycache.extend_from_slice(&value[..i + 1]);
                    }
                }
            } else {
                return last_match;
            }
        }
        last_match
    }
}
