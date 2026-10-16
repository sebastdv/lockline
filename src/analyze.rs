//! Walks migration units statement by statement and produces the report.

use std::collections::HashSet;

use pg_query::NodeEnum;

use crate::catalog::analyze_statement;
use crate::context::{UnitContext, ends_transaction};
use crate::locks::LockMode;
use crate::model::{Analysis, Effect, Finding, Severity, TableRef};
use crate::parse::parse_sql;
use crate::rules::{self, RuleCtx};
use crate::schema::Schema;
use crate::suppress::{Ignore, parse_ignores};

/// One migration as it will run: a file, stdin, or one migration from a tool's output.
#[derive(Debug, Clone)]
pub struct MigrationUnit {
    /// Shown in reports: the path as given, or `<stdin>`.
    pub name: String,
    pub sql: String,
}

/// How the migration tool runs each unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TransactionMode {
    /// The whole unit runs in one transaction (Laravel on Postgres, Flyway, Rails, Django…).
    #[default]
    PerUnit,
    /// Autocommit, like `psql -f`.
    None,
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub transaction: TransactionMode,
}

#[derive(Debug, Clone)]
pub struct StatementReport {
    pub line: usize,
    pub col: usize,
    pub text: String,
    pub analysis: Analysis,
    /// Every table this statement locks was created earlier in the same unit.
    pub new_table: bool,
    pub findings: Vec<Finding>,
    /// Findings silenced by a `lockline:ignore` comment.
    pub suppressed: Vec<Suppressed>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suppressed {
    pub rule: &'static str,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnitReport {
    pub name: String,
    pub statements: Vec<StatementReport>,
    /// Findings about the unit as a whole, such as `parse-error`.
    pub findings: Vec<Finding>,
    /// Transactions with two or more statements that hold locks until they end.
    pub transactions: Vec<TransactionSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeldLock {
    pub table: TableRef,
    pub lock: LockMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransactionSummary {
    /// Index in `statements` of the COMMIT/ROLLBACK/PREPARE; `None` when the unit's implicit
    /// transaction ends with the file.
    pub ends_after: Option<usize>,
    /// Statements inside the transaction, not counting BEGIN/COMMIT.
    pub statement_count: usize,
    /// Strongest lock per existing table, in the order they were first taken.
    pub locks: Vec<HeldLock>,
}

impl TransactionSummary {
    fn open() -> Self {
        TransactionSummary {
            ends_after: None,
            statement_count: 0,
            locks: Vec::new(),
        }
    }

    fn record(&mut self, effects: &[Effect]) {
        self.statement_count += 1;
        for effect in effects.iter().filter(|e| !e.new_table) {
            match self.locks.iter_mut().find(|h| h.table == effect.table) {
                Some(held) => held.lock = held.lock.max(effect.lock),
                None => self.locks.push(HeldLock {
                    table: effect.table.clone(),
                    lock: effect.lock,
                }),
            }
        }
    }

    fn worth_reporting(&self) -> bool {
        self.statement_count >= 2 && !self.locks.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub units: Vec<UnitReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
    pub suppressed: usize,
    pub files: usize,
    pub statements: usize,
    /// Statements outside the lock catalog.
    pub not_analyzed: usize,
}

impl UnitReport {
    pub fn all_findings(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .chain(self.statements.iter().flat_map(|s| s.findings.iter()))
    }
}

impl Report {
    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            files: self.units.len(),
            ..Counts::default()
        };
        for unit in &self.units {
            counts.statements += unit.statements.len();
            counts.suppressed += unit
                .statements
                .iter()
                .map(|s| s.suppressed.len())
                .sum::<usize>();
            counts.not_analyzed += unit
                .statements
                .iter()
                .filter(|s| s.analysis == Analysis::NotAnalyzed)
                .count();
            for finding in unit.all_findings() {
                match finding.severity {
                    Severity::Error => counts.errors += 1,
                    Severity::Warning => counts.warnings += 1,
                }
            }
        }
        counts
    }

    pub fn max_severity(&self) -> Option<Severity> {
        self.units
            .iter()
            .flat_map(UnitReport::all_findings)
            .map(|f| f.severity)
            .max()
    }
}

pub fn analyze(units: &[MigrationUnit], config: &Config) -> Report {
    let rules = rules::all();
    // The virtual schema accumulates across units, in the order they run.
    let mut schema = Schema::default();
    let units = units
        .iter()
        .map(|unit| analyze_unit(unit, config, &rules, &mut schema))
        .collect();
    Report { units }
}

fn analyze_unit(
    unit: &MigrationUnit,
    config: &Config,
    rules: &[Box<dyn rules::Rule>],
    schema: &mut Schema,
) -> UnitReport {
    let mut report = UnitReport {
        name: unit.name.clone(),
        statements: Vec::new(),
        findings: Vec::new(),
        transactions: Vec::new(),
    };
    let statements = match parse_sql(&unit.sql) {
        Ok(statements) => statements,
        Err(error) => {
            report.findings.push(Finding {
                rule: "parse-error",
                severity: Severity::Error,
                message: match error.line {
                    Some(line) => {
                        format!("Could not parse this file (line {line}): {}", error.message)
                    }
                    None => format!("Could not parse this file: {}", error.message),
                },
                help: None,
                line: error.line,
            });
            return report;
        }
    };

    let ignores = parse_ignores(&unit.sql);
    let mut attached = vec![false; ignores.len()];
    let mut prev_end_line = 0;
    let mut ctx = UnitContext::new(config.transaction);
    let mut transaction = TransactionSummary::open();
    let mut fired_once: HashSet<&'static str> = HashSet::new();
    for (index, stmt) in statements.iter().enumerate() {
        let mut analysis = analyze_statement(&stmt.node, schema);
        if let Analysis::Locks(effects) = &mut analysis {
            for effect in effects.iter_mut() {
                effect.new_table = ctx.created_tables.contains(&effect.table);
            }
        }
        let new_table = match &analysis {
            Analysis::Locks(effects) => !effects.is_empty() && effects.iter().all(|e| e.new_table),
            _ => false,
        };
        let rule_ctx = RuleCtx {
            stmt,
            analysis: &analysis,
            new_table,
            in_transaction: ctx.in_transaction,
            schema,
            lock_timeout_set: ctx.lock_timeout_set(),
        };
        let mut findings: Vec<Finding> = rules
            .iter()
            .filter(|r| !r.once_per_unit() || !fired_once.contains(r.id()))
            .filter_map(|r| r.check(&rule_ctx))
            .collect();
        for rule in rules.iter().filter(|r| r.once_per_unit()) {
            if findings.iter().any(|f| f.rule == rule.id()) {
                fired_once.insert(rule.id());
            }
        }
        let applicable: Vec<usize> = (0..ignores.len())
            .filter(|&i| {
                let ignore = &ignores[i];
                ignore.file_wide || (prev_end_line < ignore.line && ignore.line < stmt.line)
            })
            .collect();
        let mut suppressed = Vec::new();
        findings.retain(|f| {
            let by = applicable
                .iter()
                .map(|&i| &ignores[i])
                .find(|ignore| ignore.rules.iter().any(|r| r == f.rule));
            match by {
                Some(ignore) => {
                    suppressed.push(Suppressed {
                        rule: f.rule,
                        reason: ignore.reason.clone(),
                    });
                    false
                }
                None => true,
            }
        });
        for &i in applicable.iter().filter(|&&i| !ignores[i].file_wide) {
            attached[i] = true;
            if ignores[i].reason.is_none() {
                findings.push(ignore_without_reason(&ignores[i], None));
            }
        }
        prev_end_line = stmt.end_line;
        let is_control = matches!(stmt.node, NodeEnum::TransactionStmt(_));
        if ctx.in_transaction && !is_control {
            let effects = match &analysis {
                Analysis::Locks(effects) => effects.as_slice(),
                _ => &[],
            };
            transaction.record(effects);
        }
        report.statements.push(StatementReport {
            line: stmt.line,
            col: stmt.col,
            text: stmt.text.clone(),
            analysis,
            new_table,
            findings,
            suppressed,
        });
        ctx.observe(&stmt.node);
        schema.observe(&stmt.node);
        if ends_transaction(&stmt.node) {
            let mut ended = std::mem::replace(&mut transaction, TransactionSummary::open());
            ended.ends_after = Some(index);
            if ended.worth_reporting() {
                report.transactions.push(ended);
            }
        }
    }
    if ctx.in_transaction && transaction.worth_reporting() {
        report.transactions.push(transaction);
    }
    for (ignore, attached) in ignores.iter().zip(attached) {
        if ignore.reason.is_none() && (ignore.file_wide || !attached) {
            report
                .findings
                .push(ignore_without_reason(ignore, Some(ignore.line)));
        }
    }
    report
}

fn ignore_without_reason(ignore: &Ignore, line: Option<usize>) -> Finding {
    Finding {
        rule: "ignore-without-reason",
        severity: Severity::Warning,
        message: format!(
            "lockline:ignore for {} has no reason; say why this is safe.",
            ignore.rules.join(",")
        ),
        help: Some(
            "Write the reason after the rule ids: -- lockline:ignore <rule> <why this is safe>"
                .to_string(),
        ),
        line,
    }
}
