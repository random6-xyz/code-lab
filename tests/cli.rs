use std::{fs, process::Command};

use tempfile::tempdir;

fn write_problem(directory: &std::path::Path) {
    fs::write(
        directory.join("sample.yaml"),
        r#"schema_version: 1
id: sample
title: Sample
statement: Read two integers and print their sum.
time_limit_ms: 1000
memory_limit_mb: 128
test_cases:
  - name: basic
    input: "2 3\n"
    expected_output: "5\n"
"#,
    )
    .expect("write test problem");
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
        "sample - Sample"
    );
}

#[test]
fn show_displays_problem_statement_and_limits() {
    let directory = tempdir().expect("temporary directory");
    write_problem(directory.path());

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
    assert!(stdout.contains("Read two integers and print their sum."));
    assert!(stdout.contains("Time limit: 1000 ms"));
    assert!(stdout.contains("Memory limit: 128 MB"));
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
