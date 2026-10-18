use std::io::{self, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use globset::GlobSet;

use lockline::config::{ConfigError, FileConfig, exclude_set, find_config, load_config};
use lockline::input::{DEFAULT_MIGRATIONS_DIR, collect_history, collect_laravel, collect_units};
use lockline::model::Severity;
use lockline::report::{HumanOptions, render_github, render_human, render_json, render_sarif};
use lockline::rules::infos;
use lockline::{Config, TransactionMode, analyze_with_history};

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
    /// Analyze migration files, directories (searched for *.sql), stdin (`-`), or the output
    /// of `php artisan migrate --pretend` (`--from laravel`)
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
    /// Output format (`--quiet` and `--color` only apply to human)
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,
    /// What the input is: SQL, or the saved/piped output of `php artisan migrate --pretend`
    #[arg(long, value_enum, default_value_t = From::Sql)]
    from: From,
    /// With `--from laravel`: folder of the PHP migrations, so findings name their file
    #[arg(long, value_name = "DIR", default_value = DEFAULT_MIGRATIONS_DIR)]
    laravel_dir: String,
    /// Migrations that already ran, read only to learn the schema and never reported: a
    /// directory of .sql files (those that sort before the ones checked) or one file, such as
    /// a schema dump (`php artisan schema:dump`, `pg_dump --schema-only`)
    #[arg(long, value_name = "PATH")]
    history: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
    /// SARIF 2.1.0, for GitHub code scanning
    Sarif,
    /// GitHub Actions annotations on the pull request
    Github,
}

#[derive(Clone, Copy, ValueEnum)]
enum From {
    /// SQL files, directories of .sql files, or SQL on stdin
    Sql,
    /// Output of `php artisan migrate --pretend` (a saved file, or `-` for a pipe)
    Laravel,
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
    let (config_dir, file_config) = match load_project_config() {
        Ok(found) => found,
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
    let collected = match args.from {
        From::Sql => collect_units(&args.paths, &mut io::stdin()),
        From::Laravel => collect_laravel(&args.paths, &mut io::stdin(), &args.laravel_dir),
    };
    let units = match collected {
        Ok(collected) => {
            for warning in &collected.warnings {
                eprintln!("warning: {warning}");
            }
            collected
                .units
                .into_iter()
                .filter(|u| {
                    u.name == "<stdin>" || !is_excluded(&exclude, config_dir.as_deref(), &u.name)
                })
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
    let history = match &args.history {
        None => Vec::new(),
        Some(path) => match collect_history(path, &units) {
            Ok(collected) => collected.units,
            Err(error) => {
                eprintln!("error: --history {error}");
                return ExitCode::from(2);
            }
        },
    };
    let report = analyze_with_history(&history, &units, &config);
    let output = match args.format {
        Format::Human => render_human(
            &report,
            &HumanOptions {
                color: use_color(args.color),
                quiet: args.quiet,
            },
        ),
        Format::Json => render_json(&report),
        Format::Sarif => render_sarif(&report),
        Format::Github => render_github(&report),
    };
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

/// The closest `lockline.toml` from the current directory up (and its directory), or
/// defaults.
fn load_project_config() -> Result<(Option<PathBuf>, FileConfig), ConfigError> {
    let cwd = std::env::current_dir().map_err(|source| ConfigError::Io {
        path: ".".to_string(),
        source,
    })?;
    match find_config(&cwd) {
        Some(path) => {
            let config = load_config(&path)?;
            Ok((path.parent().map(Path::to_path_buf), config))
        }
        None => Ok((None, FileConfig::default())),
    }
}

/// `exclude` patterns are relative to the directory of `lockline.toml`, however the path was
/// typed (`m/x.sql`, `./m/x.sql`, an absolute path, or from a subdirectory).
fn is_excluded(exclude: &GlobSet, config_dir: Option<&Path>, name: &str) -> bool {
    let path = Path::new(name);
    // canonicalize resolves symlinks too (macOS /var -> /private/var); fall back to a
    // lexical normalization when a path cannot be resolved.
    let resolve = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| normalize(p));
    let absolute = resolve(path);
    let relative = config_dir
        .map(resolve)
        .and_then(|dir| absolute.strip_prefix(&dir).ok().map(Path::to_path_buf));
    exclude.is_match(normalize(path)) || relative.is_some_and(|r| exclude.is_match(r))
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
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
