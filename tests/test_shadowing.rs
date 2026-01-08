use camino::Utf8PathBuf;
use std::fs;
use std::process::Command;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

#[test]
fn test_shadowing_behavior() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("shadow_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("shadow_{}.json", std::process::id()));

    // Case 1: ABC and ABCDE. ABC is now shadowed by ABCDE (simplified rule).
    // Case 2: ABC and ABC-DE. ABC is shadowed by ABC-DE.
    // Case 3: A, A-B, A-B-C. A and A-B are both shadowed by A-B-C.
    // Case 4: !EXCLAMATION starts with non-word char.
    // Case 5: "TRAILING " ends with non-word char.
    let json_content = r#"{"key":"ABC","val":"1"}
{"key":"ABCDE","val":"2"}
{"key":"ABC-DE","val":"3"}
{"key":"A","val":"4"}
{"key":"A-B","val":"5"}
{"key":"A-B-C","val":"6"}
{"key":"!EXCLAMATION","val":"7"}
{"key":"TRAILING ","val":"8"}"#;
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
    
    // Check for Shadowing warnings
    // Shadowed: A, A-B, ABC
    assert!(stderr.contains("Warning: 3 keys are shadowed"));
    assert!(stderr.contains("- ABC"));
    assert!(stderr.contains("- A"));
    assert!(stderr.contains("- A-B"));
    
    // Check for Non-word character warning
    // Only !EXCLAMATION and "TRAILING " start or end with non-word chars.
    // ABC-DE, A-B, A-B-C, KEY WORD are internal.
    assert!(stderr.contains("Warning: 2 keys start or end with non-word characters"));
    assert!(stderr.contains("- !EXCLAMATION"));
    assert!(stderr.contains("- TRAILING "));

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}
