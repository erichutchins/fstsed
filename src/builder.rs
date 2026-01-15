//! FST database building functionality.
//!
//! This module provides [`build_fstsed`] for creating FST databases from
//! newline-delimited JSON (NDJSON) input.

use crate::error::{FstSedError, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use fst::SetBuilder;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufWriter;
use std::str;

use crate::util::{is_word_char, SENTINEL};

/// Statistics collected during FST building.
struct BuildStats {
    num_errors: usize,
    num_blanks: usize,
}

impl BuildStats {
    fn new() -> Self {
        Self {
            num_errors: 0,
            num_blanks: 0,
        }
    }
}

/// An entry in the data pool for unsorted input processing.
struct Entry {
    start: usize,
    end: usize,
    key_len: usize,
}

/// Parses a JSON line and extracts the key value.
///
/// Returns `Some(key_string)` if successful, `None` if the line is invalid
/// or the key field is missing.
fn parse_json_key(line: &[u8], key: &str, is_pointer: bool) -> Option<String> {
    let jsonline: Value = serde_json::from_slice(line).ok()?;

    let key_value = if is_pointer {
        jsonline.pointer(key).and_then(Value::as_str)
    } else {
        jsonline.get(key).and_then(Value::as_str)
    };

    key_value.map(String::from)
}

/// Processes sorted input, streaming directly to the FST builder.
///
/// This path uses O(1) memory since entries are written immediately
/// without buffering.
fn process_sorted_input<R>(mut input: R, key: &str, output: &Utf8PathBuf) -> Result<BuildStats>
where
    R: BufReadExt,
{
    let mut stats = BuildStats::new();
    let is_pointer = key.starts_with('/');

    eprintln!("Streaming sorted data directly to FST...");
    let wtr = BufWriter::new(File::create(output).map_err(FstSedError::Build)?);
    let mut set = SetBuilder::new(wtr).map_err(FstSedError::FstWrite)?;
    let mut last_line: Option<Vec<u8>> = None;

    input.for_byte_line(|line| {
        if line.is_empty() {
            stats.num_blanks += 1;
            return Ok(true);
        }

        let Some(keyvalue) = parse_json_key(line, key, is_pointer) else {
            stats.num_errors += 1;
            return Ok(true);
        };

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
            // This expect is acceptable here because FST insertion should only fail
            // if keys are not in sorted order, which indicates corrupted input data.
            set.insert(&tuple).expect("error building fstsed database");
            last_line = Some(tuple);
        } else {
            stats.num_errors += 1;
        }

        Ok(true)
    })?;

    set.finish()?;
    eprintln!(
        "Processed with {} errors and {} blank lines.",
        stats.num_errors, stats.num_blanks
    );

    Ok(stats)
}

/// Processes unsorted input by buffering all entries, sorting, then writing.
///
/// This path requires O(n) memory to hold all entries before sorting.
fn process_unsorted_input<R>(mut input: R, key: &str, output: &Utf8PathBuf) -> Result<BuildStats>
where
    R: BufReadExt,
{
    let mut stats = BuildStats::new();
    let is_pointer = key.starts_with('/');

    let mut data_pool = Vec::new();
    let mut entries = Vec::new();

    input.for_byte_line(|line| {
        if line.is_empty() {
            stats.num_blanks += 1;
            return Ok(true);
        }

        let Some(keyvalue) = parse_json_key(line, key, is_pointer) else {
            stats.num_errors += 1;
            return Ok(true);
        };

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
            stats.num_errors += 1;
        }

        Ok(true)
    })?;

    eprintln!(
        "Processed {} lines with {} errors and {} blank lines...",
        entries.len(),
        stats.num_errors,
        stats.num_blanks
    );

    eprintln!("Sorting keys to build the fst...");
    entries.sort_unstable_by(|a, b| data_pool[a.start..a.end].cmp(&data_pool[b.start..b.end]));
    entries.dedup_by(|a, b| data_pool[a.start..a.end] == data_pool[b.start..b.end]);

    // Detect and warn about problematic keys
    detect_problematic_keys(&entries, &data_pool);

    eprintln!("Assembling the fst...");
    let wtr = BufWriter::new(File::create(output).map_err(FstSedError::Build)?);
    let mut set = SetBuilder::new(wtr).map_err(FstSedError::FstWrite)?;
    for e in entries {
        set.insert(&data_pool[e.start..e.end])
            .map_err(FstSedError::FstWrite)?;
    }
    set.finish().map_err(FstSedError::FstWrite)?;

    Ok(stats)
}

