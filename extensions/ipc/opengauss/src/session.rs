use anyhow::{Context, Result, anyhow};
use base64::Engine;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use extension_protocol::row::{CellValue, ColumnSpec, ColumnTypeKind, Row};
use extension_protocol::schema::{
    CheckInfo, ColumnInfo, DatabaseInfo, DumpDdlOptions, ForeignKeyInfo, FunctionInfo, IndexInfo,
    ObjectInfo, ObjectKind, ObjectRef, SchemaInfo, SequenceInfo, TriggerInfo, TypeInfo,
    ViewDefinitionResult, ViewInfo,
};
use serde_json::json;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio_opengauss::tls::MakeTlsConnect;
use tokio_opengauss::types::{ToSql, Type};
use tokio_opengauss::{Client, NoTls, Socket};

use crate::config::OpenGaussConnectionConfig;

pub struct OpenGaussSession {
    runtime: Runtime,
    client: Client,
    connection_task: JoinHandle<()>,
}

impl OpenGaussSession {
    pub fn connect(cfg: OpenGaussConnectionConfig) -> Result<Self> {
        let runtime = Runtime::new().context("failed to create OpenGauss Tokio runtime")?;
        let endpoint = cfg.endpoint();
        let requires_tls_connector = cfg
            .requires_tls_connector()
            .map_err(anyhow::Error::msg)
            .context("invalid OpenGauss SSL mode")?;
        let client_config = cfg.to_client_config().map_err(anyhow::Error::msg)?;
        let (client, connection_task) = if requires_tls_connector {
            let tls = cfg
                .to_native_tls_connector()
                .map_err(anyhow::Error::msg)
                .context("invalid OpenGauss TLS certificate configuration")?;
            let tls = opengauss_native_tls::MakeTlsConnector::new(tls);
            Self::connect_client(&runtime, &client_config, tls)
        } else {
            Self::connect_client(&runtime, &client_config, NoTls)
        }
        .with_context(|| format!("failed to connect to OpenGauss at {endpoint}"))?;
        Ok(Self {
            runtime,
            client,
            connection_task,
        })
    }

