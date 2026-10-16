//! lockline: explains the locks PostgreSQL migrations take and flags the dangerous ones.

pub mod analyze;
pub mod catalog;
pub mod context;
pub mod input;
pub mod locks;
pub mod model;
pub mod parse;
pub mod report;
pub mod rules;
pub mod schema;
pub mod volatility;

pub use analyze::{
    Config, Counts, HeldLock, MigrationUnit, Report, StatementReport, TransactionMode,
    TransactionSummary, UnitReport, analyze,
};
