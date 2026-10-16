use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use lockline::input::collect_units;
use lockline::model::Severity;
use lockline::report::{HumanOptions, render_human};
use lockline::{Config, TransactionMode, analyze};

#[derive(Parser)]
#[command(
    name = "lockline",
    version,
    about = "Explains the locks your PostgreSQL migrations take and flags the dangerous ones"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze migration files, directories (searched for *.sql) or stdin (`-`)
    Check(CheckArgs),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Files, directories, or `-` for stdin
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Only show statements with findings
    #[arg(long, short)]
    quiet: bool,
    /// Exit with status 1 when there are findings of this severity or higher
    #[arg(long, value_enum, default_value_t = FailOn::Error)]
    fail_on: FailOn,
    /// How your migration tool runs each file
    #[arg(long, value_enum, default_value_t = Transaction::PerUnit)]
    transaction: Transaction,
    /// When to use colors
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    color: Color,
}

#[derive(Clone, Copy, ValueEnum)]
enum FailOn {
    Error,
    Warning,
}

#[derive(Clone, Copy, ValueEnum)]
enum Transaction {
    /// Each file runs inside one transaction (Laravel, Flyway, Rails, Django…)
    PerUnit,
    /// Autocommit, like `psql -f`
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check(args) => check(args),
    }
}

fn check(args: CheckArgs) -> ExitCode {
    let units = match collect_units(&args.paths, &mut io::stdin()) {
        Ok(collected) => {
            for warning in &collected.warnings {
                eprintln!("warning: {warning}");
            }
            collected.units
        }
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let config = Config {
        transaction: match args.transaction {
            Transaction::PerUnit => TransactionMode::PerUnit,
            Transaction::None => TransactionMode::None,
        },
    };
    let report = analyze(&units, &config);
    let options = HumanOptions {
        color: use_color(args.color),
        quiet: args.quiet,
    };
    let output = render_human(&report, &options);
    if let Err(error) = io::stdout().lock().write_all(output.as_bytes())
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        eprintln!("error: {error}");
        return ExitCode::from(2);
    }

    let threshold = match args.fail_on {
        FailOn::Error => Severity::Error,
        FailOn::Warning => Severity::Warning,
    };
    if report.max_severity().is_some_and(|s| s >= threshold) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn use_color(choice: Color) -> bool {
    match choice {
        Color::Always => true,
        Color::Never => false,
        Color::Auto => {
            io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        }
    }
}