/// Detects and warns about keys that may cause matching issues.
///
/// This includes:
/// - Keys that start or end with non-word characters (won't match at word boundaries)
/// - Keys that are shadowed by longer keys (e.g., "apple" shadowed by "applepie")
fn detect_problematic_keys(entries: &[Entry], data_pool: &[u8]) {
    let mut shadowed_keys = BTreeSet::new();
    let mut nonword_keys = BTreeSet::new();

    for (i, e1) in entries.iter().enumerate() {
        let key1_bytes = &data_pool[e1.start..e1.start + e1.key_len];
        let key1_str = str::from_utf8(key1_bytes).unwrap_or("???");

        // Check for non-word character boundaries
        if let Some(first) = key1_str.chars().next() {
            if !is_word_char(first) {
                nonword_keys.insert(key1_str.to_string());
            } else if let Some(last) = key1_str.chars().last() {
                if !is_word_char(last) {
                    nonword_keys.insert(key1_str.to_string());
                }
            }
        }

        // Check for shadowed keys
        if i + 1 < entries.len() {
            let e2 = &entries[i + 1];
            let e2_data = &data_pool[e2.start..e2.end];
            if e2_data.starts_with(key1_bytes) && e2_data[e1.key_len] != SENTINEL {
                shadowed_keys.insert(key1_str.to_string());
            }
        }
    }

    if !nonword_keys.is_empty() {
        eprintln!(
            "Warning: {} keys start or end with non-word characters...",
            nonword_keys.len()
        );
        for k in nonword_keys.iter().take(10) {
            eprintln!("  - {k}");
        }
    }

    if !shadowed_keys.is_empty() {
        eprintln!("Warning: {} keys are shadowed...", shadowed_keys.len());
        for k in shadowed_keys.iter().take(10) {
            eprintln!("  - {k}");
        }
    }
}

/// Builds an FST database from NDJSON input.
///
/// Each line of input should be a JSON object containing at least the specified
/// key field. The key becomes the searchable term, and the entire JSON line
/// (zstd-compressed) is stored as the value.
///
/// # Arguments
///
/// * `input` - A buffered reader providing NDJSON input
/// * `key` - The JSON field name to use as the key (e.g., "name" or "/path/to/field")
/// * `output` - Path where the FST file will be written
/// * `sorted` - If `true`, assumes input is pre-sorted by key for streaming O(1) memory build
///
/// # Errors
///
/// Returns an error if:
/// - The output file cannot be created
/// - The FST builder encounters an error
/// - I/O errors occur during reading or writing
///
/// # Panics
///
/// Panics if sorted input contains out-of-order keys, as this indicates
/// corrupted or incorrectly sorted input data.
pub fn build_fstsed<R>(input: R, key: &str, output: &Utf8PathBuf, sorted: bool) -> Result<()>
where
    R: BufReadExt,
{
    if sorted {
        process_sorted_input(input, key, output)?;
    } else {
        process_unsorted_input(input, key, output)?;
    }
    Ok(())
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

    #[test]
    fn test_parse_json_key() {
        let line = br#"{"name": "test", "value": 42}"#;

        assert_eq!(
            parse_json_key(line, "name", false),
            Some("test".to_string())
        );
        assert_eq!(parse_json_key(line, "missing", false), None);

        let nested = br#"{"data": {"name": "nested"}}"#;
        assert_eq!(
            parse_json_key(nested, "/data/name", true),
            Some("nested".to_string())
        );
    }
}
