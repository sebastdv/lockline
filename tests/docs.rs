use std::collections::BTreeSet;

use lockline::rules::infos;

const ALL_IDS: [&str; 21] = [
    "create-index-not-concurrently",
    "drop-index-not-concurrently",
    "reindex-not-concurrently",
    "cannot-run-in-transaction",
    "add-column-rewrite",
    "add-column-not-null-without-default",
    "add-column-unknown-default",
    "alter-column-type",
    "set-not-null",
    "add-check-without-not-valid",
    "add-foreign-key-without-not-valid",
    "add-unique-without-index",
    "rewrite-table",
    "refresh-matview-not-concurrently",
    "missing-lock-timeout",
    "rename-object",
    "drop-object",
    "backfill-in-migration",
    "parse-error",
    "ignore-without-reason",
    "ignore-unknown-rule",
];

#[test]
fn every_rule_has_docs() {
    for info in infos() {
        assert!(
            info.docs.starts_with(&format!("# {}\n", info.id)),
            "{}",
            info.id
        );
        assert!(
            info.docs
                .contains(&format!("**Severity:** {}", info.severity.as_str())),
            "{}",
            info.id
        );
        assert!(info.docs.contains("## Why it matters"), "{}", info.id);
        assert!(info.docs.contains("## Instead"), "{}", info.id);
        assert!(
            !info.summary.is_empty() && !info.summary.contains('\n'),
            "{}",
            info.id
        );
    }
}

#[test]
fn infos_cover_all_ids() {
    let ids: BTreeSet<&str> = infos().iter().map(|i| i.id).collect();
    assert_eq!(ids, ALL_IDS.into_iter().collect::<BTreeSet<_>>());
    assert_eq!(infos().len(), ALL_IDS.len());
}
