//! Walks migration units statement by statement and produces the report.

use crate::catalog::analyze_statement;
use crate::context::UnitContext;
use crate::model::{Analysis, Finding, Severity};
use crate::parse::parse_sql;
use crate::rules::{self, RuleCtx};
use crate::schema::Schema;

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
}

#[derive(Debug, Clone)]
pub struct UnitReport {
    pub name: String,
    pub statements: Vec<StatementReport>,
    /// Findings about the unit as a whole, such as `parse-error`.
    pub findings: Vec<Finding>,
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

    let mut ctx = UnitContext::new(config.transaction);
    for stmt in &statements {
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
        };
        let findings = rules.iter().filter_map(|r| r.check(&rule_ctx)).collect();
        report.statements.push(StatementReport {
            line: stmt.line,
            col: stmt.col,
            text: stmt.text.clone(),
            analysis,
            new_table,
            findings,
        });
        ctx.observe(&stmt.node);
        schema.observe(&stmt.node);
    }
    report
}
