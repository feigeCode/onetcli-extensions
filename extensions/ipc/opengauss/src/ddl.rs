use extension_protocol::ddl::{
    BuildAlterTableParams, BuildAlterTableResult, BuildCreateTableParams, BuildCreateTableResult,
    BuildDdlParams, BuildDdlResult, BuildDropParams, BuildDropResult, ColumnRenameSpec, ColumnSpec,
    CreateTableOptions, DdlBuildOp, ForeignKeySpec, TableSpec,
};
use extension_protocol::error::ProtocolError;
use extension_protocol::schema::ObjectKind;
use serde_json::Value;

use crate::server::{invalid_params, params_deserialize_error};

pub fn handle_ddl_build(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildDdlParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let result = match p.op {
        DdlBuildOp::CreateTable => {
            let spec: TableSpec =
                serde_json::from_value(p.payload).map_err(params_deserialize_error)?;
            let create = build_create_table(BuildCreateTableParams {
                conn_id: p.conn_id,
                spec,
                options: CreateTableOptions::default(),
            })?;
            BuildDdlResult {
                statements: create.statements,
                warnings: Vec::new(),
            }
        }
        DdlBuildOp::AlterTable => {
            let alter: BuildAlterTableParams =
                serde_json::from_value(p.payload).map_err(params_deserialize_error)?;
            let alter = build_alter_table(alter)?;
            BuildDdlResult {
                statements: alter.statements,
                warnings: alter.warnings,
            }
        }
        DdlBuildOp::DropTable => {
            let mut drop: BuildDropParams =
                serde_json::from_value(p.payload).map_err(params_deserialize_error)?;
            drop.kind = ObjectKind::Table;
            BuildDdlResult {
                statements: vec![build_drop(drop)?.sql],
                warnings: Vec::new(),
            }
        }
        DdlBuildOp::DropView => {
            let mut drop: BuildDropParams =
                serde_json::from_value(p.payload).map_err(params_deserialize_error)?;
            drop.kind = ObjectKind::View;
            BuildDdlResult {
                statements: vec![build_drop(drop)?.sql],
                warnings: Vec::new(),
            }
        }
        other => {
            return Err(invalid_params(format!(
                "unsupported OpenGauss ddl/build op `{other:?}`"
            )));
        }
    };
    serde_json::to_value(result).map_err(params_deserialize_error)
}

pub fn handle_ddl_build_create_table(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildCreateTableParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(build_create_table(p)?).map_err(params_deserialize_error)
}

pub fn handle_ddl_build_alter_table(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildAlterTableParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(build_alter_table(p)?).map_err(params_deserialize_error)
}

pub fn handle_ddl_build_drop(params: &Value) -> Result<Value, ProtocolError> {
    let p: BuildDropParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    serde_json::to_value(build_drop(p)?).map_err(params_deserialize_error)
}

fn build_create_table(p: BuildCreateTableParams) -> Result<BuildCreateTableResult, ProtocolError> {
    if p.spec.name.trim().is_empty() {
        return Err(invalid_params("create table requires table name"));
    }
    if p.spec.columns.is_empty() {
        return Err(invalid_params("create table requires at least one column"));
    }

    let mut defs = Vec::new();
    let mut inline_pk = Vec::new();
    for column in &p.spec.columns {
        defs.push(column_definition(column)?);
        if column.is_primary {
            inline_pk.push(column.name.clone());
        }
    }
    let pk = if p.spec.primary_key.is_empty() {
        inline_pk
    } else {
        p.spec.primary_key.clone()
    };
    if !pk.is_empty() {
        defs.push(format!("PRIMARY KEY ({})", quote_list(&pk)));
    }
    if p.options.with_foreign_keys {
        for fk in &p.spec.foreign_keys {
            defs.push(foreign_key_definition(fk));
        }
    }

    let mut head = vec!["CREATE".to_string()];
    if p.options.temporary {
        head.push("TEMPORARY".to_string());
    }
    head.push("TABLE".to_string());
    if p.options.if_not_exists {
        head.push("IF NOT EXISTS".to_string());
    }
    let sql = format!(
        "{} {} ({})",
        head.join(" "),
        qualified_name(
            p.spec.database.as_deref(),
            p.spec.schema.as_deref(),
            &p.spec.name
        ),
        defs.join(", ")
    );
    let mut statements = vec![sql.clone()];
    if p.options.with_indexes {
        for index in &p.spec.indexes {
            if index.columns.is_empty() {
                continue;
            }
            let mut stmt = "CREATE ".to_string();
            if index.is_unique {
                stmt.push_str("UNIQUE ");
            }
            stmt.push_str("INDEX ");
            stmt.push_str(&quote_identifier(&index.name));
            stmt.push_str(" ON ");
            stmt.push_str(&qualified_name(
                p.spec.database.as_deref(),
                p.spec.schema.as_deref(),
                &p.spec.name,
            ));
            if let Some(kind) = index.kind.as_deref().filter(|kind| !kind.trim().is_empty()) {
                stmt.push_str(" USING ");
                stmt.push_str(kind);
            }
            stmt.push_str(" (");
            stmt.push_str(&quote_list(&index.columns));
            stmt.push(')');
            if let Some(where_clause) = index
                .where_clause
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                stmt.push_str(" WHERE ");
                stmt.push_str(where_clause);
            }
            statements.push(stmt);
        }
    }
    if p.options.with_comments && !p.spec.comment.trim().is_empty() {
        statements.push(format!(
            "COMMENT ON TABLE {} IS {}",
            qualified_name(
                p.spec.database.as_deref(),
                p.spec.schema.as_deref(),
                &p.spec.name
            ),
            quote_literal(&p.spec.comment)
        ));
    }
    if p.options.with_comments {
        for column in &p.spec.columns {
            if !column.comment.trim().is_empty() {
                statements.push(format!(
                    "COMMENT ON COLUMN {}.{} IS {}",
                    qualified_name(
                        p.spec.database.as_deref(),
                        p.spec.schema.as_deref(),
                        &p.spec.name
                    ),
                    quote_identifier(&column.name),
                    quote_literal(&column.comment)
                ));
            }
        }
    }
    Ok(BuildCreateTableResult { sql, statements })
}

