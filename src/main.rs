use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use lockline::config::{ConfigError, FileConfig, exclude_set, find_config, load_config};
use lockline::input::collect_units;
use lockline::model::Severity;
use lockline::report::{HumanOptions, render_human};
use lockline::rules::infos;
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
    /// List every rule with its default severity
    Rules,
    /// Show the documentation of one rule
    Explain {
        /// Rule id, as printed by `lockline rules`
        rule: String,
    },
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
    /// [default: error, or `fail_on` in lockline.toml]
    #[arg(long, value_enum)]
    fail_on: Option<FailOn>,
    /// How your migration tool runs each file
    /// [default: per-unit, or `transaction` in lockline.toml]
    #[arg(long, value_enum)]
    transaction: Option<Transaction>,
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
        Command::Rules => {
            for info in infos() {
                println!(
                    "{:<36} {:<8} {}",
                    info.id,
                    info.severity.as_str(),
                    info.summary
                );
            }
            ExitCode::SUCCESS
        }
        Command::Explain { rule } => match infos().into_iter().find(|i| i.id == rule) {
            Some(info) => {
                print!("{}", info.docs);
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("error: unknown rule \"{rule}\"; run \"lockline rules\" to list them");
                ExitCode::from(2)
            }
        },
    }
}

fn check(args: CheckArgs) -> ExitCode {
    let file_config = match load_project_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let exclude = match exclude_set(&file_config.exclude) {
        Ok(set) => set,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let units = match collect_units(&args.paths, &mut io::stdin()) {
        Ok(collected) => {
            for warning in &collected.warnings {
                eprintln!("warning: {warning}");
            }
            collected
                .units
                .into_iter()
                .filter(|u| u.name == "<stdin>" || !exclude.is_match(&u.name))
                .collect::<Vec<_>>()
        }
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    // Precedence: command-line flags > lockline.toml > defaults.
    let transaction = match args.transaction {
        Some(Transaction::PerUnit) => TransactionMode::PerUnit,
        Some(Transaction::None) => TransactionMode::None,
        None => file_config.transaction.unwrap_or_default(),
    };
    let threshold = match args.fail_on {
        Some(FailOn::Error) => Severity::Error,
        Some(FailOn::Warning) => Severity::Warning,
        None => file_config.fail_on.unwrap_or(Severity::Error),
    };
    let config = Config {
        transaction,
        rules: file_config.rules.into_iter().collect(),
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

    if report.max_severity().is_some_and(|s| s >= threshold) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// The closest `lockline.toml` from the current directory up, or defaults.
fn load_project_config() -> Result<FileConfig, ConfigError> {
    let cwd = std::env::current_dir().map_err(|source| ConfigError::Io {
        path: ".".to_string(),
        source,
    })?;
    match find_config(&cwd) {
        Some(path) => load_config(&path),
        None => Ok(FileConfig::default()),
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
