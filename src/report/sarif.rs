//! SARIF 2.1.0 (`--format sarif`), for GitHub code scanning and other tools that read it.

use std::path::Path;

use serde_json::{Value, json};

use crate::analyze::{Report, Source, UnitReport};
use crate::model::Severity;
use crate::rules::{RuleInfo, infos};

const REPO: &str = "https://github.com/sebastdv/lockline";

pub fn render_sarif(report: &Report) -> String {
    let infos = infos();
    let rules: Vec<Value> = infos.iter().map(rule).collect();
    let index_of = |id: &str| infos.iter().position(|i| i.id == id);
    let mut results = Vec::new();

    for unit in &report.units {
        for finding in &unit.findings {
            let line = match unit.source {
                Source::Sql => finding.line.unwrap_or(1),
                Source::Laravel => 1,
            };
            results.push(result(
                finding.rule,
                index_of(finding.rule),
                finding.severity,
                &with_help(&finding.message, finding.help.as_deref()),
                location(unit, line, None),
                None,
            ));
        }
        for stmt in &unit.statements {
            let (line, col) = match unit.source {
                Source::Sql => (stmt.line, Some(stmt.col)),
                Source::Laravel => (1, None),
            };
            for finding in &stmt.findings {
                let mut message = with_help(&finding.message, finding.help.as_deref());
                if unit.source == Source::Laravel {
                    message = format!(
                        "{message}\nStatement: {}",
                        stmt.text.split_whitespace().collect::<Vec<_>>().join(" ")
                    );
                }
                results.push(result(
                    finding.rule,
                    index_of(finding.rule),
                    finding.severity,
                    &message,
                    location(unit, line, col),
                    None,
                ));
            }
            // Suppressed findings are reported as suppressed, so code scanning shows them
            // as dismissed in source with the reason given in the comment.
            for suppressed in &stmt.suppressed {
                let info = infos.iter().find(|i| i.id == suppressed.rule);
                results.push(result(
                    suppressed.rule,
                    index_of(suppressed.rule),
                    info.map(|i| i.severity).unwrap_or(Severity::Warning),
                    info.map(|i| i.summary).unwrap_or(suppressed.rule),
                    location(unit, line, col),
                    Some(suppressed.reason.as_deref().unwrap_or("")),
                ));
            }
        }
    }

    let log = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "lockline",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": REPO,
                    "rules": rules,
                }
            },
            "columnKind": "unicodeCodePoints",
            "results": results,
        }]
    });
    let mut out = serde_json::to_string_pretty(&log).expect("SARIF is always serializable");
    out.push('\n');
    out
}

fn rule(info: &RuleInfo) -> Value {
    json!({
        "id": info.id,
        "name": pascal_case(info.id),
        "shortDescription": { "text": info.summary },
        "helpUri": format!("{REPO}/blob/main/docs/rules/{}.md", info.id),
        "help": { "text": info.docs, "markdown": info.docs },
        "defaultConfiguration": { "level": level(info.severity) },
        "properties": { "tags": ["postgresql", "migrations", "locks"] },
    })
}

fn result(
    rule: &str,
    index: Option<usize>,
    severity: Severity,
    message: &str,
    location: Option<Value>,
    suppressed: Option<&str>,
) -> Value {
    let mut result = json!({
        "ruleId": rule,
        "level": level(severity),
        "message": { "text": message },
    });
    if let Some(index) = index {
        result["ruleIndex"] = json!(index);
    }
    if let Some(location) = location {
        result["locations"] = json!([location]);
    }
    if let Some(justification) = suppressed {
        result["suppressions"] = json!([{ "kind": "inSource", "justification": justification }]);
    }
    result
}

/// `None` for stdin: SARIF results may omit the location.
fn location(unit: &UnitReport, line: usize, col: Option<usize>) -> Option<Value> {
    if unit.name == "<stdin>" {
        return None;
    }
    let mut region = json!({ "startLine": line });
    if let Some(col) = col {
        region["startColumn"] = json!(col);
    }
    Some(json!({
        "physicalLocation": {
            "artifactLocation": { "uri": uri(&unit.name) },
            "region": region,
        }
    }))
}

/// Relative paths stay relative (code scanning resolves them against the repository root),
/// with forward slashes; absolute paths become `file://` URIs.
fn uri(name: &str) -> String {
    let path = name.trim_start_matches("./").replace('\\', "/");
    if Path::new(name).is_absolute() {
        format!("file://{path}")
    } else {
        path
    }
}

fn with_help(message: &str, help: Option<&str>) -> String {
    match help {
        Some(help) => format!("{message}\nhelp: {help}"),
        None => message.to_string(),
    }
}

fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// `create-index-not-concurrently` → `CreateIndexNotConcurrently`.
fn pascal_case(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}