fn build_alter_table(p: BuildAlterTableParams) -> Result<BuildAlterTableResult, ProtocolError> {
    let table = if p.to_spec.name.trim().is_empty() {
        &p.from_spec
    } else {
        &p.to_spec
    };
    if table.name.trim().is_empty() {
        return Err(invalid_params("alter table requires table name"));
    }
    let table_name = qualified_name(
        table.database.as_deref(),
        table.schema.as_deref(),
        &table.name,
    );
    let mut statements = Vec::new();
    let mut rollback = Vec::new();
    let mut warnings = Vec::new();

    for rename in &p.column_renames {
        if rename.old_name.trim().is_empty() || rename.new_name.trim().is_empty() {
            continue;
        }
        statements.push(rename_column_sql(&table_name, rename));
        if p.options.with_rollback {
            rollback.insert(
                0,
                format!(
                    "ALTER TABLE {table_name} RENAME COLUMN {} TO {}",
                    quote_identifier(&rename.new_name),
                    quote_identifier(&rename.old_name)
                ),
            );
        }
    }

    let renamed_targets = p
        .column_renames
        .iter()
        .map(|rename| rename.new_name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for column in p.to_spec.columns.iter().filter(|column| {
        !renamed_targets.contains(column.name.as_str())
            && !p
                .from_spec
                .columns
                .iter()
                .any(|from| from.name == column.name)
    }) {
        statements.push(format!(
            "ALTER TABLE {table_name} ADD COLUMN {}",
            column_definition(column)?
        ));
        if p.options.with_rollback {
            rollback.insert(
                0,
                format!(
                    "ALTER TABLE {table_name} DROP COLUMN {}",
                    quote_identifier(&column.name)
                ),
            );
        }
        // ADD COLUMN 不会携带注释,新增列带非空注释时需补 COMMENT ON COLUMN。
        if !column.comment.trim().is_empty() {
            statements.push(format!(
                "COMMENT ON COLUMN {table_name}.{} IS {}",
                quote_identifier(&column.name),
                quote_literal(&column.comment)
            ));
        }
    }
    if p.options.allow_destructive {
        for column in p
            .from_spec
            .columns
            .iter()
            .filter(|column| !p.to_spec.columns.iter().any(|to| to.name == column.name))
        {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP COLUMN {}",
                quote_identifier(&column.name)
            ));
            warnings.push(format!("drop column may lose data: {}", column.name));
        }
    }
    diff_comments(
        &mut statements,
        &mut rollback,
        &table_name,
        &p.from_spec,
        &p.to_spec,
        p.options.with_rollback,
    );

    Ok(BuildAlterTableResult {
        statements,
        rollback_statements: rollback,
        warnings,
    })
}