    fn connect_client<T>(
        runtime: &Runtime,
        client_config: &tokio_opengauss::Config,
        tls: T,
    ) -> Result<(Client, JoinHandle<()>)>
    where
        T: MakeTlsConnect<Socket>,
        T::Stream: Send + 'static,
    {
        let (client, connection) = runtime.block_on(client_config.connect(tls))?;
        let connection_task = runtime.spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "OpenGauss connection task stopped");
            }
        });
        Ok((client, connection_task))
    }

    pub fn close(&mut self) {
        self.connection_task.abort();
    }

    pub fn ping(&mut self) -> Result<()> {
        self.runtime
            .block_on(self.client.simple_query("SELECT 1"))
            .context("failed to ping OpenGauss with SELECT 1")
            .map(|_| ())
    }

    pub fn server_version(&mut self) -> Result<String> {
        self.query_single_string("SELECT version()")
    }

    pub fn current_database(&mut self) -> Result<String> {
        self.query_single_string("SELECT current_database()")
    }

    pub fn query(&mut self, sql: &str, params: &[CellValue]) -> Result<QueryResult> {
        let params = bind_params(params)?;
        let refs = param_refs(&params);
        let rows = self
            .runtime
            .block_on(self.client.query(sql, &refs))
            .with_context(|| format!("failed to execute OpenGauss query `{sql}`"))?;
        Ok(rows_to_query_result(rows))
    }

    pub fn execute(&mut self, sql: &str, params: &[CellValue]) -> Result<u64> {
        let params = bind_params(params)?;
        let refs = param_refs(&params);
        self.runtime
            .block_on(self.client.execute(sql, &refs))
            .with_context(|| format!("failed to execute OpenGauss statement `{sql}`"))
    }

    pub fn execute_batch(&mut self, statements: &[String]) -> Result<Vec<u64>> {
        statements
            .iter()
            .map(|statement| self.execute(statement, &[]))
            .collect()
    }

    pub fn simple_execute(&mut self, sql: &str) -> Result<()> {
        self.runtime
            .block_on(self.client.batch_execute(sql))
            .with_context(|| format!("failed to execute OpenGauss batch `{sql}`"))
    }

    pub fn list_databases(&mut self) -> Result<Vec<DatabaseInfo>> {
        let rows = self.runtime.block_on(self.client.query(
            "SELECT datname, pg_catalog.pg_get_userbyid(datdba) AS owner \
             FROM pg_database WHERE datallowconn ORDER BY datname",
            &[],
        ))?;
        Ok(rows
            .into_iter()
            .map(|row| DatabaseInfo {
                name: row.get::<_, String>(0),
                owner: row.try_get::<_, Option<String>>(1).ok().flatten(),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_schemas(&mut self, database: Option<&str>) -> Result<Vec<SchemaInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let rows = self.runtime.block_on(self.client.query(
            "SELECT schema_name, schema_owner FROM information_schema.schemata ORDER BY schema_name",
            &[],
        ))?;
        Ok(rows
            .into_iter()
            .map(|row| SchemaInfo {
                name: row.get::<_, String>(0),
                owner: row.try_get::<_, Option<String>>(1).ok().flatten(),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_objects(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        kinds: &[ObjectKind],
    ) -> Result<Vec<ObjectInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let include_tables = kinds.is_empty() || kinds.contains(&ObjectKind::Table);
        let include_views = kinds.is_empty() || kinds.contains(&ObjectKind::View);
        let include_sequences = kinds.is_empty() || kinds.contains(&ObjectKind::Sequence);
        let include_functions = kinds.is_empty() || kinds.contains(&ObjectKind::Function);
        let include_types = kinds.is_empty() || kinds.contains(&ObjectKind::Type);
        let schema = schema.filter(|value| !value.trim().is_empty());
        let mut objects = Vec::new();

        if include_tables {
            let mut sql = "SELECT t.table_name, t.table_schema, \
                                  pg_catalog.obj_description(c.oid, 'pg_class') \
                           FROM information_schema.tables t \
                           LEFT JOIN pg_catalog.pg_namespace ns ON ns.nspname = t.table_schema \
                           LEFT JOIN pg_catalog.pg_class c \
                             ON c.relname = t.table_name AND c.relnamespace = ns.oid \
                           WHERE t.table_type = 'BASE TABLE'"
                .to_string();
            if let Some(schema) = schema {
                sql.push_str(" AND t.table_schema = ");
                sql.push_str(&quote_literal(schema));
            }
            sql.push_str(" ORDER BY t.table_schema, t.table_name");
            let rows = self
                .runtime
                .block_on(self.client.query(sql.as_str(), &[]))?;
            objects.extend(rows.into_iter().map(|row| {
                ObjectInfo {
                    name: row.get::<_, String>(0),
                    kind: ObjectKind::Table,
                    schema: row
                        .try_get::<_, Option<String>>(1)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    comment: row
                        .try_get::<_, Option<String>>(2)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: serde_json::Value::Null,
                }
            }));
        }

        if include_views {
            let mut sql = "SELECT t.table_name, t.table_schema, \
                                  pg_catalog.obj_description(c.oid, 'pg_class') \
                           FROM information_schema.views t \
                           LEFT JOIN pg_catalog.pg_namespace ns ON ns.nspname = t.table_schema \
                           LEFT JOIN pg_catalog.pg_class c \
                             ON c.relname = t.table_name AND c.relnamespace = ns.oid \
                           WHERE 1 = 1"
                .to_string();
            if let Some(schema) = schema {
                sql.push_str(" AND t.table_schema = ");
                sql.push_str(&quote_literal(schema));
            }
            sql.push_str(" ORDER BY t.table_schema, t.table_name");
            let rows = self
                .runtime
                .block_on(self.client.query(sql.as_str(), &[]))?;
            objects.extend(rows.into_iter().map(|row| {
                ObjectInfo {
                    name: row.get::<_, String>(0),
                    kind: ObjectKind::View,
                    schema: row
                        .try_get::<_, Option<String>>(1)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    comment: row
                        .try_get::<_, Option<String>>(2)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: serde_json::Value::Null,
                }
            }));
        }

        if include_sequences {
            for seq in self.list_sequences(database, schema)? {
                let seq_schema = seq
                    .extra
                    .get("schema")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                objects.push(ObjectInfo {
                    name: seq.name,
                    kind: ObjectKind::Sequence,
                    schema: seq_schema,
                    comment: String::new(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: seq.extra,
                });
            }
        }
        if include_functions {
            for function in self.list_functions(database, schema)? {
                let function_schema = function
                    .extra
                    .get("schema")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                objects.push(ObjectInfo {
                    name: function.name,
                    kind: ObjectKind::Function,
                    schema: function_schema,
                    comment: function.comment,
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: function.extra,
                });
            }
        }
        if include_types {
            for ty in self.list_types(database, schema)? {
                let type_schema = ty
                    .extra
                    .get("schema")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                objects.push(ObjectInfo {
                    name: ty.name,
                    kind: ObjectKind::Type,
                    schema: type_schema,
                    comment: String::new(),
                    row_count_estimate: None,
                    size_bytes: None,
                    created_at: None,
                    updated_at: None,
                    extra: ty.extra,
                });
            }
        }

        Ok(objects)
    }

    pub fn list_columns(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let schema = schema
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("public");
        let sql = format!(
            "SELECT c.ordinal_position, c.column_name, c.data_type, c.udt_name, c.is_nullable, c.column_default, \
                    c.character_maximum_length, c.numeric_precision, c.numeric_scale, \
                    pg_catalog.col_description(pc.oid, c.ordinal_position) \
             FROM information_schema.columns c \
             LEFT JOIN pg_catalog.pg_class pc \
               ON pc.relname = c.table_name \
              AND pc.relnamespace = (SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = c.table_schema) \
             WHERE c.table_schema = {} AND c.table_name = {} \
             ORDER BY c.ordinal_position",
            quote_literal(schema),
            quote_literal(table)
        );
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let ordinal: i32 = row.get(0);
                let type_str: String = row.get(2);
                ColumnInfo {
                    ordinal: ordinal.max(0) as u32,
                    name: row.get(1),
                    type_str: type_str.clone(),
                    raw_type: row
                        .try_get::<_, Option<String>>(3)
                        .ok()
                        .flatten()
                        .or(Some(type_str)),
                    nullable: row
                        .try_get::<_, String>(4)
                        .map(|value| value.eq_ignore_ascii_case("YES"))
                        .unwrap_or(true),
                    default: row.try_get::<_, Option<String>>(5).ok().flatten(),
                    max_length: row
                        .try_get::<_, Option<i32>>(6)
                        .ok()
                        .flatten()
                        .map(|v| v as u32),
                    precision: row
                        .try_get::<_, Option<i32>>(7)
                        .ok()
                        .flatten()
                        .map(|v| v as u32),
                    scale: row.try_get::<_, Option<i32>>(8).ok().flatten(),
                    comment: row
                        .try_get::<_, Option<String>>(9)
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    ..Default::default()
                }
            })
            .collect())
    }

    pub fn list_views(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Vec<ViewInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT table_schema, table_name, view_definition FROM information_schema.views WHERE 1 = 1".to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND table_schema = ");
            sql.push_str(&quote_literal(schema));
        }
        sql.push_str(" ORDER BY table_schema, table_name");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| ViewInfo {
                name: row.get::<_, String>(1),
                kind: ObjectKind::View,
                definition_sql: row
                    .try_get::<_, Option<String>>(2)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                comment: String::new(),
                extra: json!({ "schema": row.get::<_, String>(0) }),
            })
            .collect())
    }

    pub fn list_indexes(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: Option<&str>,
    ) -> Result<Vec<IndexInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT schemaname, tablename, indexname, indexdef \
                       FROM pg_indexes WHERE 1 = 1"
            .to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND schemaname = ");
            sql.push_str(&quote_literal(schema));
        }
        if let Some(table) = table.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND tablename = ");
            sql.push_str(&quote_literal(table));
        }
        sql.push_str(" ORDER BY schemaname, tablename, indexname");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| IndexInfo {
                name: row.get::<_, String>(2),
                table: row.get::<_, String>(1),
                columns: Vec::new(),
                is_unique: row
                    .try_get::<_, String>(3)
                    .map(|def| def.to_ascii_uppercase().starts_with("CREATE UNIQUE"))
                    .unwrap_or(false),
                extra: json!({
                    "schema": row.get::<_, String>(0),
                    "definition": row.get::<_, String>(3),
                }),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_foreign_keys(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKeyInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let schema = schema
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("public");
        let sql = format!(
            "SELECT tc.constraint_name, tc.table_name, kcu.column_name, \
                    ccu.table_name AS foreign_table_name, ccu.column_name AS foreign_column_name, \
                    rc.update_rule, rc.delete_rule, ccu.table_schema AS foreign_table_schema \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
             LEFT JOIN information_schema.referential_constraints rc \
               ON rc.constraint_name = tc.constraint_name AND rc.constraint_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = {} AND tc.table_name = {} \
             ORDER BY tc.constraint_name, kcu.ordinal_position",
            quote_literal(schema),
            quote_literal(table)
        );
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| ForeignKeyInfo {
                name: row.get(0),
                from_table: row.get(1),
                from_columns: vec![row.get(2)],
                to_table: row.get(3),
                to_columns: vec![row.get(4)],
                to_schema: row.try_get::<_, Option<String>>(7).ok().flatten(),
                on_update: row.try_get::<_, Option<String>>(5).ok().flatten(),
                on_delete: row.try_get::<_, Option<String>>(6).ok().flatten(),
                comment: String::new(),
            })
            .collect())
    }

    pub fn list_checks(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<CheckInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let schema = schema
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("public");
        let sql = format!(
            "SELECT tc.constraint_name, tc.table_name, cc.check_clause \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.check_constraints cc \
               ON cc.constraint_name = tc.constraint_name AND cc.constraint_schema = tc.constraint_schema \
             WHERE tc.constraint_type = 'CHECK' AND tc.table_schema = {} AND tc.table_name = {} \
             ORDER BY tc.constraint_name",
            quote_literal(schema),
            quote_literal(table)
        );
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| CheckInfo {
                name: row.get(0),
                table: row.get(1),
                definition: row.try_get::<_, Option<String>>(2).ok().flatten(),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_functions(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Vec<FunctionInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT n.nspname, p.proname, pg_catalog.pg_get_function_result(p.oid), l.lanname, pg_catalog.pg_get_functiondef(p.oid) \
                       FROM pg_catalog.pg_proc p \
                       JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                       JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
                       WHERE n.nspname NOT LIKE 'pg_toast%'"
            .to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND n.nspname = ");
            sql.push_str(&quote_literal(schema));
        }
        sql.push_str(" ORDER BY n.nspname, p.proname");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| FunctionInfo {
                name: row.get::<_, String>(1),
                return_type: row.try_get::<_, Option<String>>(2).ok().flatten(),
                language: row.try_get::<_, Option<String>>(3).ok().flatten(),
                definition: row.try_get::<_, Option<String>>(4).ok().flatten(),
                extra: json!({ "schema": row.get::<_, String>(0) }),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_procedures(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Vec<FunctionInfo>> {
        self.list_functions(database, schema)
    }

    pub fn list_triggers(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: Option<&str>,
    ) -> Result<Vec<TriggerInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT trigger_schema, event_object_table, trigger_name, action_timing, event_manipulation, action_statement \
                       FROM information_schema.triggers WHERE 1 = 1"
            .to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND trigger_schema = ");
            sql.push_str(&quote_literal(schema));
        }
        if let Some(table) = table.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND event_object_table = ");
            sql.push_str(&quote_literal(table));
        }
        sql.push_str(" ORDER BY trigger_schema, event_object_table, trigger_name");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| TriggerInfo {
                name: row.get(2),
                table: row.get(1),
                timing: row.get::<_, String>(3).to_ascii_lowercase(),
                event: row.get::<_, String>(4).to_ascii_lowercase(),
                definition: row.try_get::<_, Option<String>>(5).ok().flatten(),
                comment: String::new(),
                extra: json!({ "schema": row.get::<_, String>(0) }),
            })
            .collect())
    }

    pub fn list_sequences(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Vec<SequenceInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT sequence_schema, sequence_name, start_value, minimum_value, maximum_value, increment \
                       FROM information_schema.sequences WHERE 1 = 1"
            .to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND sequence_schema = ");
            sql.push_str(&quote_literal(schema));
        }
        sql.push_str(" ORDER BY sequence_schema, sequence_name");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| SequenceInfo {
                name: row.get(1),
                start_value: row
                    .try_get::<_, Option<String>>(2)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok()),
                min_value: row
                    .try_get::<_, Option<String>>(3)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok()),
                max_value: row
                    .try_get::<_, Option<String>>(4)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok()),
                increment: row
                    .try_get::<_, Option<String>>(5)
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok()),
                extra: json!({ "schema": row.get::<_, String>(0) }),
                ..Default::default()
            })
            .collect())
    }

    pub fn list_types(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Vec<TypeInfo>> {
        if !self.database_matches(database)? {
            return Ok(Vec::new());
        }
        let mut sql = "SELECT n.nspname, t.typname, t.typtype \
                       FROM pg_catalog.pg_type t \
                       JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                       WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
                         AND n.nspname NOT LIKE 'pg_toast%'"
            .to_string();
        if let Some(schema) = schema.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND n.nspname = ");
            sql.push_str(&quote_literal(schema));
        }
        sql.push_str(" ORDER BY n.nspname, t.typname");
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .into_iter()
            .map(|row| TypeInfo {
                name: row.get(1),
                kind: pg_type_kind(&row.get::<_, String>(2)),
                definition: None,
                extra: json!({ "schema": row.get::<_, String>(0) }),
            })
            .collect())
    }

    pub fn view_definition(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        view: &str,
    ) -> Result<ViewDefinitionResult> {
        if !self.database_matches(database)? {
            return Ok(ViewDefinitionResult {
                sql: String::new(),
                is_materialized: false,
            });
        }
        let schema = schema
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("public");
        let sql = format!(
            "SELECT pg_catalog.pg_get_viewdef(c.oid, true), c.relkind = 'm' \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relname = {} AND c.relkind IN ('v','m')",
            quote_literal(schema),
            quote_literal(view)
        );
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]))?;
        Ok(rows
            .first()
            .map(|row| ViewDefinitionResult {
                sql: row
                    .try_get::<_, Option<String>>(0)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                is_materialized: row.try_get::<_, bool>(1).unwrap_or(false),
            })
            .unwrap_or(ViewDefinitionResult {
                sql: String::new(),
                is_materialized: false,
            }))
    }

    pub fn dump_ddl(
        &mut self,
        objects: &[ObjectRef],
        options: &DumpDdlOptions,
    ) -> Result<Vec<String>> {
        let mut statements = Vec::new();
        for object in objects {
            match object.kind {
                ObjectKind::View | ObjectKind::MaterializedView => {
                    let def = self.view_definition(
                        object.database.as_deref(),
                        object.schema.as_deref(),
                        &object.name,
                    )?;
                    if !def.sql.trim().is_empty() {
                        let create = if def.is_materialized {
                            "CREATE MATERIALIZED VIEW"
                        } else {
                            "CREATE VIEW"
                        };
                        let exists = if options.if_not_exists {
                            " IF NOT EXISTS"
                        } else {
                            ""
                        };
                        statements.push(format!(
                            "{create}{exists} {} AS {}",
                            crate::ddl::qualified_name(
                                object.database.as_deref(),
                                object.schema.as_deref(),
                                &object.name
                            ),
                            def.sql
                        ));
                    }
                }
                ObjectKind::Table => {
                    let qualified = crate::ddl::qualified_name(
                        object.database.as_deref(),
                        object.schema.as_deref(),
                        &object.name,
                    );
                    match self.table_definition(
                        object.database.as_deref(),
                        object.schema.as_deref(),
                        &object.name,
                    )? {
                        Some(ddl) => statements.push(ddl),
                        None => statements.push(format!(
                            "-- DDL dump for table {qualified} requires server-side pg_get_tabledef support"
                        )),
                    }
                }
                _ => {}
            }
        }
        Ok(statements)
    }

    /// Returns the official CREATE TABLE definition produced by the server, or
    /// `None` when the table does not exist or the server does not expose
    /// `pg_get_tabledef` (older openGauss builds).
    fn table_definition(
        &mut self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Option<String>> {
        if !self.database_matches(database)? {
            return Ok(None);
        }
        let schema = schema
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("public");
        let sql = format!(
            "SELECT pg_catalog.pg_get_tabledef(c.oid) \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relname = {} AND c.relkind IN ('r','p','f')",
            quote_literal(schema),
            quote_literal(table)
        );
        let rows = self
            .runtime
            .block_on(self.client.query(sql.as_str(), &[]));
        Ok(rows.ok().and_then(|rows| {
            rows.first().and_then(|row| {
                row.try_get::<_, Option<String>>(0).ok().flatten()
            })
        }))
    }

    fn query_single_string(&mut self, sql: &str) -> Result<String> {
        let rows = self.runtime.block_on(self.client.query(sql, &[]))?;
        rows.first()
            .map(|row| row.get::<_, String>(0))
            .ok_or_else(|| anyhow!("query `{sql}` returned no rows"))
    }

    fn database_matches(&mut self, database: Option<&str>) -> Result<bool> {
        let Some(database) = database.filter(|value| !value.trim().is_empty()) else {
            return Ok(true);
        };
        Ok(database == self.current_database()?)
    }
}

