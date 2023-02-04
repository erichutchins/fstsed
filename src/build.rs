use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use fst::SetBuilder;
use serde_json::Value;
use std::fs::File;
use std::io;
use std::str;

const SENTINEL: u8 = 0;

pub fn build_fstsed<R>(mut input: R, key: &str, output: &Utf8PathBuf) -> Result<(), Error>
where
    R: BufReadExt,
{
    let mut vals: Vec<Vec<u8>> = Vec::new();
    let mut num_errors = 0;

    // for this loop, we omit the line terminators
    input.for_byte_line(|line| {
        if line.is_empty() {
            return Ok(false);
        }
        let jsonline = serde_json::from_slice(line).unwrap_or_else(|_| Value::default());
        if let Some(keyvalue) = jsonline.get(key).and_then(|v| v.as_str()) {
            let mut tuple: Vec<u8> = Vec::new();
            tuple.extend_from_slice(keyvalue.as_bytes());
            tuple.push(SENTINEL);
            tuple.extend_from_slice(line);
            vals.push(tuple);
        } else {
            num_errors += 1;
        }
        Ok(true)
    })?;

    eprintln!(
        "Processed {} lines successfully with {num_errors} errors...",
        vals.len()
    );
    eprintln!("Sorting keys to build the fst...");
    // sort the vector for fst
    vals.sort_unstable();

    // create file
    let wtr = io::BufWriter::new(File::create(output)?);
    let mut set = SetBuilder::new(wtr)?;

    eprintln!("Assembling the fst...");
    // insert into set builder
    vals.iter().for_each(|line| {
        set.insert(line).expect("could not update fstsed database");
    });

    // close the fst
    set.finish().map_err(From::from)
}
