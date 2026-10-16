//! Output formats.

mod human;
mod json;

pub use human::{HumanOptions, render_human};
pub use json::render_json;
