use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use fst::SetBuilder;
use serde_json::Value;
use std::fs::File;
use std::io;
use std::str;
use zstd::stream::copy_encode;

const SENTINEL: u8 = 0;

pub fn build_fstsed<R>(mut input: R, key: &str, output: &Utf8PathBuf) -> Result<(), Error>
where
    R: BufReadExt,
{
    let mut vals: Vec<Vec<u8>> = Vec::new();
    let mut num_errors = 0;
    let mut num_blanks = 0;

    // for this loop, we omit the line terminators
    input.for_byte_line(|line| {
        if line.is_empty() {
            num_blanks += 1;
            return Ok(true);
        }
        let jsonline = serde_json::from_slice(line).unwrap_or_else(|_| Value::default());
        if let Some(keyvalue) = jsonline.get(key).and_then(|v| v.as_str()) {
            // conservative sizing - allocate enough memory for key plus full line
            let mut tuple: Vec<u8> = Vec::with_capacity(keyvalue.len() + line.len());

            // the fst key itself
            tuple.extend_from_slice(keyvalue.as_bytes());
            // the sentinel to delineate key from data
            tuple.push(SENTINEL);
            // zstd compress the line and write directly into output tuple
            if copy_encode(line, &mut tuple, 3).is_err() {
                num_errors += 1;
            } else {
                // push the assembled tuple to our vector of vectors
                vals.push(tuple);
            }
        } else {
            num_errors += 1;
        }
        Ok(true)
    })?;

    eprintln!(
        "Processed {} lines successfully with {num_errors} errors and {num_blanks} blank lines...",
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
