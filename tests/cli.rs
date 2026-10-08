use std::{fs, process::Command};

use tempfile::tempdir;

fn write_problem_with_difficulty(
    directory: &std::path::Path,
    id: &str,
    title: &str,
    difficulty: &str,
) {
    let yaml = format!(
        r#"schema_version: 1
id: {id}
difficulty: {difficulty}
title: {title}
statement: Read two integers and print their sum.
time_limit_ms: 1000
memory_limit_mb: 128
test_cases:
  - name: basic
    input: "2 3\n"
    expected_output: "5\n"
  - name: negative
    input: "-4 9\n"
    expected_output: "5\n"
  - name: zero
    input: "0 0\n"
    expected_output: "0\n"
  - name: one-negative
    input: "-9 2\n"
    expected_output: "-7\n"
  - name: cancellation
    input: "7 -7\n"
    expected_output: "0\n"
  - name: large-sum
    input: "1000000 2000000\n"
    expected_output: "3000000\n"
"#
    );
    fs::write(directory.join(format!("{id}.yaml")), yaml).expect("write test problem");
}

fn write_problem(directory: &std::path::Path) {
    write_problem_with_difficulty(directory, "sample", "Sample", "easy");
}

fn write_problem_with_tags(directory: &std::path::Path) {
    write_problem(directory);
    let path = directory.join("sample.yaml");
    let contents = fs::read_to_string(&path).expect("read test problem");
    let tagged = contents.replacen(
        "statement: Read two integers and print their sum.\n",
        "statement: Read two integers and print their sum.\ntags:\n  - arithmetic\n  - beginner\n",
        1,
    );
    assert_ne!(tagged, contents, "statement should be present in test YAML");
    fs::write(path, tagged).expect("add test tags");
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_code-lab"))
}

#[test]
fn list_displays_problem_ids_and_titles() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Easy\n  sample - Sample"
    );
}

#[test]
fn list_groups_problems_by_difficulty() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());
    write_problem_with_difficulty(directory.path(), "alpha-easy", "Alpha Easy", "easy");
    write_problem_with_difficulty(directory.path(), "middle", "Middle", "medium");
    write_problem_with_difficulty(directory.path(), "advanced", "Advanced", "hard");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Easy\n  alpha-easy - Alpha Easy\n  sample - Sample\n\nMedium\n  middle - Middle\n\nHard\n  advanced - Advanced"
    );
}

#[test]
fn list_loads_yaml_from_nested_directories() {
    let directory = tempdir().expect("temporary directory");
    let nested = directory.path().join("arrays").join("intro");
    fs::create_dir_all(&nested).expect("create nested problem directory");
    write_problem(&nested);
    fs::rename(nested.join("sample.yaml"), nested.join("sample.YML"))
        .expect("rename nested problem file");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Easy\n  sample - Sample"
    );
}

#[test]
fn list_rejects_unknown_difficulty() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());
    let path = directory.path().join("sample.yaml");
    let yaml = fs::read_to_string(&path)
        .expect("read test problem")
        .replacen("difficulty: easy", "difficulty: expert", 1);
    fs::write(path, yaml).expect("set unknown difficulty");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(!output.status.success());
}

#[test]
fn list_rejects_problem_with_fewer_than_six_test_cases() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());
    let path = directory.path().join("sample.yaml");
    let yaml = fs::read_to_string(&path).expect("read test problem");
    let truncated = yaml
        .split("  - name: large-sum\n")
        .next()
        .expect("find final test case");
    fs::write(path, truncated).expect("remove final test case");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("at least 6 test cases"));
}

#[test]
fn list_validates_yaml_in_nested_directories() {
    let directory = tempdir().expect("temporary directory");
    let nested = directory.path().join("arrays");
    fs::create_dir_all(&nested).expect("create nested problem directory");
    write_problem(&nested);
    let path = nested.join("sample.yaml");
    let yaml = fs::read_to_string(&path)
        .expect("read test problem")
        .replacen("schema_version: 1", "schema_version: 2", 1);
    fs::write(path, yaml).expect("make nested problem invalid");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported schema_version"));
}

#[test]
fn show_displays_problem_statement_tags_and_limits() {
    let directory = tempdir().expect("temporary directory");
    write_problem_with_tags(directory.path());

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("show")
        .arg("sample")
        .output()
        .expect("run show command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sample - Sample"));
    assert!(stdout.contains("Difficulty: Easy"));
    assert!(stdout.contains("Read two integers and print their sum."));
    assert!(stdout.contains("Tags: arithmetic, beginner"));
    assert!(stdout.contains("Time limit: 1000 ms"));
    assert!(stdout.contains("Memory limit: 128 MB"));
    assert!(stdout.contains("Test cases: 6"));
    assert!(stdout.contains("Example: basic"));
    assert!(stdout.contains("Input:\n```text\n2 3\n```"));
    assert!(stdout.contains("Expected output:\n```text\n5\n```"));
    assert!(!stdout.contains("Example: negative"));
    assert!(!stdout.contains("-4 9"));
}

#[test]
fn run_fails_closed_when_nsjail_is_not_on_path() {
    let directory = tempdir().expect("temporary directory");
    let problems = directory.path().join("problems");
    let empty_path = directory.path().join("empty-path");
    fs::create_dir(&problems).expect("create problems directory");
    fs::create_dir(&empty_path).expect("create empty PATH directory");
    write_problem(&problems);
    let source = directory.path().join("solution.py");
    fs::write(&source, "print('must not run')\n").expect("write solution");

    let output = cli()
        .env("PATH", &empty_path)
        .arg("--problems-dir")
        .arg(&problems)
        .arg("run")
        .arg("sample")
        .arg(source)
        .output()
        .expect("run command without nsjail on PATH");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nsjail is required"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("must not run"));
}

#[test]
fn list_rejects_a_problem_file_whose_name_does_not_match_its_id() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());
    fs::rename(
        directory.path().join("sample.yaml"),
        directory.path().join("other.yaml"),
    )
    .expect("rename test problem");

    let output = cli()
        .arg("--problems-dir")
        .arg(directory.path())
        .arg("list")
        .output()
        .expect("run list command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must match its file name"));
}
