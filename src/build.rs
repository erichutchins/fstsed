use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use fst::SetBuilder;
use serde_json::Value;
use std::fs::File;
use std::io;
use std::str;

const SENTINEL: u8 = 0;

pub fn build_fstsed<R>(
    mut input: R,
    key: &str,
    output: &Utf8PathBuf,
    sorted: bool,
) -> Result<(), Error>
where
    R: BufReadExt,
{
    let mut vals: Vec<Vec<u8>> = Vec::new();
    let mut num_errors = 0;
    let mut num_blanks = 0;

    let is_pointer = key.starts_with('/');

    // for this loop, we omit the line terminators
    input.for_byte_line(|line| {
        if line.is_empty() {
            num_blanks += 1;
            return Ok(true);
        }

        let jsonline: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => {
                num_errors += 1;
                return Ok(true);
            }
        };

        // Extract the value based on whether it's a pointer or not
        let keyvalue = if is_pointer {
            jsonline.pointer(key).and_then(Value::as_str)
        } else {
            jsonline.get(key).and_then(Value::as_str)
        };

        if let Some(keyvalue) = keyvalue {
            // conservative sizing - allocate enough memory for key plus full uncompressed line
            let kv_bytes = keyvalue.as_bytes();
            let mut tuple = Vec::with_capacity(kv_bytes.len() + 1 + (line.len() / 2)); // Guessing 2x compression
            
            // start with the fst key itself
            tuple.extend_from_slice(keyvalue.as_bytes());
            // then add the sentinel to delineate key from data
            tuple.push(SENTINEL);

            // zstd compress the line and write directly into output tuple
            if zstd::stream::copy_encode(line, &mut tuple, 3).is_err() {
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

    if !sorted {
        eprintln!("Sorting keys to build the fst...");
        // sort the vector for fst
        vals.sort_unstable();
    }

    // Flag shadowed keys and keys with non-word characters
    let mut shadowed_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut nonword_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // 1. Warning if the input key starts or ends with non-word-chars
    for v in &vals {
        let key_end = v.iter().position(|&b| b == SENTINEL).unwrap();
        let key_bytes = &v[..key_end];
        let key_str = str::from_utf8(key_bytes).unwrap_or("???");

        if let Some(first) = key_str.chars().next() {
            if !crate::is_word_char(first) {
                nonword_keys.insert(key_str.to_string());
            } else if let Some(last) = key_str.chars().last() {
                if !crate::is_word_char(last) {
                    nonword_keys.insert(key_str.to_string());
                }
            }
        }
    }

    // 2. Identify shadowed keys (keys that are prefixes of longer keys)
    // Since vals is sorted, we only need to compare adjacent elements.
    use itermore::IterArrayWindows;
    for [v1, v2] in vals.iter().array_windows() {
        let key1_end = v1.iter().position(|&b| b == SENTINEL).unwrap();
        let key1_bytes = &v1[..key1_end];

        if v2.starts_with(key1_bytes) && v2[key1_end] != SENTINEL {
            let key1_str = str::from_utf8(key1_bytes).unwrap_or("???");
            shadowed_keys.insert(key1_str.to_string());
        }
    }

    if !nonword_keys.is_empty() {
        eprintln!(
            "Warning: {} keys start or end with non-word characters. These may not be discovered correctly with default word boundary matching:",
            nonword_keys.len()
        );
        for (i, key) in nonword_keys.iter().enumerate() {
            if i < 10 {
                eprintln!("  - {}", key);
            } else {
                eprintln!("  ... and {} more", nonword_keys.len() - 10);
                break;
            }
        }
    }

    if !shadowed_keys.is_empty() {
        eprintln!(
            "Warning: {} keys are shadowed by longer terms and will not be matched independently:",
            shadowed_keys.len()
        );
        for (i, key) in shadowed_keys.iter().enumerate() {
            if i < 10 {
                eprintln!("  - {}", key);
            } else {
                eprintln!("  ... and {} more", shadowed_keys.len() - 10);
                break;
            }
        }
        eprintln!("(Shadowed keys are prefixes of longer keys and are ignored when the longer key matches.)");
    }

    // create file
    let wtr = io::BufWriter::new(File::create(output)?);
    let mut set = SetBuilder::new(wtr)?;

    eprintln!("Assembling the fst...");
    // insert into set builder
    vals.iter().for_each(|line| {
        set.insert(line).expect("error building fstsed database");
    });

    // close the fst
    set.finish().map_err(From::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_build_fstsed_basic() -> Result<()> {
        let input_data = r#"{"key": "apple", "value": "A"}
{"key": "banana", "value": "B"}"#;
        let reader = Cursor::new(input_data);
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("test_build_{}.fst", std::process::id()));

        build_fstsed(reader, "key", &fst_path, false)?;

        assert!(fst_path.exists());
        let _ = std::fs::remove_file(fst_path);
        Ok(())
    }

    #[test]
    fn test_build_fstsed_json_pointer() -> Result<()> {
        let input_data = r#"{"data": {"name": "apple"}, "value": "A"}
{"data": {"name": "banana"}, "value": "B"}"#;
        let reader = Cursor::new(input_data);
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("test_build_ptr_{}.fst", std::process::id()));

        build_fstsed(reader, "/data/name", &fst_path, false)?;

        assert!(fst_path.exists());
        let _ = std::fs::remove_file(fst_path);
        Ok(())
    }
}
