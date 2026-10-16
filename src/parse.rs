//! Parses SQL with the real PostgreSQL parser and keeps the position of every statement.

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
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct ParseError {
    pub message: String,
}

pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, ParseError> {
    let result = pg_query::parse(sql).map_err(|e| ParseError {
        message: e.to_string(),
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
        let text = sql[start..end]
            .trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string();
        let (line, col) = line_col(sql, start);
        statements.push(Statement {
            node,
            text,
            line,
            col,
        });
    }
    Ok(statements)
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
}
