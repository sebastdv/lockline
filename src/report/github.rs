//! GitHub Actions workflow commands (`--format github`): one annotation per finding, shown
//! on the pull request's changed files.

use std::fmt::Write;

use crate::analyze::{Report, Source, UnitReport};
use crate::model::{Finding, Severity};

/// Statement text quoted in Laravel annotations, which cannot point at a line of the PHP file.
const MAX_STATEMENT_CHARS: usize = 120;

pub fn render_github(report: &Report) -> String {
    let mut out = String::new();
    for unit in &report.units {
        for finding in &unit.findings {
            let line = match unit.source {
                Source::Sql => finding.line.unwrap_or(1),
                Source::Laravel => 1,
            };
            annotate(&mut out, unit, line, None, finding, None);
        }
        for stmt in &unit.statements {
            for finding in &stmt.findings {
                match unit.source {
                    Source::Sql => {
                        annotate(&mut out, unit, stmt.line, Some(stmt.col), finding, None)
                    }
                    // The line is the statement's position in Laravel's output, not in the PHP
                    // file: annotate the file and quote the statement instead.
                    Source::Laravel => annotate(&mut out, unit, 1, None, finding, Some(&stmt.text)),
                }
            }
        }
    }
    let counts = report.counts();
    let _ = writeln!(
        out,
        "lockline: {} error{}, {} warning{} in {} file{}",
        counts.errors,
        plural(counts.errors),
        counts.warnings,
        plural(counts.warnings),
        counts.files,
        plural(counts.files)
    );
    out
}

fn annotate(
    out: &mut String,
    unit: &UnitReport,
    line: usize,
    col: Option<usize>,
    finding: &Finding,
    statement: Option<&str>,
) {
    let command = match finding.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let mut properties = Vec::new();
    if unit.name != "<stdin>" {
        properties.push(format!(
            "file={}",
            escape_property(unit.name.trim_start_matches("./"))
        ));
        properties.push(format!("line={line}"));
        if let Some(col) = col {
            properties.push(format!("col={col}"));
        }
    }
    properties.push(format!(
        "title={}",
        escape_property(&format!("lockline: {}", finding.rule))
    ));

    let mut message = finding.message.clone();
    if let Some(statement) = statement {
        let _ = write!(message, "\nStatement: {}", truncate(statement));
    }
    if let Some(help) = &finding.help {
        let _ = write!(message, "\nhelp: {help}");
    }
    let _ = writeln!(
        out,
        "::{command} {}::{}",
        properties.join(","),
        escape_data(&message)
    );
}

fn truncate(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_STATEMENT_CHARS {
        flat
    } else {
        format!(
            "{}…",
            flat.chars().take(MAX_STATEMENT_CHARS).collect::<String>()
        )
    }
}

/// Escaping for the message of a workflow command.
fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escaping for `key=value` properties, which also reserve `:` and `,`.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
