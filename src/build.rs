use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use fst::SetBuilder;
use serde_json::Value;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::{fs, str};

const SENTINEL: u8 = 0;

pub fn build_fstsed<R>(mut input: R, key: &str, output: Utf8PathBuf) -> Result<(), Error>
where
    R: BufRead,
{
    let mut vals: Vec<String> = Vec::new();
    let mut num_errors = 0;

    input.for_byte_line_with_terminator(|line| {
        if line.is_empty() {
            return Ok(false);
        }
        let jsonline = serde_json::from_slice(line).unwrap_or_else(|_| Value::default());
        if let Some(keyvalue) = jsonline.get(key).and_then(|v| v.as_str()) {
            vals.push(format!(
                "{}{}{}",
                keyvalue,
                SENTINEL,
                str::from_utf8(&line).unwrap_or("")
            ));
        } else {
            num_errors += 1;
        }
        Ok(true)
    })?;

    // sort the vector for fst
    vals.sort_unstable();

    // create file
    let wtr = io::BufWriter::new(File::create(output)?);
    let mut set = SetBuilder::new(wtr)?;

    // insert into set builder
    vals.iter().for_each(|line| {
        set.insert(line).expect("could not update fstsed database");
    });

    // close the fst
    set.finish().map_err(From::from)
}