/// 对比 from/to 的表注释与列注释差异,追加 COMMENT ON 语句。
/// with_rollback 时把「还原原注释」的语句插到回滚列表头部。
fn diff_comments(
    statements: &mut Vec<String>,
    rollback: &mut Vec<String>,
    table_name: &str,
    from_spec: &TableSpec,
    to_spec: &TableSpec,
    with_rollback: bool,
) {
    if from_spec.comment != to_spec.comment {
        statements.push(format!(
            "COMMENT ON TABLE {table_name} IS {}",
            quote_literal(&to_spec.comment)
        ));
        if with_rollback {
            rollback.insert(
                0,
                format!(
                    "COMMENT ON TABLE {table_name} IS {}",
                    quote_literal(&from_spec.comment)
                ),
            );
        }
    }
    let from_cols = from_spec
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column))
        .collect::<std::collections::HashMap<_, _>>();
    for column in &to_spec.columns {
        if column.name.is_empty() {
            continue;
        }
        let Some(from) = from_cols.get(column.name.as_str()) else {
            continue;
        };
        if from.comment == column.comment {
            continue;
        }
        statements.push(format!(
            "COMMENT ON COLUMN {table_name}.{} IS {}",
            quote_identifier(&column.name),
            quote_literal(&column.comment)
        ));
        if with_rollback {
            rollback.insert(
                0,
                format!(
                    "COMMENT ON COLUMN {table_name}.{} IS {}",
                    quote_identifier(&column.name),
                    quote_literal(&from.comment)
                ),
            );
        }
    }
}

fn build_drop(p: BuildDropParams) -> Result<BuildDropResult, ProtocolError> {
    if p.name.trim().is_empty() {
        return Err(invalid_params("drop requires object name"));
    }
    let kind = match p.kind {
        ObjectKind::Table => "TABLE",
        ObjectKind::View => "VIEW",
        ObjectKind::MaterializedView => "MATERIALIZED VIEW",
        ObjectKind::Index => "INDEX",
        ObjectKind::Sequence => "SEQUENCE",
        ObjectKind::Function => "FUNCTION",
        ObjectKind::Procedure => "PROCEDURE",
        ObjectKind::Trigger => "TRIGGER",
        ObjectKind::Type => "TYPE",
        other => return Err(invalid_params(format!("unsupported drop kind `{other:?}`"))),
    };
    let mut sql = format!("DROP {kind}");
    if p.if_exists {
        sql.push_str(" IF EXISTS");
    }
    sql.push(' ');
    sql.push_str(&qualified_name(
        p.database.as_deref(),
        p.schema.as_deref(),
        &p.name,
    ));
    if p.cascade {
        sql.push_str(" CASCADE");
    }
    Ok(BuildDropResult { sql })
}

fn column_definition(column: &ColumnSpec) -> Result<String, ProtocolError> {
    if column.name.trim().is_empty() {
        return Err(invalid_params("column name is required"));
    }
    if column.type_str.trim().is_empty() {
        return Err(invalid_params(format!(
            "column `{}` type is required",
            column.name
        )));
    }
    let mut parts = vec![quote_identifier(&column.name), column.type_str.clone()];
    if column.auto_increment {
        parts.push("GENERATED BY DEFAULT AS IDENTITY".to_string());
    }
    if !column.nullable {
        parts.push("NOT NULL".to_string());
    }
    if let Some(default) = column
        .default
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push("DEFAULT".to_string());
        parts.push(default.to_string());
    }
    if column.is_unique && !column.is_primary {
        parts.push("UNIQUE".to_string());
    }
    Ok(parts.join(" "))
}

fn foreign_key_definition(fk: &ForeignKeySpec) -> String {
    let mut parts = Vec::new();
    if !fk.name.trim().is_empty() {
        parts.push("CONSTRAINT".to_string());
        parts.push(quote_identifier(&fk.name));
    }
    parts.push("FOREIGN KEY".to_string());
    parts.push(format!("({})", quote_list(&fk.from_columns)));
    parts.push("REFERENCES".to_string());
    parts.push(quote_identifier(&fk.to_table));
    parts.push(format!("({})", quote_list(&fk.to_columns)));
    if let Some(on_delete) = fk
        .on_delete
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push("ON DELETE".to_string());
        parts.push(on_delete.replace('_', " ").to_ascii_uppercase());
    }
    if let Some(on_update) = fk
        .on_update
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push("ON UPDATE".to_string());
        parts.push(on_update.replace('_', " ").to_ascii_uppercase());
    }
    parts.join(" ")
}

fn rename_column_sql(table_name: &str, rename: &ColumnRenameSpec) -> String {
    format!(
        "ALTER TABLE {table_name} RENAME COLUMN {} TO {}",
        quote_identifier(&rename.old_name),
        quote_identifier(&rename.new_name)
    )
}

