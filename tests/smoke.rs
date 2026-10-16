#[test]
fn pg_query_parses_postgres_sql() {
    let result = pg_query::parse("CREATE INDEX CONCURRENTLY i ON t (c);").unwrap();
    assert_eq!(result.protobuf.stmts.len(), 1);
}
