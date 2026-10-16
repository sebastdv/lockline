//! Stable JSON output (`--format json`). The structs here are the public schema; they are
//! kept separate from the internal model so the model can change without breaking it.

use serde::Serialize;

use crate::analyze::{Report, StatementReport, UnitReport};
use crate::model::{Analysis, Cost, Effect, Finding};

const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct JsonReport<'a> {
    version: u32,
    files: Vec<JsonFile<'a>>,
    summary: JsonSummary,
}

#[derive(Serialize)]
struct JsonFile<'a> {
    path: &'a str,
    findings: Vec<JsonFinding<'a>>,
    statements: Vec<JsonStatement<'a>>,
    transactions: Vec<JsonTransaction>,
}

#[derive(Serialize)]
struct JsonStatement<'a> {
    line: usize,
    col: usize,
    end_line: usize,
    text: &'a str,
    analyzed: bool,
    effects: Vec<JsonEffect<'a>>,
    findings: Vec<JsonFinding<'a>>,
    suppressed: Vec<JsonSuppressed<'a>>,
}

#[derive(Serialize)]
struct JsonEffect<'a> {
    table: String,
    lock: &'static str,
    blocks_reads: bool,
    blocks_writes: bool,
    cost: &'static str,
    new_table: bool,
    note: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonFinding<'a> {
    rule: &'static str,
    severity: &'static str,
    message: &'a str,
    help: Option<&'a str>,
    line: Option<usize>,
}

#[derive(Serialize)]
struct JsonSuppressed<'a> {
    rule: &'static str,
    reason: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonTransaction {
    /// Line of the COMMIT/ROLLBACK/PREPARE; null when the file's implicit transaction ends.
    ends_after_line: Option<usize>,
    statement_count: usize,
    locks: Vec<JsonHeldLock>,
}

#[derive(Serialize)]
struct JsonHeldLock {
    table: String,
    lock: &'static str,
}

#[derive(Serialize)]
struct JsonSummary {
    errors: usize,
    warnings: usize,
    suppressed: usize,
    files: usize,
    statements: usize,
    not_analyzed: usize,
}

pub fn render_json(report: &Report) -> String {
    let counts = report.counts();
    let json = JsonReport {
        version: SCHEMA_VERSION,
        files: report.units.iter().map(file).collect(),
        summary: JsonSummary {
            errors: counts.errors,
            warnings: counts.warnings,
            suppressed: counts.suppressed,
            files: counts.files,
            statements: counts.statements,
            not_analyzed: counts.not_analyzed,
        },
    };
    let mut out = serde_json::to_string_pretty(&json).expect("the JSON report always serializes");
    out.push('\n');
    out
}

fn file(unit: &UnitReport) -> JsonFile<'_> {
    JsonFile {
        path: &unit.name,
        findings: unit.findings.iter().map(finding).collect(),
        statements: unit.statements.iter().map(statement).collect(),
        transactions: unit
            .transactions
            .iter()
            .map(|t| JsonTransaction {
                ends_after_line: t.ends_after.map(|i| unit.statements[i].line),
                statement_count: t.statement_count,
                locks: t
                    .locks
                    .iter()
                    .map(|h| JsonHeldLock {
                        table: h.table.to_string(),
                        lock: h.lock.as_str(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn statement(stmt: &StatementReport) -> JsonStatement<'_> {
    let effects = match &stmt.analysis {
        Analysis::Locks(effects) => effects.iter().map(effect).collect(),
        _ => Vec::new(),
    };
    JsonStatement {
        line: stmt.line,
        col: stmt.col,
        end_line: stmt.end_line,
        text: &stmt.text,
        analyzed: stmt.analysis != Analysis::NotAnalyzed,
        effects,
        findings: stmt.findings.iter().map(finding).collect(),
        suppressed: stmt
            .suppressed
            .iter()
            .map(|s| JsonSuppressed {
                rule: s.rule,
                reason: s.reason.as_deref(),
            })
            .collect(),
    }
}

fn effect(e: &Effect) -> JsonEffect<'_> {
    JsonEffect {
        table: e.table.to_string(),
        lock: e.lock.as_str(),
        blocks_reads: e.lock.blocks_reads(),
        blocks_writes: e.lock.blocks_writes(),
        cost: match e.cost {
            Cost::Metadata => "metadata",
            Cost::Scan => "scan",
            Cost::Rewrite => "rewrite",
            Cost::Unknown => "unknown",
        },
        new_table: e.new_table,
        note: e.note.as_deref(),
    }
}

fn finding(f: &Finding) -> JsonFinding<'_> {
    JsonFinding {
        rule: f.rule,
        severity: f.severity.as_str(),
        message: &f.message,
        help: f.help.as_deref(),
        line: f.line,
    }
}
