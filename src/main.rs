mod cli;
mod problem;
mod runner;

use anyhow::Result;
use clap::Parser;

use crate::{
    cli::{Cli, Command},
    problem::{find_problem, load_problems},
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let problems = load_problems(&cli.problems_dir)?;

    match cli.command {
        Command::List => {
            if problems.is_empty() {
                println!("No problems found in {}", cli.problems_dir.display());
            } else {
                for problem in problems {
                    println!("{} - {}", problem.id, problem.title);
                }
            }
        }
        Command::Show { id } => {
            let problem = find_problem(&problems, &id)?;
            println!("{} - {}\n", problem.id, problem.title);
            println!("{}\n", problem.statement);
            if problem.tags.is_empty() {
                println!("Tags: (none)");
            } else {
                println!("Tags: {}", problem.tags.join(", "));
            }
            println!("Time limit: {} ms", problem.time_limit_ms);
            println!("Memory limit: {} MB", problem.memory_limit_mb);
            println!("Test cases: {}", problem.test_cases.len());
        }
        Command::Run { id, source } => {
            let problem = find_problem(&problems, &id)?;
            runner::run_solution(problem, &source)?;
        }
    }

    Ok(())
}