pub fn qualified_name(database: Option<&str>, schema: Option<&str>, name: &str) -> String {
    [database, schema, Some(name)]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .map(quote_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_list(values: &[String]) -> String {
    values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| quote_identifier(value))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_table_builds_identity_and_comments() {
        let result = build_create_table(BuildCreateTableParams {
            conn_id: None,
            spec: TableSpec {
                name: "users".into(),
                schema: Some("public".into()),
                columns: vec![ColumnSpec {
                    name: "id".into(),
                    type_str: "BIGINT".into(),
                    nullable: false,
                    is_primary: true,
                    auto_increment: true,
                    comment: "primary".into(),
                    ..Default::default()
                }],
                comment: "users table".into(),
                ..Default::default()
            },
            options: CreateTableOptions::default(),
        })
        .unwrap();

        assert!(result.sql.contains("GENERATED BY DEFAULT AS IDENTITY"));
        assert!(
            result
                .statements
                .iter()
                .any(|sql| sql.starts_with("COMMENT ON TABLE"))
        );
    }

    #[test]
    fn alter_table_renames_column() {
        let result = build_alter_table(BuildAlterTableParams {
            conn_id: None,
            from_spec: TableSpec {
                name: "events".into(),
                columns: vec![ColumnSpec {
                    name: "payload".into(),
                    type_str: "TEXT".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            to_spec: TableSpec {
                name: "events".into(),
                columns: vec![ColumnSpec {
                    name: "body".into(),
                    type_str: "TEXT".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            column_renames: vec![ColumnRenameSpec {
                old_name: "payload".into(),
                new_name: "body".into(),
            }],
            options: extension_protocol::ddl::AlterTableOptions {
                allow_destructive: false,
                with_rollback: true,
            },
        })
        .unwrap();

        assert_eq!(
            result.statements,
            vec![r#"ALTER TABLE "events" RENAME COLUMN "payload" TO "body""#]
        );
        assert_eq!(
            result.rollback_statements,
            vec![r#"ALTER TABLE "events" RENAME COLUMN "body" TO "payload""#]
        );
    }

    #[test]
    fn alter_table_diffs_comments_with_rollback() {
        let result = build_alter_table(BuildAlterTableParams {
            conn_id: None,
            from_spec: TableSpec {
                name: "events".into(),
                comment: "old".into(),
                columns: vec![ColumnSpec {
                    name: "payload".into(),
                    type_str: "TEXT".into(),
                    comment: "old payload".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            to_spec: TableSpec {
                name: "events".into(),
                comment: "new".into(),
                columns: vec![ColumnSpec {
                    name: "payload".into(),
                    type_str: "TEXT".into(),
                    comment: "new payload".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            column_renames: Vec::new(),
            options: extension_protocol::ddl::AlterTableOptions {
                allow_destructive: false,
                with_rollback: true,
            },
        })
        .unwrap();

        assert_eq!(
            result.statements,
            vec![
                "COMMENT ON TABLE \"events\" IS 'new'".to_string(),
                "COMMENT ON COLUMN \"events\".\"payload\" IS 'new payload'".to_string(),
            ]
        );
        assert_eq!(
            result.rollback_statements,
            vec![
                "COMMENT ON COLUMN \"events\".\"payload\" IS 'old payload'".to_string(),
                "COMMENT ON TABLE \"events\" IS 'old'".to_string(),
            ]
        );
    }

    #[test]
    fn alter_table_emits_comment_for_newly_added_column() {
        let result = build_alter_table(BuildAlterTableParams {
            conn_id: None,
            from_spec: TableSpec {
                name: "events".into(),
                columns: vec![ColumnSpec {
                    name: "payload".into(),
                    type_str: "TEXT".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            to_spec: TableSpec {
                name: "events".into(),
                columns: vec![
                    ColumnSpec {
                        name: "payload".into(),
                        type_str: "TEXT".into(),
                        ..Default::default()
                    },
                    ColumnSpec {
                        name: "note".into(),
                        type_str: "TEXT".into(),
                        comment: "note text".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            column_renames: Vec::new(),
            options: extension_protocol::ddl::AlterTableOptions {
                allow_destructive: false,
                with_rollback: true,
            },
        })
        .unwrap();

        assert_eq!(
            result.statements,
            vec![
                "ALTER TABLE \"events\" ADD COLUMN \"note\" TEXT NOT NULL".to_string(),
                "COMMENT ON COLUMN \"events\".\"note\" IS 'note text'".to_string(),
            ]
        );
        assert_eq!(
            result.rollback_statements,
            vec!["ALTER TABLE \"events\" DROP COLUMN \"note\"".to_string()]
        );
    }
}
