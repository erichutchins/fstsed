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
    let mut num_errors = 0;
    let mut num_blanks = 0;

    let is_pointer = key.starts_with('/');

    // --- CASE 1: STREAMING PATH (O(1) Memory) ---
    if sorted {
        eprintln!("Streaming sorted data directly to FST...");
        let wtr = io::BufWriter::new(File::create(output)?);
        let mut set = SetBuilder::new(wtr)?;
        let mut last_line: Option<Vec<u8>> = None;

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

            let keyvalue = if is_pointer {
                jsonline.pointer(key).and_then(Value::as_str)
            } else {
                jsonline.get(key).and_then(Value::as_str)
            };

            if let Some(keyvalue) = keyvalue {
                let kv_bytes = keyvalue.as_bytes();
                let mut tuple = Vec::with_capacity(kv_bytes.len() + 1 + (line.len() / 2));
                tuple.extend_from_slice(kv_bytes);
                tuple.push(SENTINEL);

                if zstd::stream::copy_encode(line, &mut tuple, 3).is_ok() {
                    // FST requires strictly increasing keys. Dedup on the fly.
                    if let Some(ref last) = last_line {
                        if tuple == *last {
                            return Ok(true);
                        }
                    }
                    set.insert(&tuple).expect("error building fstsed database");
                    last_line = Some(tuple);
                } else {
                    num_errors += 1;
                }
            } else {
                num_errors += 1;
            }
            Ok(true)
        })?;

        set.finish()?;
        eprintln!("Processed with {num_errors} errors and {num_blanks} blank lines.");
        return Ok(());
    }

    // --- CASE 2: BIG BUFFER PATH (Unsorted) ---
    struct Entry {
        start: usize,
        end: usize,
        key_len: usize,
    }

    let mut data_pool = Vec::new();
    let mut entries = Vec::new();

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

        let keyvalue = if is_pointer {
            jsonline.pointer(key).and_then(Value::as_str)
        } else {
            jsonline.get(key).and_then(Value::as_str)
        };

        if let Some(keyvalue) = keyvalue {
            let start = data_pool.len();
            let kv_bytes = keyvalue.as_bytes();
            let key_len = kv_bytes.len();
            
            data_pool.extend_from_slice(kv_bytes);
            data_pool.push(SENTINEL);

            if zstd::stream::copy_encode(line, &mut data_pool, 3).is_ok() {
                entries.push(Entry {
                    start,
                    end: data_pool.len(),
                    key_len,
                });
            } else {
                data_pool.truncate(start);
                num_errors += 1;
            }
        } else {
            num_errors += 1;
        }
        Ok(true)
    })?;

    eprintln!(
        "Processed {} lines with {num_errors} errors and {num_blanks} blank lines...",
        entries.len()
    );

    eprintln!("Sorting keys to build the fst...");
    entries.sort_unstable_by(|a, b| {
        data_pool[a.start..a.end].cmp(&data_pool[b.start..b.end])
    });

    entries.dedup_by(|a, b| {
        data_pool[a.start..a.end] == data_pool[b.start..b.end]
    });

    let mut shadowed_keys = std::collections::BTreeSet::new();
    let mut nonword_keys = std::collections::BTreeSet::new();

    for i in 0..entries.len() {
        let e1 = &entries[i];
        let key1_bytes = &data_pool[e1.start..e1.start + e1.key_len];
        let key1_str = str::from_utf8(key1_bytes).unwrap_or("???");

        if let Some(first) = key1_str.chars().next() {
            if !crate::is_word_char(first) {
                nonword_keys.insert(key1_str.to_string());
            } else if let Some(last) = key1_str.chars().last() {
                if !crate::is_word_char(last) {
                    nonword_keys.insert(key1_str.to_string());
                }
            }
        }

        if i + 1 < entries.len() {
            let e2 = &entries[i + 1];
            let e2_data = &data_pool[e2.start..e2.end];
            if e2_data.starts_with(key1_bytes) && e2_data[e1.key_len] != SENTINEL {
                shadowed_keys.insert(key1_str.to_string());
            }
        }
    }

    if !nonword_keys.is_empty() {
        eprintln!("Warning: {} keys start or end with non-word characters...", nonword_keys.len());
        for k in nonword_keys.iter().take(10) { eprintln!("  - {k}"); }
    }

    if !shadowed_keys.is_empty() {
        eprintln!("Warning: {} keys are shadowed...", shadowed_keys.len());
        for k in shadowed_keys.iter().take(10) { eprintln!("  - {k}"); }
    }

    eprintln!("Assembling the fst...");
    let wtr = io::BufWriter::new(File::create(output)?);
    let mut set = SetBuilder::new(wtr)?;
    for e in entries {
        set.insert(&data_pool[e.start..e.end])?;
    }
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
