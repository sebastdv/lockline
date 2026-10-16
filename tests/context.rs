use lockline::TransactionMode;
use lockline::context::UnitContext;
use lockline::parse::parse_sql;

fn after(sql: &str, mode: TransactionMode) -> UnitContext {
    let mut ctx = UnitContext::new(mode);
    for stmt in parse_sql(sql).unwrap() {
        ctx.observe(&stmt.node);
    }
    ctx
}

#[test]
fn lock_timeout_states() {
    let cases = [
        ("", false),
        ("SET lock_timeout = '5s';", true),
        ("SET lock_timeout = 1000;", true),
        ("SET lock_timeout = 0;", false),
        ("SET lock_timeout = '0';", false),
        ("SET lock_timeout = '0ms';", false),
        (
            "SET lock_timeout = '5s'; SET lock_timeout TO DEFAULT;",
            false,
        ),
        ("SET lock_timeout = '5s'; RESET lock_timeout;", false),
        ("SET lock_timeout = '5s'; RESET ALL;", false),
        ("SET LOCAL lock_timeout = '2s';", true),
        ("SET LOCAL lock_timeout = '2s'; COMMIT;", false),
        ("SET lock_timeout = '5s'; COMMIT;", true),
    ];
    for (sql, expected) in cases {
        let ctx = after(sql, TransactionMode::PerUnit);
        assert_eq!(ctx.lock_timeout_set(), expected, "{sql:?}");
    }
}

#[test]
fn set_local_outside_a_transaction_has_no_effect() {
    let ctx = after("SET LOCAL lock_timeout = '2s';", TransactionMode::None);
    assert!(!ctx.lock_timeout_set());
}

#[test]
fn commit_and_chain_keeps_transaction() {
    assert!(after("BEGIN; COMMIT AND CHAIN;", TransactionMode::None).in_transaction);
    assert!(after("BEGIN; ROLLBACK AND CHAIN;", TransactionMode::None).in_transaction);
    assert!(!after("BEGIN; COMMIT;", TransactionMode::None).in_transaction);
}

#[test]
fn prepare_transaction_ends_it() {
    assert!(!after("BEGIN; PREPARE TRANSACTION 'x';", TransactionMode::None).in_transaction);
}
