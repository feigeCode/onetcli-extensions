use std::collections::BTreeSet;
use std::time::Duration;

use extension_host::{FramedTransport, JsonRpcClient, RequestOptions};
use extension_protocol::lifecycle::{InitParams, InitResult};
use extension_protocol::method;
use serde_json::{Value, json};

#[tokio::test]
async fn opengauss_driver_declares_manifest_methods() {
    let (client_stream, server_stream) = tokio::io::duplex(1024 * 1024);
    let (server_reader, server_writer) = tokio::io::split(server_stream);
    let server = tokio::spawn(async move {
        opengauss_driver::server::handle_stream(server_reader, server_writer).await
    });

    let (reader, writer) = tokio::io::split(client_stream);
    let client = JsonRpcClient::start(FramedTransport::new(reader, writer));
    let handle = client.handle();
    let timeout = RequestOptions::default().with_timeout(Duration::from_secs(2));

    let ping: Value = handle
        .call(method::PING, json!({}), timeout.clone())
        .await
        .expect("$/ping should succeed");
    assert_eq!(Some(true), ping["pong"].as_bool());

    let init: InitResult = handle
        .call(
            method::INIT,
            serde_json::to_value(InitParams::new("onetcli-test", "opengauss-test")).unwrap(),
            timeout.clone(),
        )
        .await
        .expect("init should succeed");
    assert!(
        init.drivers_ready
            .iter()
            .any(|driver| driver == "opengauss")
    );

    for method_name in declared_driver_methods() {
        assert!(
            init.declares_method(&method_name),
            "init result should declare {method_name}"
        );
    }

    let manifest_methods: BTreeSet<String> = declared_driver_methods().into_iter().collect();
    let init_methods: BTreeSet<String> = init.methods.into_iter().collect();
    assert_eq!(manifest_methods, init_methods);

    let bad_open = handle
        .call::<Value>(
            method::CONN_OPEN,
            json!({ "driver_id": "duckdb", "config": { "host": "127.0.0.1" } }),
            timeout.clone(),
        )
        .await
        .expect_err("wrong driver_id should be rejected");
    assert!(bad_open.to_string().contains("unsupported driver_id"));

    let _: Value = handle
        .call(method::SHUTDOWN, json!({}), timeout)
        .await
        .expect("shutdown should succeed");
    let _ = server.await.expect("server task should join");
}

#[test]
fn manifest_methods_match_handler_declarations() {
    assert_eq!(
        declared_driver_methods(),
        opengauss_driver::handlers::declared_methods()
            .iter()
            .map(|method| method.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn opengauss_manifest_exposes_full_shared_method_surface() {
    let expected_methods = [
        "$/ping",
        "shutdown",
        "conn/test",
        "conn/open",
        "conn/close",
        "conn/ping",
        "conn/use",
        "query/start",
        "cursor/fetch",
        "cursor/close",
        "cursor/cancel",
        "exec/run",
        "exec/batch",
        "tx/begin",
        "tx/commit",
        "tx/rollback",
        "tx/savepoint",
        "tx/release",
        "ddl/build",
        "ddl/build_create_table",
        "ddl/build_alter_table",
        "ddl/build_drop",
        "data/export",
        "data/import_begin",
        "data/import_chunk",
        "data/import_commit",
        "data/import_abort",
        "stream/read",
        "stream/close",
        "schema/object_view",
        "schema/databases",
        "schema/schemas",
        "schema/objects",
        "schema/columns",
        "schema/indexes",
        "schema/foreign_keys",
        "schema/checks",
        "schema/views",
        "schema/functions",
        "schema/procedures",
        "schema/triggers",
        "schema/sequences",
        "schema/types",
        "schema/view_definition",
        "schema/dump_ddl",
    ];

    assert_eq!(declared_driver_methods(), expected_methods);
}

#[test]
fn opengauss_manifest_reuses_host_postgresql_ssl_tab_contract() {
    let manifest = declared_manifest();
    let tabs = manifest["ui"]["form"]["forms"][0]["tabs"]
        .as_array()
        .expect("connection form tabs are an array");
    let tab_ids = tabs
        .iter()
        .map(|tab| tab["id"].as_str().expect("tab id is string"))
        .collect::<Vec<_>>();
    assert_eq!(tab_ids, ["general", "ssl", "ssh", "remark"]);

    let ssl_tab = tabs
        .iter()
        .find(|tab| tab["id"] == "ssl")
        .expect("ssl tab is declared");
    let ssl_field_ids = ssl_tab["fields"]
        .as_array()
        .expect("ssl tab fields are an array")
        .iter()
        .map(|field| field["id"].as_str().expect("field id is string"))
        .collect::<Vec<_>>();
    assert_eq!(
        ssl_field_ids,
        [
            "ssl_mode",
            "ssl_root_cert_path",
            "ssl_accept_invalid_certs",
            "ssl_accept_invalid_hostnames",
        ]
    );
}

#[test]
fn opengauss_manifest_declares_host_managed_empty_tabs() {
    let manifest = declared_manifest();
    let tabs = manifest["ui"]["form"]["forms"][0]["tabs"]
        .as_array()
        .expect("connection form tabs are an array");
    for tab_id in ["ssh", "remark"] {
        let tab = tabs
            .iter()
            .find(|tab| tab["id"] == tab_id)
            .unwrap_or_else(|| panic!("{tab_id} tab is declared"));
        assert!(
            tab["fields"]
                .as_array()
                .expect("host-managed tab fields are an array")
                .is_empty(),
            "{tab_id} tab should let the host provide its managed fields",
        );
    }
}

fn declared_driver_methods() -> Vec<String> {
    declared_manifest()["methods"]
        .as_array()
        .expect("methods is an array")
        .iter()
        .map(|method| method.as_str().expect("method is string").to_string())
        .collect()
}

fn declared_manifest() -> Value {
    let manifest: Value =
        serde_json::from_str(include_str!("../driver.json")).expect("driver.json is valid json");
    manifest
}