pub struct QueryResult {
    pub columns: Vec<ColumnSpec>,
    pub rows: Vec<Row>,
}

fn rows_to_query_result(rows: Vec<tokio_opengauss::Row>) -> QueryResult {
    let columns = rows
        .first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|column| column_to_spec(column.name(), column.type_()))
                .collect()
        })
        .unwrap_or_default();
    let rows = rows
        .iter()
        .map(|row| {
            row.columns()
                .iter()
                .enumerate()
                .map(|(idx, column)| row_cell_value(row, idx, column.type_()))
                .collect()
        })
        .collect();
    QueryResult { columns, rows }
}

pub fn bind_params(params: &[CellValue]) -> Result<Vec<Box<dyn ToSql + Sync>>> {
    params.iter().map(bind_param).collect()
}

fn bind_param(param: &CellValue) -> Result<Box<dyn ToSql + Sync>> {
    Ok(match param {
        CellValue::Null => Box::new(Option::<String>::None),
        CellValue::Bool { value } => Box::new(*value),
        CellValue::I64 { value } => Box::new(*value),
        CellValue::U64 { value } => {
            if *value <= i64::MAX as u64 {
                Box::new(*value as i64)
            } else {
                Box::new(value.to_string())
            }
        }
        CellValue::F64 { value } => Box::new(*value),
        CellValue::Bytes { value } => Box::new(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .context("invalid base64 bytes parameter")?,
        ),
        CellValue::Json { value } => Box::new(value.clone()),
        CellValue::Date { value } => Box::new(NaiveDate::parse_from_str(value, "%Y-%m-%d")?),
        CellValue::Time { value } => Box::new(NaiveTime::parse_from_str(value, "%H:%M:%S%.f")?),
        CellValue::Datetime { value } => {
            if let Ok(value) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
                Box::new(value)
            } else {
                Box::new(value.clone())
            }
        }
        CellValue::Decimal { value }
        | CellValue::Text { value }
        | CellValue::Uuid { value }
        | CellValue::Duration { value }
        | CellValue::Geo { value, .. } => Box::new(value.clone()),
        CellValue::Array { value, .. } => Box::new(serde_json::to_value(value)?),
        CellValue::Map { value } => Box::new(serde_json::Value::Object(value.clone())),
        CellValue::Custom { raw, .. } => Box::new(raw.clone()),
    })
}

