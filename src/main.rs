mod cli;
mod problem;
mod runner;

use anyhow::Result;
use clap::Parser;

use crate::{
    cli::{Cli, Command},
    problem::{Difficulty, find_problem, load_problems},
};

fn print_example_block(label: &str, content: &str) {
    println!("{label}:");
    println!("```text");
    print!("{content}");
    if !content.ends_with('\n') {
        println!();
    }
    println!("```");
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let problems = load_problems(&cli.problems_dir)?;

    match cli.command {
        Command::List => {
            if problems.is_empty() {
                println!("No problems found in {}", cli.problems_dir.display());
            } else {
                let mut printed_group = false;
                for difficulty in Difficulty::ALL {
                    let group: Vec<_> = problems
                        .iter()
                        .filter(|problem| problem.difficulty == difficulty)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    if printed_group {
                        println!();
                    }
                    println!("{}", difficulty.label());
                    for problem in group {
                        println!("  {} - {}", problem.id, problem.title);
                    }
                    printed_group = true;
                }
            }
        }
        Command::Show { id } => {
            let problem = find_problem(&problems, &id)?;
            println!("{} - {}\n", problem.id, problem.title);
            println!("Difficulty: {}", problem.difficulty.label());
            println!("{}\n", problem.statement);
            if problem.tags.is_empty() {
                println!("Tags: (none)");
            } else {
                println!("Tags: {}", problem.tags.join(", "));
            }
            println!("Time limit: {} ms", problem.time_limit_ms);
            println!("Memory limit: {} MB", problem.memory_limit_mb);
            println!("Test cases: {}", problem.test_cases.len());
            if let Some(test_case) = problem.test_cases.first() {
                println!("\nExample: {}", test_case.name);
                print_example_block("Input", &test_case.input);
                println!();
                print_example_block("Expected output", &test_case.expected_output);
            }
        }
        Command::Run { id, source } => {
            let problem = find_problem(&problems, &id)?;
            runner::run_solution(problem, &source)?;
        }
    }

    Ok(())
}
