use iotdb_driver::config::IotDbConnectionConfig;
use iotdb_driver::session::IotDbSession;
use serde_json::json;

fn integration_config() -> IotDbConnectionConfig {
    IotDbConnectionConfig {
        host: std::env::var("ONETCLI_IOTDB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: std::env::var("ONETCLI_IOTDB_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6667),
        username: std::env::var("ONETCLI_IOTDB_USERNAME").unwrap_or_else(|_| "root".to_string()),
        password: std::env::var("ONETCLI_IOTDB_PASSWORD").unwrap_or_else(|_| "root".to_string()),
        database: std::env::var("ONETCLI_IOTDB_DATABASE")
            .unwrap_or_else(|_| "root.onetcli_smoke".to_string()),
        time_zone: "UTC+8".to_string(),
        timeout_ms: 30_000,
        fetch_size: 1_000,
        rpc_compaction: false,
        enable_redirect_query: false,
    }
}

#[test]
#[ignore = "needs a running local IoTDB: ONETCLI_IOTDB_INTEGRATION=1"]
fn local_iotdb_end_to_end() {
    if std::env::var("ONETCLI_IOTDB_INTEGRATION").as_deref() != Ok("1") {
        return;
    }

    let cfg = integration_config();
    let mut session = IotDbSession::connect(cfg.clone()).expect("connect");

    session.ping().expect("ping");

    // Drop any leftover so the test is repeatable.
    let _ = session.exec_update("DELETE STORAGE GROUP root.onetcli_smoke");

    // Create storage group + two timeseries.
    session
        .exec_update("CREATE STORAGE GROUP root.onetcli_smoke")
        .expect("create storage group");
    session
        .exec_update(
            "CREATE TIMESERIES root.onetcli_smoke.d1.temperature WITH DATATYPE=FLOAT, ENCODING=GORILLA",
        )
        .expect("create temperature timeseries");
    session
        .exec_update(
            "CREATE TIMESERIES root.onetcli_smoke.d1.status WITH DATATYPE=BOOLEAN, ENCODING=PLAIN",
        )
        .expect("create status timeseries");

    // Insert a row.
    session
        .exec_update("INSERT INTO root.onetcli_smoke.d1(timestamp, temperature, status) VALUES (1, 42.5, true)")
        .expect("insert row");

    // Query it back.
    let parsed = session
        .query("SELECT temperature, status FROM root.onetcli_smoke.d1")
        .expect("select");
    assert!(
        parsed.columns.iter().any(|c| c.contains("temperature")),
        "columns = {:?}",
        parsed.columns
    );
    assert_eq!(parsed.rows.len(), 1, "rows = {:?}", parsed.rows);

    // Metadata: storage groups should include ours.
    let databases = session.list_databases().expect("list_databases");
    assert!(
        databases.iter().any(|d| d.name == "root.onetcli_smoke"),
        "databases = {:?}",
        databases
    );

    // Metadata: columns for the device.
    let columns = session
        .list_columns("root.onetcli_smoke.d1")
        .expect("list_columns");
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"Time"), "columns = {:?}", names);
    assert!(names.contains(&"temperature"), "columns = {:?}", names);
    assert!(names.contains(&"status"), "columns = {:?}", names);

    // DDL builder path: CREATE TIMESERIES via the declarative builder.
    let ddl = iotdb_driver::ddl::handle_ddl_build_create_table(&json!({
        "spec": {
            "name": "root.onetcli_smoke.d1",
            "columns": [
                {"name": "temperature", "type": "FLOAT"},
                {"name": "status", "type": "BOOLEAN"}
            ]
        },
        "options": {"if_not_exists": true}
    }))
    .expect("ddl build create table");
    let sql = ddl["sql"].as_str().expect("sql string");
    assert!(
        sql.contains(
            "CREATE TIMESERIES IF NOT EXISTS root.onetcli_smoke.d1.temperature WITH DATATYPE=FLOAT"
        ),
        "sql = {sql}"
    );

    // DDL builder path: DROP device.
    let drop = iotdb_driver::ddl::handle_ddl_build_drop(&json!({
        "kind": "table",
        "name": "root.onetcli_smoke.d1",
        "if_exists": true,
        "cascade": true
    }))
    .expect("ddl build drop");
    assert_eq!(
        drop["sql"], "DELETE TIMESERIES root.onetcli_smoke.d1.**",
        "drop = {drop}"
    );

    session.close();
}
