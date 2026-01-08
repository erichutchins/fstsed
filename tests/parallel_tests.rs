use camino::Utf8PathBuf;
use std::fs;
use std::io::Write;
use std::process::Command;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

fn create_temp_fst(name: &str) -> Utf8PathBuf {
    let dir = std::env::temp_dir();
    let bin = get_bin();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap()).join(format!(
        "test_parallel_{}_{}.fst",
        name,
        std::process::id()
    ));
    let json_path = dir.join(format!("test_parallel_data_{}.json", std::process::id()));

    let mut file = fs::File::create(&json_path).unwrap();
    writeln!(file, r#"{{"key": "foo", "value": "FOO"}}"#).unwrap();
    writeln!(file, r#"{{"key": "bar", "value": "BAR"}}"#).unwrap();

    let status = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg(&json_path)
        .status()
        .expect("failed to execute build");
    assert!(status.success());

    let _ = fs::remove_file(json_path);
    fst_path
}

#[test]
fn test_recursive_directory_walk() {
    let bin = get_bin();
    // Create a temporary directory structure
    // temp/
    //   a.txt      (contains "foo")
    //   subdir/
    //     b.txt    (contains "bar")
    //     ignore.txt (contains "foo", but should be ignored)
    //   .gitignore (ignores ignore.txt)

    let root_dir = std::env::temp_dir().join(format!("test_recursive_{}", std::process::id()));
    fs::create_dir(&root_dir).unwrap();
    let sub_dir = root_dir.join("subdir");
    fs::create_dir(&sub_dir).unwrap();

    fs::write(root_dir.join("a.txt"), "hello foo world").unwrap();
    fs::write(sub_dir.join("b.txt"), "hello bar world").unwrap();
    fs::write(sub_dir.join("ignore.txt"), "should ignore foo").unwrap();
    // Use .ignore instead of .gitignore so it works outside of a git repo (like in temp dir)
    fs::write(root_dir.join(".ignore"), "ignore.txt").unwrap();

    let fst_path = create_temp_fst("recursive");

    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("<{key}|{/value}>")
        .arg(&root_dir)
        .output()
        .expect("failed to execute fstsed");

    let stdout = String::from_utf8(output.stdout).unwrap();

    // Check results
    // Should contain modified content for a.txt and b.txt
    if !stdout.contains("hello <foo|FOO> world") {
        println!("STDOUT: {}", stdout);
    }
    assert!(stdout.contains("hello <foo|FOO> world"));
    assert!(stdout.contains("hello <bar|BAR> world"));

    // Should NOT contain content from ignore.txt
    assert!(!stdout.contains("should ignore <foo|FOO>"));

    // Clean up
    let _ = fs::remove_dir_all(root_dir);
    let _ = fs::remove_file(fst_path);
}

#[test]
fn test_explicit_multiple_files_parallel() {
    let bin = get_bin();
    let root_dir = std::env::temp_dir().join(format!("test_multi_{}", std::process::id()));
    fs::create_dir(&root_dir).unwrap();

    let file1 = root_dir.join("1.txt");
    let file2 = root_dir.join("2.txt");

    fs::write(&file1, "foo").unwrap();
    fs::write(&file2, "bar").unwrap();

    let fst_path = create_temp_fst("multi");

    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("<{key}|{/value}>")
        .arg(file1.to_str().unwrap())
        .arg(file2.to_str().unwrap())
        .output()
        .expect("failed to execute fstsed");

    let stdout = String::from_utf8(output.stdout).unwrap();

    // Order is non-deterministic, but both should be present
    if !stdout.contains("<foo|FOO>") {
        println!("STDOUT: {}", stdout);
    }
    assert!(stdout.contains("<foo|FOO>"));
    assert!(stdout.contains("<bar|BAR>"));

    let _ = fs::remove_dir_all(root_dir);
    let _ = fs::remove_file(fst_path);
}

#[test]
fn test_cli_flags_ignore_and_hidden() {
    let bin = get_bin();
    let root_dir = std::env::temp_dir().join(format!("test_flags_{}", std::process::id()));
    fs::create_dir(&root_dir).unwrap();

    // Create: visible, ignored, hidden
    fs::write(root_dir.join("visible.txt"), "content visible").unwrap();
    fs::write(root_dir.join("ignored.txt"), "content secret_ignored").unwrap();
    fs::write(root_dir.join(".hidden"), "content hidden").unwrap();
    fs::write(root_dir.join(".ignore"), "ignored.txt").unwrap();

    // FST with keys for all
    let fst_path = {
        let dir = std::env::temp_dir();
        let fst_path = dir.join(format!("test_flags_fst_{}.fst", std::process::id()));
        let json_path = dir.join(format!("test_flags_data_{}.json", std::process::id()));
        let mut file = fs::File::create(&json_path).unwrap();
        writeln!(file, r#"{{"key": "visible", "value": "VISIBLE"}}"#).unwrap();
        writeln!(file, r#"{{"key": "secret_ignored", "value": "IGNORED"}}"#).unwrap();
        writeln!(file, r#"{{"key": "hidden", "value": "HIDDEN"}}"#).unwrap();

        // Build FST
        let status = Command::new(&bin)
            .arg("--build")
            .arg("-f")
            .arg(&fst_path)
            .arg(&json_path)
            .status()
            .expect("build failed");
        assert!(status.success());
        let _ = fs::remove_file(json_path);
        fst_path
    };

    // 1. Default run: Should only find visible
    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg(&root_dir)
        .output()
        .expect("run failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("VISIBLE"));
    assert!(!stdout.contains("IGNORED"));
    assert!(!stdout.contains("HIDDEN"));

    // 2. No-ignore run (-u): Should find visible and ignored
    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("-u") // --no-ignore
        .arg(&root_dir)
        .output()
        .expect("run failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("VISIBLE"));
    assert!(stdout.contains("IGNORED"));
    assert!(!stdout.contains("HIDDEN"));

    // 3. Hidden run (-.): Should find visible and hidden
    let output = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("-.") // --hidden
        .arg(&root_dir)
        .output()
        .expect("run failed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("VISIBLE"));
    assert!(!stdout.contains("IGNORED"));
    assert!(stdout.contains("HIDDEN"));

    // Cleanup
    let _ = fs::remove_dir_all(root_dir);
    let _ = fs::remove_file(fst_path);
}
