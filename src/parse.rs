//! Parses SQL with the real PostgreSQL parser and keeps the position of every statement.

use pg_query::protobuf::Token;
use thiserror::Error;

/// One parsed statement and where it starts in the source.
#[derive(Debug, Clone)]
pub struct Statement {
    pub node: pg_query::NodeEnum,
    /// Source text without leading whitespace/comments or the trailing `;`.
    pub text: String,
    /// 1-based line of the first significant character.
    pub line: usize,
    /// 1-based column, counted in characters (not bytes).
    pub col: usize,
    /// 1-based line of the last character of `text`.
    pub end_line: usize,
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct ParseError {
    pub message: String,
    /// Line of the statement that failed to parse, when it can be located.
    pub line: Option<usize>,
}

pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, ParseError> {
    let result = pg_query::parse(sql).map_err(|e| ParseError {
        message: e.to_string(),
        line: failing_statement_line(sql),
    })?;

    let mut statements = Vec::new();
    for raw in &result.protobuf.stmts {
        let Some(node) = raw.stmt.as_ref().and_then(|n| n.node.clone()) else {
            continue;
        };
        // stmt_location/stmt_len are byte offsets; a length of 0 means "until the end".
        let location = raw.stmt_location as usize;
        let end = if raw.stmt_len == 0 {
            sql.len()
        } else {
            location + raw.stmt_len as usize
        };
        let start = skip_trivia(sql, location, end);
        let end = significant_end(sql, start, end);
        let text = sql[start..end].to_string();
        let (line, col) = line_col(sql, start);
        let (end_line, _) = line_col(sql, end);
        statements.push(Statement {
            node,
            text,
            line,
            col,
            end_line,
        });
    }
    Ok(statements)
}

/// End of the last token that is neither a comment nor the closing `;`.
fn significant_end(sql: &str, start: usize, end: usize) -> usize {
    let fallback = || {
        start
            + sql[start..end]
                .trim_end()
                .trim_end_matches(';')
                .trim_end()
                .len()
    };
    let Ok(scan) = pg_query::scan(&sql[start..end]) else {
        return fallback();
    };
    let ignored = [
        Token::SqlComment as i32,
        Token::CComment as i32,
        Token::Ascii59 as i32,
    ];
    scan.tokens
        .iter()
        .rev()
        .find(|t| !ignored.contains(&t.token))
        .map_or_else(fallback, |t| start + t.end as usize)
}

/// Splits the file at top-level `;` tokens (the scanner understands strings and dollar quotes)
/// and parses each piece to find the first one that fails.
fn failing_statement_line(sql: &str) -> Option<usize> {
    let scan = pg_query::scan(sql).ok()?;
    let mut bounds: Vec<usize> = scan
        .tokens
        .iter()
        .filter(|t| t.token == Token::Ascii59 as i32)
        .map(|t| t.start as usize)
        .collect();
    bounds.push(sql.len());
    let mut start = 0;
    for end in bounds {
        if pg_query::parse(&sql[start..end]).is_err() {
            let first = skip_trivia(sql, start, end);
            return Some(line_col(sql, first).0);
        }
        start = (end + 1).min(sql.len());
    }
    None
}

/// Advances past whitespace, `-- line` comments and (nested) `/* block */` comments.
fn skip_trivia(sql: &str, mut pos: usize, end: usize) -> usize {
    loop {
        let rest = &sql[pos..end];
        let trimmed = rest.trim_start();
        pos += rest.len() - trimmed.len();
        if trimmed.starts_with("--") {
            pos += trimmed.find('\n').map_or(trimmed.len(), |i| i + 1);
        } else if trimmed.starts_with("/*") {
            pos += block_comment_len(trimmed);
        } else {
            return pos;
        }
    }
}

fn block_comment_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match &bytes[i..i + 2] {
            b"/*" => {
                depth += 1;
                i += 2;
            }
            b"*/" => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    s.len()
}

fn line_col(sql: &str, pos: usize) -> (usize, usize) {
    let prefix = &sql[..pos];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let col = prefix[line_start..].chars().count() + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_statements_with_positions() {
        let sql = "-- add index\nCREATE INDEX i ON t (c);\n\n  ALTER TABLE t ADD COLUMN x int;\n";
        let s = parse_sql(sql).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].line, s[0].col), (2, 1));
        assert_eq!(s[0].text, "CREATE INDEX i ON t (c)");
        assert_eq!((s[1].line, s[1].col), (4, 3));
        assert_eq!(s[1].text, "ALTER TABLE t ADD COLUMN x int");
    }

    #[test]
    fn crlf_line_numbers() {
        let s = parse_sql("SELECT 1;\r\n\r\nSELECT 2;\r\n").unwrap();
        assert_eq!(s[1].line, 3);
    }

    #[test]
    fn multibyte_text_counts_chars_not_bytes() {
        let s = parse_sql("SELECT 'ñandú'; /* acción */ SELECT 2;").unwrap();
        assert_eq!((s[1].line, s[1].col), (1, 30));
    }

    #[test]
    fn last_statement_without_semicolon() {
        let s = parse_sql("SELECT 1;\nSELECT 2").unwrap();
        assert_eq!(s[1].text, "SELECT 2");
    }

    #[test]
    fn empty_and_comment_only_inputs_have_no_statements() {
        assert!(parse_sql("").unwrap().is_empty());
        assert!(
            parse_sql("-- nothing here\n/* still nothing */\n")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn syntax_error_is_reported() {
        let e = parse_sql("CREATE INDEX ON;").unwrap_err();
        assert!(!e.message.is_empty());
    }

    #[test]
    fn trailing_comment_is_not_part_of_last_statement() {
        let s = parse_sql("SELECT 1;\nSELECT 2 -- trailing").unwrap();
        assert_eq!(s[1].text, "SELECT 2");
        let s = parse_sql("SELECT 1;\nSELECT 2 /* c */").unwrap();
        assert_eq!(s[1].text, "SELECT 2");
    }

    #[test]
    fn comment_inside_statement_is_kept() {
        let s = parse_sql("SELECT 1 -- one\n + 1;").unwrap();
        assert_eq!(s[0].text, "SELECT 1 -- one\n + 1");
    }

    #[test]
    fn end_line_of_multiline_statement() {
        let s = parse_sql("SELECT\n  1;\nSELECT 2;").unwrap();
        assert_eq!(s[0].end_line, 2);
        assert_eq!(s[1].end_line, 3);
    }

    #[test]
    fn parse_error_reports_line() {
        let e = parse_sql("SELECT 1;\n\nCREATE INDEX ON;\nSELECT 2;").unwrap_err();
        assert_eq!(e.line, Some(3));
    }

    #[test]
    fn parse_error_line_after_dollar_quotes() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\nSELEC 2;";
        assert_eq!(parse_sql(sql).unwrap_err().line, Some(2));
    }
}
