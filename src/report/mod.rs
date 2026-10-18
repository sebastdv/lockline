//! Output formats.

mod github;
mod human;
mod json;
mod sarif;

pub use github::render_github;
pub use human::{HumanOptions, render_human};
pub use json::render_json;
pub use sarif::render_sarif;
