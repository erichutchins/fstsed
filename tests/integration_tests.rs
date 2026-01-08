use std::process::Command;
use std::fs;
use std::io::Write;
use camino::Utf8PathBuf;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

fn create_temp_json(name: &str) -> Utf8PathBuf {
    let dir = std::env::temp_dir();
    let path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("test_{}_{}.json", name, std::process::id()));
    let mut file = fs::File::create(&path).unwrap();
    writeln!(file, r#"{{"key": "apple", "value": "A"}}"#).unwrap();
    writeln!(file, r#"{{"key": "banana", "value": "B"}}"#).unwrap();
    path
}

#[test]
fn test_cli_build_and_search() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("test_cli_{}.fst", std::process::id()));
    let json_path = create_temp_json("basic");

    // 1. Build
    let status = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg(&json_path)
        .status()
        .expect("failed to execute build");
    assert!(status.success());

    // 2. Search
    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("<{key}|{/value}>")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = output.stdin.as_ref().unwrap();
        stdin.write_all(b"i like apple and banana\n").unwrap();
    }

    let out = output.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();

    assert!(result.contains("<apple|A>"));
    assert!(result.contains("<banana|B>"));

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

#[test]
fn test_cli_template() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("test_tmpl_{}.fst", std::process::id()));
    let json_path = create_temp_json("tmpl");

    Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg(&json_path)
        .status()
        .unwrap();

    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("FOUND[{key}]->{/value}")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"apple banana\n").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();

    assert!(result.contains("FOUND[apple]->A"));
    assert!(result.contains("FOUND[banana]->B"));

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

#[test]
fn test_cli_json_mode() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("test_json_{}.fst", std::process::id()));
    let json_path = create_temp_json("json");

    Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg(&json_path)
        .status()
        .unwrap();

    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--json")
        .arg("--template")
        .arg("<{key}|{/value}>")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        // search inside a json string
        stdin.write_all(r#"{"text": "i want an apple"}"#.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();

    // The output should be valid JSON with the replaced content
    assert!(result.contains(r#""text": "i want an <apple|A>""#));

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

#[test]
fn test_cli_only_matching() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!("test_only_{}.fst", std::process::id()));
    let json_path = create_temp_json("only");

    Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg(&json_path)
        .status()
        .unwrap();

    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--only-matching")
        .arg("--template")
        .arg("<{key}|{/value}>")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"apple banana cherry\n").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();

    assert_eq!(result, "<apple|A>\n<banana|B>\n");

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}
