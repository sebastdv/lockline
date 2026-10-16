use pg_query::NodeEnum;
use pg_query::protobuf::{ConstrType, Constraint, a_const};

use super::constraints::added_constraints;
use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity, quote_ident};

pub struct AddUniqueWithoutIndex;

impl Rule for AddUniqueWithoutIndex {
    fn id(&self) -> &'static str {
        "add-unique-without-index"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let table = &target.table;
        let added = added_constraints(&ctx.stmt.node).into_iter().find(|a| {
            (a.is(ConstrType::ConstrUnique) || a.is(ConstrType::ConstrPrimary))
                && a.constraint.indexname.is_empty()
        })?;
        let primary = added.is(ConstrType::ConstrPrimary);
        let kind = if primary { "PRIMARY KEY" } else { "UNIQUE" };
        let name = added.name(table);
        let index = quote_ident(&format!("{name}_idx"));
        let name = quote_ident(&name);
        let columns = quoted_list(&added.columns());
        let c = added.constraint;
        let mut help = String::new();
        if added.inline_column.is_some() {
            help.push_str("Add the column without the constraint, then:\n");
        }
        help.push_str(&format!(
            "CREATE UNIQUE INDEX CONCURRENTLY {index} ON {table} ({columns}){};\n\
             ALTER TABLE {table} ADD CONSTRAINT {name} {kind} USING INDEX {index}{};",
            index_options(c),
            deferrable(c)
        ));
        if primary {
            help.push_str(
                "\n(For a primary key the columns must already be NOT NULL; otherwise USING INDEX scans the table. See \"lockline explain set-not-null\".)",
            );
        }
        Some(self.finding(
            format!(
                "Adding a {kind} constraint builds an index under an ACCESS EXCLUSIVE lock on \"{}\".",
                table.plain()
            ),
            Some(help),
        ))
    }
}

fn quoted_list(names: &[String]) -> String {
    names
        .iter()
        .map(|n| quote_ident(n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `INCLUDE (...)`, `NULLS NOT DISTINCT` and `WITH (...)`, so the index enforces the same
/// thing the constraint would.
fn index_options(c: &Constraint) -> String {
    let mut out = String::new();
    let including: Vec<String> = c
        .including
        .iter()
        .filter_map(|n| match n.node.as_ref() {
            Some(NodeEnum::String(s)) => Some(s.sval.clone()),
            _ => None,
        })
        .collect();
    if !including.is_empty() {
        out.push_str(&format!(" INCLUDE ({})", quoted_list(&including)));
    }
    if c.nulls_not_distinct {
        out.push_str(" NULLS NOT DISTINCT");
    }
    let options: Vec<String> = c
        .options
        .iter()
        .filter_map(|n| match n.node.as_ref() {
            Some(NodeEnum::DefElem(d)) => {
                Some(match d.arg.as_deref().and_then(|a| a.node.as_ref()) {
                    Some(NodeEnum::Integer(i)) => format!("{} = {}", d.defname, i.ival),
                    Some(NodeEnum::Float(f)) => format!("{} = {}", d.defname, f.fval),
                    Some(NodeEnum::String(s)) => {
                        format!("{} = '{}'", d.defname, s.sval.replace('\'', "''"))
                    }
                    Some(NodeEnum::Boolean(b)) => format!("{} = {}", d.defname, b.boolval),
                    Some(NodeEnum::AConst(a)) => match &a.val {
                        Some(a_const::Val::Ival(i)) => format!("{} = {}", d.defname, i.ival),
                        Some(a_const::Val::Sval(s)) => {
                            format!("{} = '{}'", d.defname, s.sval.replace('\'', "''"))
                        }
                        _ => d.defname.clone(),
                    },
                    _ => d.defname.clone(),
                })
            }
            _ => None,
        })
        .collect();
    if !options.is_empty() {
        out.push_str(&format!(" WITH ({})", options.join(", ")));
    }
    out
}

fn deferrable(c: &Constraint) -> &'static str {
    match (c.deferrable, c.initdeferred) {
        (_, true) => " DEFERRABLE INITIALLY DEFERRED",
        (true, false) => " DEFERRABLE",
        _ => "",
    }
}
