use camino::Utf8PathBuf;
use fstsed::search::{FstCache, FstSed};
use std::fs;
use std::process::Command;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

/// Test demonstrating that alphanumeric-only prefixes do NOT shadow each other
/// because word boundaries prevent them from matching inside longer strings.
#[test]
fn test_no_shadowing_alphanumeric_prefixes() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!(
        "word_boundary_no_shadow_{}.fst",
        std::process::id()
    ));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!(
        "word_boundary_no_shadow_{}.json",
        std::process::id()
    ));

    // Keys "abc" and "abcde" should NOT shadow each other because:
    // - "abc" requires word boundaries: \b abc \b
    // - "abcde" requires word boundaries: \b abcde \b
    // - "abc" cannot match inside "abcde" (no boundary between 'c' and 'd')
    let json_content = r#"{"key":"abc","val":"short"}
{"key":"abcde","val":"long"}
{"key":"test","val":"word"}
{"key":"testing","val":"longer"}"#;
    fs::write(&json_path, json_content).unwrap();

    let output = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("key")
        .arg(&json_path)
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    // Should have NO shadowing warnings for these purely alphanumeric keys
    assert!(
        !stderr.contains("shadowed"),
        "Alphanumeric prefixes should not be flagged as shadowed: {}",
        stderr
    );

    let mut cache = FstCache::new();
    let fsed = FstSed::new(
        fst_path.clone(),
        Some("{key}={val}".to_string()),
        termcolor::ColorChoice::Never,
    )
    .unwrap();

    // Test 1: "abc" matches when word-bounded
    let input1 = "hello abc world";
    let mut iter = fsed.find_iter(input1.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(iter.get_match().display().to_string(), "abc=short");

    // Test 2: "abcde" matches when word-bounded
    let input2 = "hello abcde world";
    let mut iter = fsed.find_iter(input2.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(iter.get_match().display().to_string(), "abcde=long");

    // Test 3: "abc" does NOT match inside "abcde"
    let input3 = "abcde contains abc as prefix but no match";
    let mut iter = fsed.find_iter(input3.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(0), "Should match 'abcde' at start");
    assert_eq!(iter.get_match().display().to_string(), "abcde=long");
    // "abc" at position 15 should match because it's a standalone word
    assert_eq!(iter.next(), Some(15), "Should match standalone 'abc'");
    assert_eq!(iter.get_match().display().to_string(), "abc=short");
    assert_eq!(iter.next(), None);

    // Test 4: Both can match in same text when properly bounded
    let input4 = "abc and abcde are different";
    let mut iter = fsed.find_iter(input4.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(0));
    assert_eq!(iter.get_match().display().to_string(), "abc=short");
    assert_eq!(iter.next(), Some(8));
    assert_eq!(iter.get_match().display().to_string(), "abcde=long");

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

/// Test demonstrating TRUE shadowing: when a key is followed by a non-word character
/// that creates a longer key.
#[test]
fn test_true_shadowing_with_non_word_chars() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("true_shadow_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("true_shadow_{}.json", std::process::id()));

    // These keys demonstrate TRUE shadowing:
    // - "API" is shadowed by "API-KEY" (hyphen is non-word char)
    // - "user" is shadowed by "user@domain" (@ is non-word char)
    // - "file" is shadowed by "file.txt" (dot is non-word char)
    let json_content = r#"{"key":"API","val":"api-only"}
{"key":"API-KEY","val":"api-key-combo"}
{"key":"user","val":"user-only"}
{"key":"user@domain","val":"email"}
{"key":"file","val":"file-only"}
{"key":"file.txt","val":"filename"}"#;
    fs::write(&json_path, json_content).unwrap();

    let output = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("key")
        .arg(&json_path)
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    // Should warn about shadowed keys: API, user, file
    assert!(
        stderr.contains("Warning: 3 keys are shadowed"),
        "Should detect 3 shadowed keys: {}",
        stderr
    );
    assert!(
        stderr.contains("- API") || stderr.contains("API"),
        "Should flag API as shadowed: {}",
        stderr
    );
    assert!(
        stderr.contains("- user") || stderr.contains("user"),
        "Should flag user as shadowed: {}",
        stderr
    );
    assert!(
        stderr.contains("- file") || stderr.contains("file"),
        "Should flag file as shadowed: {}",
        stderr
    );

    let mut cache = FstCache::new();
    let fsed = FstSed::new(
        fst_path.clone(),
        Some("{key}".to_string()),
        termcolor::ColorChoice::Never,
    )
    .unwrap();

    // The shadowed "API" will never match because "API-KEY" always wins
    let input1 = "check API-KEY value";
    let mut iter = fsed.find_iter(input1.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(iter.get_match().display().to_string(), "API-KEY");
    assert_eq!(iter.next(), None);

    // "user@domain" matches, not "user"
    let input2 = "email user@domain here";
    let mut iter = fsed.find_iter(input2.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(iter.get_match().display().to_string(), "user@domain");

    // "file.txt" matches, not "file"
    let input3 = "open file.txt now";
    let mut iter = fsed.find_iter(input3.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(5));
    assert_eq!(iter.get_match().display().to_string(), "file.txt");

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

/// Test keys with internal non-word characters (like User-Agent strings, URLs, paths)
#[test]
fn test_keys_with_internal_non_word_chars() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("internal_non_word_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("internal_non_word_{}.json", std::process::id()));

    // Keys with internal non-word characters are perfectly valid
    let json_content = r#"{"key":"Mozilla/5.0","val":"firefox"}
{"key":"192.168.1.1","val":"local-ip"}
{"key":"api-v2-endpoint","val":"api"}
{"key":"C:\\Windows\\System32","val":"windows-path"}
{"key":"user.name@example.com","val":"email-format"}"#;
    fs::write(&json_path, json_content).unwrap();

    let output = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("key")
        .arg(&json_path)
        .output()
        .unwrap();

    assert!(output.status.success());

    let mut cache = FstCache::new();
    let fsed = FstSed::new(
        fst_path.clone(),
        Some("[{val}]".to_string()),
        termcolor::ColorChoice::Never,
    )
    .unwrap();

    // Test 1: User-Agent string
    let input1 = "User-Agent: Mozilla/5.0 (Windows NT)";
    let mut iter = fsed.find_iter(input1.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(12));
    assert_eq!(iter.get_match().display().to_string(), "[firefox]");

    // Test 2: IP address
    let input2 = "connect to 192.168.1.1 port 80";
    let mut iter = fsed.find_iter(input2.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(11));
    assert_eq!(iter.get_match().display().to_string(), "[local-ip]");

    // Test 3: API endpoint with hyphens
    let input3 = "calling api-v2-endpoint now";
    let mut iter = fsed.find_iter(input3.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(8));
    assert_eq!(iter.get_match().display().to_string(), "[api]");

    // Test 4: Windows path
    let input4 = "located in C:\\Windows\\System32 folder";
    let mut iter = fsed.find_iter(input4.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(11));
    assert_eq!(iter.get_match().display().to_string(), "[windows-path]");

    // Test 5: Email-like pattern
    let input5 = "send to user.name@example.com today";
    let mut iter = fsed.find_iter(input5.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(8));
    assert_eq!(iter.get_match().display().to_string(), "[email-format]");

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

/// Edge case: Verify that embedded alphanumeric keys don't match
#[test]
fn test_embedded_no_match() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("embedded_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("embedded_{}.json", std::process::id()));

    let json_content = r#"{"key":"cat","val":"animal"}
{"key":"dog","val":"pet"}"#;
    fs::write(&json_path, json_content).unwrap();

    let status = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("key")
        .arg(&json_path)
        .status()
        .unwrap();
    assert!(status.success());

    let mut cache = FstCache::new();
    let fsed = FstSed::new(
        fst_path.clone(),
        Some("{key}".to_string()),
        termcolor::ColorChoice::Never,
    )
    .unwrap();

    // "cat" should not match inside "catalog", "scatter", "concatenate"
    let input = "catalog scatter concatenate dogma underdog";
    let matches: Vec<_> = fsed.find_iter(input.as_bytes(), &mut cache).collect();

    // Should have zero matches because "cat" and "dog" are embedded in longer words
    assert_eq!(
        matches.len(),
        0,
        "Should not match embedded keys in: {}",
        input
    );

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}
