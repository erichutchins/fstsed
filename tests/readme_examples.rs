use camino::Utf8PathBuf;
use std::fs;
use std::io::Write;
use std::process::Command;

fn get_bin() -> String {
    env!("CARGO_BIN_EXE_fstsed").to_string()
}

#[test]
fn test_volexity_example() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("volexity_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("volexity_{}.json", std::process::id()));

    let json_content = r#"{"value":"avsvmcloud.com","type":"hostname","notes":null,"path":"2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv"}"#;
    fs::write(&json_path, json_content).unwrap();

    // 1. Build
    let status = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("value")
        .arg(&json_path)
        .status()
        .expect("failed to execute build");
    assert!(status.success());

    // 2. Basic find and replace
    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"test of avsvmcloud.com metadata\n").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();
    assert!(result.contains("test of <avsvmcloud.com|{"));
    assert!(result.contains("\"value\":\"avsvmcloud.com\""));
    assert!(result.contains("}> metadata"));

    // 3. Template
    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("{key} (a {type} from {path} report)")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"test of avsvmcloud.com metadata\n").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        result.trim(),
        "test of avsvmcloud.com (a hostname from 2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv report) metadata"
    );

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}

#[test]
fn test_burmese_translation_example() {
    let bin = get_bin();
    let dir = std::env::temp_dir();
    let fst_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("myanmar_{}.fst", std::process::id()));
    let json_path = Utf8PathBuf::from(dir.to_str().unwrap())
        .join(format!("burmese_{}.json", std::process::id()));

    let json_content = r#"{"key":"ဗိုလခုပမူးကီး","translated":"Senior General of Myanmar Army"}"#;
    fs::write(&json_path, json_content).unwrap();

    // 1. Build
    let status = Command::new(&bin)
        .arg("--build")
        .arg("-f")
        .arg(&fst_path)
        .arg("-k")
        .arg("key")
        .arg(&json_path)
        .status()
        .expect("failed to execute build");
    assert!(status.success());

    // 2. Search with template
    let mut child = Command::new(&bin)
        .arg("-f")
        .arg(&fst_path)
        .arg("--template")
        .arg("<{key}> ({translated})")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let input_text = "လနခဲ့တဲ့ ၅ နစက တပမတောကာကယရေးဦးစီးခုပ ရဲ့သကတမးဟာ အကန့အသတမရိတဲ့ သဘောဖစနေလို့ ၆၅ နစကန့သတပီးပငခဲ့တယလို့ ဗိုလခုပမူးကီး မငးအောငလိုငက ပောခဲ့ပီး သူ့အသက းကာ အဲ့ဒီ့ကန့သတခကကို ပယဖကလိုကတဲ့ အတက တပမတောကာကယရေးဦးစီးခုပသကတမးဟာ အကန့အသတမဲ့ ပနဖစသားပတယ။\n";
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input_text.as_bytes()).unwrap();
    }

    let out = child.wait_with_output().unwrap();
    let result = String::from_utf8(out.stdout).unwrap();
    
    let expected_part = "<ဗိုလခုပမူးကီး> (Senior General of Myanmar Army)";
    assert!(result.contains(expected_part));

    let _ = fs::remove_file(fst_path);
    let _ = fs::remove_file(json_path);
}
