use camino::Utf8PathBuf;
use fstsed::engine::{FstCache, FstSed};
use std::fs;
use std::process::Command;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

#[test]
fn test_dfir_indicators() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path =
        Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("dfir_{}.fst", std::process::id()));
    let json_path =
        Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("dfir_{}.json", std::process::id()));

    let json_content = r#"{"key":"8.8.8.8","val":"google-dns"}
{"key":"avsvmcloud.com","val":"solarwinds-ioc"}
{"key":"C:\\Windows\\System32\\cmd.exe","val":"shell"}
{"key":"Mozilla/5.0","val":"useragent-prefix"}"#;
    fs::write(&json_path, json_content).unwrap();

    // Build the FST
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
        Some("{key} ({val})".to_string()),
        termcolor::ColorChoice::Never,
    )
    .unwrap();

    // 1. IP Address Tests
    let input1 = "check 8.8.8.8 for details";
    let mut iter = fsed.find_iter(input1.as_bytes(), &mut cache);
    assert!(iter.next().is_some());
    assert_eq!(
        iter.get_match().display().to_string(),
        "8.8.8.8 (google-dns)"
    );

    // 2. IP Address with trailing dot (common in logs/sentences)
    let input2 = "Found 8.8.8.8. It is bad.";
    let mut iter = fsed.find_iter(input2.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(
        iter.get_match().display().to_string(),
        "8.8.8.8 (google-dns)"
    );

    // 3. Hostname in a complex string
    let input3 = "visit avsvmcloud.com; it is dangerous";
    let mut iter = fsed.find_iter(input3.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(6));
    assert_eq!(
        iter.get_match().display().to_string(),
        "avsvmcloud.com (solarwinds-ioc)"
    );

    // 4. File Path Tests (Backslashes are now word boundaries if they aren't in is_word_char)
    // Actually \ is not in is_word_char, so it's a boundary.
    let input4 = "ran C:\\Windows\\System32\\cmd.exe now";
    let mut iter = fsed.find_iter(input4.as_bytes(), &mut cache);
    assert_eq!(iter.next(), Some(4));
    assert_eq!(
        iter.get_match().display().to_string(),
        "C:\\Windows\\System32\\cmd.exe (shell)"
    );

    // 5. Lookbehind check: exclude 18.8.8.8 but allow test.8.8.8.8
    let input5 = "exclude 18.8.8.8 but match test.8.8.8.8 and 1.8.8.8.8";
    let mut iter = fsed.find_iter(input5.as_bytes(), &mut cache);

    // First match should be test.8.8.8.8 (pos 32)
    let m1 = iter.next();
    assert_eq!(m1, Some(32));
    assert_eq!(
        iter.get_match().display().to_string(),
        "8.8.8.8 (google-dns)"
    );

    // Second match should be 1.8.8.8.8 (pos 46)
    let m2 = iter.next();
    assert_eq!(m2, Some(46));
    assert_eq!(
        iter.get_match().display().to_string(),
        "8.8.8.8 (google-dns)"
    );

    // Should be no more matches
    assert!(iter.next().is_none());

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}
