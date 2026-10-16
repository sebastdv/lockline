//! `-- lockline:ignore <rule>[,<rule>] <reason>` comments.
//!
//! Only full-line comments count. A line ignore applies to the next statement when it sits
//! between the end of the previous statement and the start of that one; a file ignore
//! (`lockline:ignore-file`) applies to every statement of the file.

#[derive(Debug, Clone, PartialEq)]
pub struct Ignore {
    /// 1-based line of the comment.
    pub line: usize,
    pub rules: Vec<String>,
    pub reason: Option<String>,
    pub file_wide: bool,
}

pub fn parse_ignores(sql: &str) -> Vec<Ignore> {
    let mut ignores = Vec::new();
    for (index, line) in sql.lines().enumerate() {
        let Some(comment) = line.trim_start().strip_prefix("--") else {
            continue;
        };
        let comment = comment.trim_start();
        let (rest, file_wide) = if let Some(rest) = comment.strip_prefix("lockline:ignore-file") {
            (rest, true)
        } else if let Some(rest) = comment.strip_prefix("lockline:ignore") {
            (rest, false)
        } else {
            continue;
        };
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            continue;
        }
        let rest = rest.trim();
        let (ids, reason) = match rest.split_once(char::is_whitespace) {
            Some((ids, reason)) => (ids, reason.trim()),
            None => (rest, ""),
        };
        ignores.push(Ignore {
            line: index + 1,
            rules: ids
                .split(',')
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect(),
            reason: (!reason.is_empty()).then(|| reason.to_string()),
            file_wide,
        });
    }
    ignores
}
