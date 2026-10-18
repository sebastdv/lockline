//! The default, human-readable report: every statement with its lock, then the findings.

use std::fmt::Write;

use owo_colors::OwoColorize;

use crate::analyze::{Report, Source, StatementReport, TransactionSummary, UnitReport};
use crate::locks::LockMode;
use crate::model::{Analysis, Cost, Effect, Finding, Severity};

const MAX_TEXT_CHARS: usize = 80;
const DETAIL_INDENT: &str = "       ";
const HELP_INDENT: &str = "             ";

pub struct HumanOptions {
    pub color: bool,
    /// Only show statements (and units) that have findings.
    pub quiet: bool,
}

pub fn render_human(report: &Report, opts: &HumanOptions) -> String {
    let blocks: Vec<String> = report
        .units
        .iter()
        .filter(|unit| !opts.quiet || unit.all_findings().next().is_some())
        .map(|unit| render_unit(unit, opts))
        .collect();
    let mut out = blocks.join("\n");
    if !blocks.is_empty() {
        out.push('\n');
    }
    out.push_str(&summary(report));
    out.push('\n');
    out
}

fn render_unit(unit: &UnitReport, opts: &HumanOptions) -> String {
    let mut out = match unit.source {
        Source::Sql => format!("{}\n", unit.name),
        // Positions count statements in Laravel's output, not lines of the PHP file.
        Source::Laravel => format!("{} (SQL from migrate --pretend)\n", unit.name),
    };
    for finding in &unit.findings {
        render_finding(&mut out, finding, opts);
    }
    let mut blocks: Vec<String> = Vec::new();
    for (index, stmt) in unit.statements.iter().enumerate() {
        if opts.quiet && stmt.findings.is_empty() {
            continue;
        }
        let mut block = String::new();
        render_statement(&mut block, stmt, opts);
        if !opts.quiet {
            for summary in unit
                .transactions
                .iter()
                .filter(|t| t.ends_after == Some(index))
            {
                render_held_locks(&mut block, summary);
            }
        }
        blocks.push(block);
    }
    if !opts.quiet {
        for summary in unit.transactions.iter().filter(|t| t.ends_after.is_none()) {
            let mut block = String::new();
            render_held_locks(&mut block, summary);
            blocks.push(block);
        }
    }
    out.push_str(&blocks.join("\n"));
    out
}

fn render_held_locks(out: &mut String, summary: &TransactionSummary) {
    let locks: Vec<String> = summary
        .locks
        .iter()
        .map(|h| format!("{} on {}", h.lock.as_str(), h.table))
        .collect();
    let _ = writeln!(out, "  holds until commit: {}", locks.join(", "));
}

fn render_statement(out: &mut String, stmt: &StatementReport, opts: &HumanOptions) {
    let _ = writeln!(
        out,
        "  {}:{}  {}",
        stmt.line,
        stmt.col,
        summarize(&stmt.text)
    );
    match &stmt.analysis {
        Analysis::Locks(effects) => {
            for effect in effects {
                render_effect(out, effect);
            }
        }
        Analysis::NotAnalyzed => {
            let _ = writeln!(out, "{DETAIL_INDENT}lock   not analyzed");
        }
        Analysis::NoTableLock => {}
    }
    for finding in &stmt.findings {
        render_finding(out, finding, opts);
    }
}

fn render_effect(out: &mut String, effect: &Effect) {
    let _ = writeln!(
        out,
        "{DETAIL_INDENT}lock   {} on {} — {}{}",
        effect.lock.as_str(),
        effect.table,
        blocking(effect.lock),
        if effect.new_table { " (new table)" } else { "" }
    );
    let note = effect
        .note
        .as_ref()
        .map(|n| format!(" ({n})"))
        .unwrap_or_default();
    let _ = writeln!(out, "{DETAIL_INDENT}cost   {}{note}", cost(effect.cost));
}

fn render_finding(out: &mut String, finding: &Finding, opts: &HumanOptions) {
    let severity = finding.severity.as_str();
    let severity = match (opts.color, finding.severity) {
        (false, _) => severity.to_string(),
        (true, Severity::Error) => severity.red().bold().to_string(),
        (true, Severity::Warning) => severity.yellow().bold().to_string(),
    };
    let rule = if opts.color {
        finding.rule.bold().to_string()
    } else {
        finding.rule.to_string()
    };
    let _ = writeln!(out, "  {severity}[{rule}] {}", finding.message);
    if let Some(help) = &finding.help {
        let help = help.replace('\n', &format!("\n{HELP_INDENT}"));
        let help = if opts.color {
            help.cyan().to_string()
        } else {
            help
        };
        let _ = writeln!(out, "{DETAIL_INDENT}help: {help}");
    }
}

/// Collapses whitespace and truncates long statements to keep one line per statement.
fn summarize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_TEXT_CHARS {
        let head: String = collapsed.chars().take(MAX_TEXT_CHARS - 1).collect();
        format!("{head}…")
    } else {
        collapsed
    }
}

fn blocking(lock: LockMode) -> &'static str {
    match (lock.blocks_reads(), lock.blocks_writes()) {
        (true, _) => "blocks reads and writes",
        (false, true) => "blocks writes, allows reads",
        (false, false) => "allows reads and writes",
    }
}

fn cost(cost: Cost) -> &'static str {
    match cost {
        Cost::Metadata => "metadata only (fast)",
        Cost::Scan => "full table scan",
        Cost::Rewrite => "rewrites the table and its indexes",
        Cost::Unknown => "depends on the data",
    }
}

fn summary(report: &Report) -> String {
    let c = report.counts();
    let suppressed = if c.suppressed > 0 {
        format!(", {} suppressed", c.suppressed)
    } else {
        String::new()
    };
    let not_analyzed = if c.not_analyzed > 0 {
        format!(", {} not analyzed", c.not_analyzed)
    } else {
        String::new()
    };
    format!(
        "{}, {}{suppressed} in {} ({}{not_analyzed})",
        plural(c.errors, "error"),
        plural(c.warnings, "warning"),
        plural(c.files, "file"),
        plural(c.statements, "statement")
    )
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}
