//! `-- lockline:ignore <rule>[,<rule>] <reason>` comments.
//!
//! Only full-line comments count. A line ignore applies to the next statement when it sits
//! between the end of the previous statement and the start of that one; a file ignore
//! (`lockline:ignore-file`) applies to every statement of the file.

use pg_query::protobuf::Token;

#[derive(Debug, Clone, PartialEq)]
pub struct Ignore {
    /// 1-based line of the comment.
    pub line: usize,
    pub rules: Vec<String>,
    pub reason: Option<String>,
    pub file_wide: bool,
}

pub fn parse_ignores(sql: &str) -> Vec<Ignore> {
    // The scanner knows strings, dollar-quoted bodies and block comments, so a
    // `-- lockline:ignore` inside them is not a comment of the migration.
    let Ok(scan) = pg_query::scan(sql) else {
        return parse_ignores_by_line(sql);
    };
    scan.tokens
        .iter()
        .filter(|t| t.token == Token::SqlComment as i32)
        .filter_map(|t| {
            let (start, end) = (t.start as usize, t.end as usize);
            let line_start = sql[..start].rfind('\n').map_or(0, |i| i + 1);
            if !sql[line_start..start].trim().is_empty() {
                return None; // trails SQL on the same line
            }
            let line = sql[..start].matches('\n').count() + 1;
            parse_comment(&sql[start..end], line)
        })
        .collect()
}

/// Fallback when the scanner fails: every full-line `--` comment.
fn parse_ignores_by_line(sql: &str) -> Vec<Ignore> {
    sql.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            trimmed
                .starts_with("--")
                .then(|| parse_comment(trimmed, index + 1))
                .flatten()
        })
        .collect()
}

/// Parses one `-- ...` comment; `None` when it is not a lockline:ignore.
fn parse_comment(comment: &str, line: usize) -> Option<Ignore> {
    let comment = comment.strip_prefix("--")?.trim_start();
    let (rest, file_wide) = if let Some(rest) = comment.strip_prefix("lockline:ignore-file") {
        (rest, true)
    } else {
        (comment.strip_prefix("lockline:ignore")?, false)
    };
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let rest = rest.trim();
    let (ids, reason) = match rest.split_once(char::is_whitespace) {
        Some((ids, reason)) => (ids, reason.trim()),
        None => (rest, ""),
    };
    Some(Ignore {
        line,
        rules: ids
            .split(',')
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect(),
        reason: (!reason.is_empty()).then(|| reason.to_string()),
        file_wide,
    })
}
