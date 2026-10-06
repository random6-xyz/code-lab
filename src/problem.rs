use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const MAX_TIME_LIMIT_MS: u64 = 300_000;
const MAX_MEMORY_LIMIT_MB: u64 = 8_192;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub statement: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub time_limit_ms: u64,
    pub memory_limit_mb: u64,
    pub test_cases: Vec<TestCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
    pub name: String,
    pub input: String,
    pub expected_output: String,
}

impl Problem {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "problem '{}' uses unsupported schema_version {}; supported version is 1",
                self.id,
                self.schema_version
            );
        }
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            bail!(
                "problem id '{}' must contain only lowercase letters, digits, and hyphens",
                self.id
            );
        }
        if self.title.trim().is_empty() {
            bail!("problem '{}' must have a non-empty title", self.id);
        }
        if self.statement.trim().is_empty() {
            bail!("problem '{}' must have a non-empty statement", self.id);
        }
        let mut tags = HashSet::new();
        for tag in &self.tags {
            if tag.trim().is_empty() {
                bail!("problem '{}' has an empty tag", self.id);
            }
            if !tags.insert(tag) {
                bail!("problem '{}' has duplicate tag '{}'", self.id, tag);
            }
        }
        if !(1..=MAX_TIME_LIMIT_MS).contains(&self.time_limit_ms) {
            bail!(
                "problem '{}' time_limit_ms must be between 1 and {MAX_TIME_LIMIT_MS}",
                self.id
            );
        }
        if !(1..=MAX_MEMORY_LIMIT_MB).contains(&self.memory_limit_mb) {
            bail!(
                "problem '{}' memory_limit_mb must be between 1 and {MAX_MEMORY_LIMIT_MB}",
                self.id
            );
        }
        if self.test_cases.is_empty() {
            bail!("problem '{}' must define at least one test case", self.id);
        }

        let mut names = HashSet::new();
        for test_case in &self.test_cases {
            if test_case.name.trim().is_empty() {
                bail!("problem '{}' has a test case with an empty name", self.id);
            }
            if !names.insert(&test_case.name) {
                bail!(
                    "problem '{}' has duplicate test case name '{}'",
                    self.id,
                    test_case.name
                );
            }
        }
        Ok(())
    }
}

pub fn load_problem(path: &Path) -> Result<Problem> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read problem file {}", path.display()))?;
    let problem: Problem = serde_yaml_ng::from_str(&contents)
        .with_context(|| format!("failed to parse problem YAML {}", path.display()))?;
    problem
        .validate()
        .with_context(|| format!("invalid problem file {}", path.display()))?;
    Ok(problem)
}

fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
}

fn collect_problem_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read problems directory {}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect problem path {}", path.display()))?;
        if file_type.is_dir() {
            collect_problem_files(&path, paths)?;
        } else if file_type.is_file() && is_yaml_file(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

pub fn load_problems(directory: &Path) -> Result<Vec<Problem>> {
    let mut paths = Vec::new();
    collect_problem_files(directory, &mut paths)?;
    paths.sort();

    let mut problems = Vec::with_capacity(paths.len());
    let mut ids = HashSet::new();
    for path in paths {
        let problem = load_problem(&path)?;
        let file_stem = path.file_stem().and_then(|stem| stem.to_str());
        if file_stem != Some(problem.id.as_str()) {
            bail!(
                "problem id '{}' must match its file name '{}'",
                problem.id,
                path.display()
            );
        }
        if !ids.insert(problem.id.clone()) {
            bail!("duplicate problem id '{}'", problem.id);
        }
        problems.push(problem);
    }
    problems.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(problems)
}

pub fn find_problem<'a>(problems: &'a [Problem], id: &str) -> Result<&'a Problem> {
    problems
        .iter()
        .find(|problem| problem.id == id)
        .with_context(|| format!("problem '{id}' was not found"))
}

#[cfg(test)]
mod tests {
    use super::{Problem, TestCase};

    fn valid_problem() -> Problem {
        Problem {
            schema_version: 1,
            id: "sample".to_owned(),
            title: "Sample".to_owned(),
            statement: "Read two numbers and print their sum.".to_owned(),
            tags: Vec::new(),
            time_limit_ms: 1_000,
            memory_limit_mb: 128,
            test_cases: vec![TestCase {
                name: "example".to_owned(),
                input: "2 3\n".to_owned(),
                expected_output: "5\n".to_owned(),
            }],
        }
    }

    #[test]
    fn accepts_valid_problem() {
        assert!(valid_problem().validate().is_ok());
    }

    #[test]
    fn rejects_empty_and_duplicate_tags() {
        let mut problem = valid_problem();
        problem.tags.push("  ".to_owned());
        assert!(
            problem
                .validate()
                .unwrap_err()
                .to_string()
                .contains("empty tag")
        );

        let mut problem = valid_problem();
        problem.tags = vec!["arrays".to_owned(), "arrays".to_owned()];
        assert!(
            problem
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate tag")
        );
    }

    #[test]
    fn rejects_unsupported_schema() {
        let mut problem = valid_problem();
        problem.schema_version = 2;
        assert!(
            problem
                .validate()
                .unwrap_err()
                .to_string()
                .contains("schema_version")
        );
    }

    #[test]
    fn rejects_empty_tests_and_duplicate_names() {
        let mut problem = valid_problem();
        problem.test_cases.clear();
        assert!(
            problem
                .validate()
                .unwrap_err()
                .to_string()
                .contains("at least one")
        );

        let mut problem = valid_problem();
        problem.test_cases.push(problem.test_cases[0].clone());
        assert!(
            problem
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate test case")
        );
    }
}
