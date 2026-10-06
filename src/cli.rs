use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "code-lab", version, about = "A local coding problem runner")]
pub struct Cli {
    /// Directory containing problem YAML files.
    #[arg(long, global = true, default_value = "problems")]
    pub problems_dir: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List available problems.
    List,
    /// Display a problem statement and its limits.
    Show {
        /// Problem identifier.
        id: String,
    },
    /// Run a single-file solution against a problem.
    Run {
        /// Problem identifier.
        id: String,
        /// Path to a Python, C, or Rust source file.
        source: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};

    #[test]
    fn parses_run_command_and_problem_directory() {
        let cli = Cli::try_parse_from([
            "code-lab",
            "--problems-dir",
            "fixtures/problems",
            "run",
            "sample",
            "solution.py",
        ])
        .expect("valid command line");

        assert_eq!(
            cli.problems_dir,
            std::path::PathBuf::from("fixtures/problems")
        );
        assert!(matches!(cli.command, Command::Run { id, .. } if id == "sample"));
    }
}
