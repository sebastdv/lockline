//! Decides whether a column DEFAULT expression is volatile.
//!
//! Since PostgreSQL 11, `ADD COLUMN ... DEFAULT <expr>` only touches the catalog when `<expr>`
//! is not volatile; a volatile default has to be evaluated per row, which rewrites the table.
//!
//! Known volatile functions (=> rewrite): random, gen_random_uuid, uuid_generate_v1,
//! uuid_generate_v1mc, uuid_generate_v4, clock_timestamp, timeofday, nextval, txid_current,
//! pg_current_xact_id.
//!
//! Known safe expressions: constants, SQL value functions (CURRENT_TIMESTAMP, CURRENT_DATE…),
//! now(), transaction_timestamp(), statement_timestamp(), casts, operators, boolean
//! expressions and arrays whose parts are all safe.
//!
//! Anything else is `Unknown`: lockline does not pretend to know the volatility of an
//! arbitrary function.

use pg_query::NodeEnum;
use pg_query::protobuf::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Volatility {
    Safe,
    Volatile,
    Unknown,
}

const VOLATILE_FUNCTIONS: &[&str] = &[
    "random",
    "gen_random_uuid",
    "uuid_generate_v1",
    "uuid_generate_v1mc",
    "uuid_generate_v4",
    "clock_timestamp",
    "timeofday",
    "nextval",
    "txid_current",
    "pg_current_xact_id",
];

const STABLE_FUNCTIONS: &[&str] = &["now", "transaction_timestamp", "statement_timestamp"];

pub fn classify_default(expr: &Node) -> Volatility {
    match expr.node.as_ref() {
        Some(NodeEnum::AConst(_)) | Some(NodeEnum::SqlvalueFunction(_)) => Volatility::Safe,
        Some(NodeEnum::TypeCast(cast)) => cast
            .arg
            .as_deref()
            .map_or(Volatility::Unknown, classify_default),
        Some(NodeEnum::FuncCall(call)) => {
            let name = last_name(&call.funcname);
            let args = combine(call.args.iter());
            if VOLATILE_FUNCTIONS.contains(&name) || args == Volatility::Volatile {
                Volatility::Volatile
            } else if STABLE_FUNCTIONS.contains(&name) {
                args
            } else {
                Volatility::Unknown
            }
        }
        Some(NodeEnum::AExpr(e)) => combine(e.lexpr.iter().chain(e.rexpr.iter()).map(|b| &**b)),
        Some(NodeEnum::BoolExpr(e)) => combine(e.args.iter()),
        Some(NodeEnum::AArrayExpr(e)) => combine(e.elements.iter()),
        _ => Volatility::Unknown,
    }
}

/// Volatile if any part is volatile, otherwise unknown if any part is unknown.
fn combine<'a>(parts: impl Iterator<Item = &'a Node>) -> Volatility {
    let mut result = Volatility::Safe;
    for part in parts {
        match classify_default(part) {
            Volatility::Volatile => return Volatility::Volatile,
            Volatility::Unknown => result = Volatility::Unknown,
            Volatility::Safe => {}
        }
    }
    result
}

/// Last component of a qualified name (`pg_catalog.now` -> `now`).
pub(crate) fn last_name(names: &[Node]) -> &str {
    names
        .iter()
        .rev()
        .find_map(|n| match n.node.as_ref() {
            Some(NodeEnum::String(s)) => Some(s.sval.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

/// How to name the first function call of unknown volatility in an expression, as
/// `name()` or `name(...)`.
pub fn unknown_call(expr: &Node) -> Option<String> {
    match expr.node.as_ref()? {
        NodeEnum::FuncCall(call) => {
            let name = last_name(&call.funcname);
            if !VOLATILE_FUNCTIONS.contains(&name) && !STABLE_FUNCTIONS.contains(&name) {
                let args = if call.args.is_empty() { "" } else { "..." };
                return Some(format!("{name}({args})"));
            }
            call.args.iter().find_map(unknown_call)
        }
        NodeEnum::TypeCast(cast) => cast.arg.as_deref().and_then(unknown_call),
        NodeEnum::AExpr(e) => e
            .lexpr
            .as_deref()
            .and_then(unknown_call)
            .or_else(|| e.rexpr.as_deref().and_then(unknown_call)),
        NodeEnum::BoolExpr(e) => e.args.iter().find_map(unknown_call),
        NodeEnum::AArrayExpr(e) => e.elements.iter().find_map(unknown_call),
        _ => None,
    }
}