pub fn param_refs(params: &[Box<dyn ToSql + Sync>]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|param| param.as_ref()).collect()
}

fn column_to_spec(name: &str, ty: &Type) -> ColumnSpec {
    ColumnSpec::new(name, ty.name(), column_type_kind(ty)).nullable(true)
}

fn column_type_kind(ty: &Type) -> ColumnTypeKind {
    match ty.name() {
        "bool" => ColumnTypeKind::Bool,
        "int2" | "int4" | "int8" | "oid" => ColumnTypeKind::I64,
        "float4" | "float8" => ColumnTypeKind::F64,
        "numeric" | "money" => ColumnTypeKind::Decimal,
        "json" | "jsonb" => ColumnTypeKind::Json,
        "bytea" => ColumnTypeKind::Bytes,
        "date" => ColumnTypeKind::Date,
        "time" | "timetz" => ColumnTypeKind::Time,
        "timestamp" | "timestamptz" => ColumnTypeKind::Datetime,
        _ => ColumnTypeKind::Text,
    }
}

fn row_cell_value(row: &tokio_opengauss::Row, idx: usize, ty: &Type) -> CellValue {
    if ty == &Type::BOOL {
        return row
            .try_get::<_, Option<bool>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Bool { value })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::INT2 {
        return option_i16(row, idx)
            .map(|value| CellValue::I64 {
                value: value as i64,
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::INT4 {
        return option_i32(row, idx)
            .map(|value| CellValue::I64 {
                value: value as i64,
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::INT8 {
        return row
            .try_get::<_, Option<i64>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::I64 { value })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::FLOAT4 {
        return row
            .try_get::<_, Option<f32>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::F64 {
                value: value as f64,
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::FLOAT8 {
        return row
            .try_get::<_, Option<f64>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::F64 { value })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::BYTEA {
        return row
            .try_get::<_, Option<Vec<u8>>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Bytes {
                value: base64::engine::general_purpose::STANDARD.encode(value),
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::JSON || ty == &Type::JSONB {
        return row
            .try_get::<_, Option<serde_json::Value>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Json { value })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::DATE {
        return row
            .try_get::<_, Option<NaiveDate>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Date {
                value: value.to_string(),
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::TIME {
        return row
            .try_get::<_, Option<NaiveTime>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Time {
                value: value.to_string(),
            })
            .unwrap_or(CellValue::Null);
    }
    if ty == &Type::TIMESTAMP {
        return row
            .try_get::<_, Option<NaiveDateTime>>(idx)
            .ok()
            .flatten()
            .map(|value| CellValue::Datetime {
                value: value.to_string(),
            })
            .unwrap_or(CellValue::Null);
    }

    row.try_get::<_, Option<String>>(idx)
        .ok()
        .flatten()
        .map(|value| CellValue::Text { value })
        .unwrap_or(CellValue::Null)
}

fn option_i16(row: &tokio_opengauss::Row, idx: usize) -> Option<i16> {
    row.try_get::<_, Option<i16>>(idx).ok().flatten()
}

fn option_i32(row: &tokio_opengauss::Row, idx: usize) -> Option<i32> {
    row.try_get::<_, Option<i32>>(idx).ok().flatten()
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn pg_type_kind(value: &str) -> String {
    match value {
        "e" => "enum",
        "c" => "composite",
        "d" => "domain",
        "r" => "range",
        _ => "udt",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_literal_doubles_single_quotes() {
        assert_eq!("'a''b'", quote_literal("a'b"));
    }
}
