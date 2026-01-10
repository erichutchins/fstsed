//! Property-based tests using proptest.
//!
//! These tests verify correctness properties across a wide range of
//! randomly generated inputs.

use camino::Utf8PathBuf;
use fstsed::fstsed::{FstCache, FstSed};
use fstsed::jsonquotes::jsonquotes_range_iter;
use fstsed::{is_word_byte, is_word_char};
use proptest::prelude::*;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use termcolor::ColorChoice;

static COUNTER: AtomicU32 = AtomicU32::new(0);

// ============================================================================
// Word Boundary Property Tests
// ============================================================================

proptest! {
    /// Property: is_word_byte should be consistent with the expected character set
    #[test]
    fn prop_is_word_byte_ascii_consistency(b in any::<u8>()) {
        let expected = matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_');
        prop_assert_eq!(is_word_byte(b), expected);
    }

    /// Property: is_word_char should agree with is_word_byte for ASCII
    #[test]
    fn prop_is_word_char_ascii_agreement(b in 0u8..128) {
        let c = b as char;
        prop_assert_eq!(is_word_char(c), is_word_byte(b));
    }

    /// Property: is_word_char should be true for all alphanumeric Unicode chars
    #[test]
    fn prop_is_word_char_unicode_alphanumeric(c in any::<char>()) {
        if c.is_alphanumeric() {
            prop_assert!(is_word_char(c));
        }
    }

    /// Property: underscore is always a word character
    #[test]
    fn prop_underscore_is_word_char(_unit in any::<()>()) {
        prop_assert!(is_word_byte(b'_'));
        prop_assert!(is_word_char('_'));
    }
}

// ============================================================================
// JSON Quote Extraction Property Tests
// ============================================================================

