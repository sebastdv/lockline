use lockline::model::TableRef;
use lockline::parse::parse_sql;
use lockline::schema::{Schema, TypeSpec, is_safe_widening};
use pg_query::NodeEnum;

fn schema_from(sql: &str) -> Schema {
    let mut schema = Schema::default();
    for stmt in parse_sql(sql).unwrap() {
        schema.observe(&stmt.node);
    }
    schema
}

fn ty(sql_type: &str) -> TypeSpec {
    let stmt = parse_sql(&format!("CREATE TABLE t (c {sql_type})"))
        .unwrap()
        .remove(0);
    let NodeEnum::CreateStmt(create) = stmt.node else {
        panic!("not a CREATE TABLE");
    };
    let Some(NodeEnum::ColumnDef(column)) = create.table_elts[0].node.as_ref() else {
        panic!("no column");
    };
    TypeSpec::from_type_name(column.type_name.as_ref().unwrap())
}

fn table(schema: Option<&str>, name: &str) -> TableRef {
    TableRef {
        schema: schema.map(str::to_string),
        name: name.to_string(),
    }
}

fn spec(name: &str, mods: &[i64]) -> TypeSpec {
    TypeSpec {
        name: name.to_string(),
        mods: mods.to_vec(),
    }
}

#[test]
fn safe_widening_table() {
    let cases = [
        ("varchar(10)", "varchar(20)", true),
        ("varchar(20)", "varchar(10)", false),
        ("varchar(10)", "varchar", true),
        ("varchar(10)", "text", true),
        ("numeric(10,2)", "numeric(12,2)", true),
        ("numeric(10,2)", "numeric(12,3)", false),
        ("varbit(4)", "varbit(8)", true),
        ("int4", "int8", false),
        ("text", "text", true),
        ("text", "varchar(10)", false),
    ];
    for (old, new, expected) in cases {
        assert_eq!(
            is_safe_widening(&ty(old), &ty(new)),
            expected,
            "{old} -> {new}"
        );
    }
}

#[test]
fn type_spec_from_sql() {
    assert_eq!(ty("varchar(100)"), spec("varchar", &[100]));
    assert_eq!(ty("numeric(10,2)"), spec("numeric", &[10, 2]));
    assert_eq!(ty("text"), spec("text", &[]));
    assert_eq!(ty("int"), spec("int4", &[]));
}

#[test]
fn tracks_columns_from_create_and_alter() {
    let schema = schema_from(
        "CREATE TABLE users (email varchar(100)); ALTER TABLE users ADD COLUMN name text;",
    );
    let users = table(None, "users");
    assert_eq!(
        schema.column_type(&users, "email"),
        Some(&spec("varchar", &[100]))
    );
    assert_eq!(schema.column_type(&users, "name"), Some(&spec("text", &[])));
    assert_eq!(schema.column_type(&users, "missing"), None);
}

#[test]
fn alter_type_and_drop_column() {
    let users = table(None, "users");
    let altered = schema_from(
        "CREATE TABLE users (email varchar(100)); ALTER TABLE users ALTER COLUMN email TYPE varchar(200);",
    );
    assert_eq!(
        altered.column_type(&users, "email"),
        Some(&spec("varchar", &[200]))
    );
    let dropped = schema_from(
        "CREATE TABLE users (email varchar(100)); ALTER TABLE users DROP COLUMN email;",
    );
    assert_eq!(dropped.column_type(&users, "email"), None);
}

#[test]
fn rename_chain_keeps_types() {
    let schema = schema_from(
        "CREATE TABLE public.users (email varchar(50));
         ALTER TABLE users RENAME TO accounts;
         ALTER TABLE accounts RENAME COLUMN email TO mail;",
    );
    assert_eq!(
        schema.column_type(&table(None, "accounts"), "mail"),
        Some(&spec("varchar", &[50]))
    );
    assert_eq!(schema.column_type(&table(None, "users"), "email"), None);
    assert_eq!(schema.column_type(&table(None, "accounts"), "email"), None);
}

#[test]
fn validated_not_null_check() {
    let users = table(None, "users");
    let base = "CREATE TABLE users (email text);";
    let not_valid =
        format!("{base} ALTER TABLE users ADD CONSTRAINT c CHECK (email IS NOT NULL) NOT VALID;");
    assert!(!schema_from(&not_valid).has_validated_not_null_check(&users, "email"));
    let validated = format!("{not_valid} ALTER TABLE users VALIDATE CONSTRAINT c;");
    assert!(schema_from(&validated).has_validated_not_null_check(&users, "email"));
    let immediate = format!("{base} ALTER TABLE users ADD CHECK (email IS NOT NULL);");
    assert!(schema_from(&immediate).has_validated_not_null_check(&users, "email"));
    let other = format!("{base} ALTER TABLE users ADD CHECK (email <> '');");
    assert!(!schema_from(&other).has_validated_not_null_check(&users, "email"));
    let dropped = format!("{validated} ALTER TABLE users DROP CONSTRAINT c;");
    assert!(!schema_from(&dropped).has_validated_not_null_check(&users, "email"));
}

#[test]
fn index_table_lookup() {
    let created = schema_from("CREATE INDEX i ON app.users (x);");
    assert_eq!(
        created.index_table(&table(Some("app"), "i")),
        Some(&table(Some("app"), "users"))
    );
    let dropped = schema_from("CREATE INDEX i ON app.users (x); DROP INDEX app.i;");
    assert_eq!(dropped.index_table(&table(Some("app"), "i")), None);
}

#[test]
fn drop_table_forgets_columns() {
    let schema = schema_from("CREATE TABLE users (email text); DROP TABLE users;");
    assert_eq!(schema.column_type(&table(None, "users"), "email"), None);
}