/// Strategy to generate valid JSON strings with embedded quotes
fn json_with_strings() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::string::string_regex("[a-zA-Z0-9 .,!?-]{0,20}").unwrap(),
        0..5,
    )
    .prop_map(|strings| {
        let mut json_parts = Vec::new();
        json_parts.push("{".to_string());

        for (i, s) in strings.iter().enumerate() {
            if i > 0 {
                json_parts.push(",".to_string());
            }
            json_parts.push(format!(r#""field{}":"{}""#, i, s.replace('"', r#"\""#)));
        }

        json_parts.push("}".to_string());
        json_parts.join("")
    })
}

proptest! {
    /// Property: Extracted JSON string ranges should be valid UTF-8
    #[test]
    fn prop_json_quotes_valid_utf8(json in json_with_strings()) {
        let bytes = json.as_bytes();
        for (start, end, _has_esc) in jsonquotes_range_iter(bytes) {
            let slice = &bytes[start..end];
            prop_assert!(std::str::from_utf8(slice).is_ok());
        }
    }

    /// Property: All extracted ranges should be within bounds
    #[test]
    fn prop_json_quotes_within_bounds(json in json_with_strings()) {
        let bytes = json.as_bytes();
        let len = bytes.len();
        for (start, end, _has_esc) in jsonquotes_range_iter(bytes) {
            prop_assert!(start <= end);
            prop_assert!(end <= len);
        }
    }

    /// Property: Extracted strings should be valid (including surrounding quotes)
    #[test]
    fn prop_json_quotes_include_delimiters(json in json_with_strings()) {
        let bytes = json.as_bytes();
        for (start, end, _has_esc) in jsonquotes_range_iter(bytes) {
            let slice = &bytes[start..end];
            let s = std::str::from_utf8(slice).unwrap();
            // The extracted range includes the opening and closing quotes
            prop_assert!(s.starts_with('"'), "Range should start with quote: {}", s);
            prop_assert!(s.ends_with('"'), "Range should end with quote: {}", s);
            prop_assert!(s.len() >= 2, "Range should include at least opening and closing quotes");
        }
    }
}

// ============================================================================
// FST Build and Search Consistency Tests
// ============================================================================

/// Strategy to generate valid key-value pairs
fn key_value_pair() -> impl Strategy<Value = (String, String)> {
    (
        prop::string::string_regex("[a-z]{3,10}").unwrap(),
        prop::string::string_regex("[A-Z0-9]{1,5}").unwrap(),
    )
}

proptest! {
    /// Property: Building an FST and searching should find all inserted keys
    #[test]
    fn prop_fst_roundtrip_single_key(key in "[a-z]{3,10}", value in "[A-Z0-9]{1,5}") {
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST
        let json_line = format!(r#"{{"key":"{}","value":"{}"}}"#, key, value);
        let mut file = File::create(&fst_path).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&fst_path).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search for the key
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();

        // The key should be found in text containing it as a word
        let text = format!("hello {} world", key);
        let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

        prop_assert!(!matches.is_empty(), "Key '{}' should be found in '{}'", key, text);

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
    }

    /// Property: Keys not in FST should not match
    #[test]
    fn prop_fst_no_false_positives(
        present_key in "[a-z]{3,10}",
        absent_key in "[a-z]{3,10}",
    ) {
        prop_assume!(present_key != absent_key);

        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST with only present_key
        let json_line = format!(r#"{{"key":"{}","value":"V"}}"#, present_key);
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search for absent_key
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();

        let text = format!("hello {} world", absent_key);
        let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

        prop_assert!(matches.is_empty(), "Absent key '{}' should not match", absent_key);

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }

    /// Property: Word boundary detection - keys inside words should not match
    #[test]
    fn prop_fst_word_boundary_prevention(
        key in "[a-z]{3,8}",
        prefix in "[a-z]{1,4}",
        suffix in "[a-z]{1,4}",
    ) {
        prop_assume!(!prefix.is_empty() || !suffix.is_empty());

        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST
        let json_line = format!(r#"{{"key":"{}","value":"V"}}"#, key);
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search for key embedded in another word
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();

        let embedded = format!("{}{}{}", prefix, key, suffix);
        let text = format!("hello {} world", embedded);
        let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

        prop_assert!(
            matches.is_empty(),
            "Key '{}' embedded in '{}' should not match due to word boundaries",
            key,
            embedded
        );

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }

    /// Property: Multiple keys in FST should all be findable
    #[test]
    fn prop_fst_multiple_keys(keys in prop::collection::vec(key_value_pair(), 1..10)) {
        prop_assume!(!keys.is_empty());

        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST with multiple keys
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        for (key, value) in &keys {
            writeln!(file, r#"{{"key":"{}","value":"{}"}}"#, key, value).unwrap();
        }
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search for each key
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();

        for (key, _) in &keys {
            let mut cache = FstCache::new();
            let text = format!("test {} end", key);
            let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

            prop_assert!(
                !matches.is_empty(),
                "Key '{}' should be found after inserting {} keys",
                key,
                keys.len()
            );
        }

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }
}

// ============================================================================
// Edge Case Tests
// ============================================================================

proptest! {
    /// Property: Empty input should produce no matches
    #[test]
    fn prop_empty_input_no_matches(key in "[a-z]{3,10}") {
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST
        let json_line = format!(r#"{{"key":"{}","value":"V"}}"#, key);
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search empty input
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();
        let matches: Vec<_> = fsed.find_iter(b"", &mut cache).collect();

        prop_assert!(matches.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }

    /// Property: Key at start of text should match
    #[test]
    fn prop_key_at_start(key in "[a-z]{3,10}") {
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST
        let json_line = format!(r#"{{"key":"{}","value":"V"}}"#, key);
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search with key at start
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();
        let text = format!("{} world", key);
        let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

        prop_assert!(!matches.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }

    /// Property: Key at end of text should match
    #[test]
    fn prop_key_at_end(key in "[a-z]{3,10}") {
        let dir = std::env::temp_dir();
        let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
            .join(format!("proptest_{}_{}.fst", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));

        // Build FST
        let json_line = format!(r#"{{"key":"{}","value":"V"}}"#, key);
        let temp_json = fst_path.with_extension("json");
        let mut file = File::create(&temp_json).unwrap();
        writeln!(file, "{}", json_line).unwrap();
        drop(file);

        let input = std::io::BufReader::new(File::open(&temp_json).unwrap());
        fstsed::build::build_fstsed(input, "key", &fst_path, false).unwrap();

        // Search with key at end
        let fsed = FstSed::new(&fst_path, None, ColorChoice::Never).unwrap();
        let mut cache = FstCache::new();
        let text = format!("hello {}", key);
        let matches: Vec<_> = fsed.find_iter(text.as_bytes(), &mut cache).collect();

        prop_assert!(!matches.is_empty());

        // Cleanup
        let _ = std::fs::remove_file(fst_path);
        let _ = std::fs::remove_file(temp_json);
    }
}
