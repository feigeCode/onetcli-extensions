use std::{fs, net::Ipv4Addr, ops::Deref, path::Path, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use extension_host::{
    HostApiHandler, HostApiProvider, HostError, HostResult, NegotiationConfig, ProcessRpcSession,
    ProcessRpcSessionConfig, SpawnConfig, UniversalPluginClient,
};
use extension_protocol::{
    blob::{BlobCloseParams, BlobReadParams, MAX_BLOB_CHUNK_BYTES},
    error::{ProtocolError, error_codes},
    event_stream::{EventCloseParams, EventOpenParams, EventReadParams},
    host,
    job::{
        JobCloseParams, JobResultParams, JobStartParams, JobState, JobStatusParams, ProgressPercent,
    },
    resource::{ResourceCloseParams, ResourceInvokeParams, ResourceOpenParams, ResourcePingParams},
    result_ref::ResultRef,
};
use serde_json::{Value, json};

fn copy_executable(source: &Path, destination: &Path) {
    fs::copy(source, destination).expect("copy provider executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(destination)
            .expect("provider executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(destination, permissions).expect("make provider executable");
    }
}

struct TestHostApi {
    secret_allowed: bool,
}

#[async_trait::async_trait]
impl HostApiProvider for TestHostApi {
    async fn request_credential(
        &self,
        _params: host::RequestCredentialParams,
    ) -> HostResult<host::RequestCredentialResult> {
        Err(HostError::NotImplemented(
            "interactive credential requests are not used by this test host".into(),
        ))
    }

    async fn resolve_secret(
        &self,
        params: host::ResolveSecretParams,
    ) -> HostResult<host::ResolveSecretResult> {
        if !self.secret_allowed {
            return Err(HostError::protocol(ProtocolError::new(
                error_codes::PERMISSION_DENIED,
                "extension is not permitted to read this secret",
            )));
        }
        if params.secret_ref.secret_ref != "secret://elasticsearch/api_key" {
            return Err(HostError::protocol(ProtocolError::new(
                error_codes::SECRET_NOT_FOUND,
                "requested secret was not found",
            )));
        }
        Ok(host::ResolveSecretResult {
            value: b"token-value".to_vec(),
        })
    }

    async fn notify(&self, _params: host::NotifyParams) -> HostResult<host::NotifyResult> {
        Err(HostError::NotImplemented(
            "notifications are not used by this test host".into(),
        ))
    }

    async fn storage_get(
        &self,
        _params: host::StorageGetParams,
    ) -> HostResult<host::StorageGetResult> {
        Err(HostError::NotImplemented(
            "storage is not used by this test host".into(),
        ))
    }

    async fn storage_set(&self, _params: host::StorageSetParams) -> HostResult<()> {
        Err(HostError::NotImplemented(
            "storage is not used by this test host".into(),
        ))
    }

    async fn log(&self, _params: host::LogParams) -> HostResult<()> {
        Err(HostError::NotImplemented(
            "host logging is not used by this test host".into(),
        ))
    }
}

struct TestPluginClient {
    inner: UniversalPluginClient,
    allowed_port: u16,
}

impl TestPluginClient {
    fn new(inner: UniversalPluginClient, allowed_port: u16) -> Self {
        Self {
            inner,
            allowed_port,
        }
    }

    async fn open_resource(
        &self,
        params: &ResourceOpenParams,
    ) -> HostResult<extension_protocol::resource::ResourceOpenResult> {
        let raw_url = params
            .config
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                HostError::protocol(ProtocolError::new(
                    error_codes::INVALID_PARAMS,
                    "resource URL is required",
                ))
            })?;
        let url = raw_url.parse::<elasticsearch::http::Url>().map_err(|_| {
            HostError::protocol(ProtocolError::new(
                error_codes::INVALID_PARAMS,
                "resource URL is invalid",
            ))
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(HostError::protocol(ProtocolError::new(
                error_codes::INVALID_PARAMS,
                "resource URL must be an HTTP(S) endpoint",
            )));
        }
        let endpoint_allowed = url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("127.0.0.1"))
            && url.port_or_known_default() == Some(self.allowed_port);
        if !endpoint_allowed {
            return Err(HostError::protocol(ProtocolError::new(
                error_codes::PERMISSION_DENIED,
                "extension is not permitted to connect to this network endpoint",
            )));
        }
        self.inner.open_resource(params).await
    }
}

impl Deref for TestPluginClient {
    type Target = UniversalPluginClient;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug)]
struct RecordedRequest {
    method: String,
    target: String,
    authorization: Option<String>,
    body: String,
}

async fn spawn_http_fixture_with_version(
    listener: TcpListener,
    version: &'static str,
) -> Arc<std::sync::Mutex<Vec<RecordedRequest>>> {
    let (records_tx, mut records_rx) = tokio::sync::mpsc::unbounded_channel();
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let collector_records = Arc::clone(&records);
    tokio::spawn(async move {
        while let Some(record) = records_rx.recv().await {
            collector_records.lock().expect("records lock").push(record);
        }
    });
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = records_tx.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let read = socket.read(&mut chunk).await.expect("read HTTP request");
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(header_end) = find_header_end(&buffer) {
                        let header = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                        if let Some(length) = content_length(&header) {
                            if buffer.len() >= header_end + 4 + length {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                let Some(header_end) = find_header_end(&buffer) else {
                    return;
                };
                let raw_request = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let mut lines = raw_request.lines();
                let request_line = lines.next().unwrap_or_default().to_owned();
                let method = request_line
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let target = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let authorization = lines.find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("authorization")
                        .then(|| value.trim().to_owned())
                });
                let body_start = header_end + 4;
                let body = String::from_utf8_lossy(&buffer.get(body_start..).unwrap_or_default())
                    .to_string();
                let response = match target.as_str() {
                    "/" => {
                        json!({"version":{"number":version},"cluster_name":"fixture"}).to_string()
                    }
                    target if target.starts_with("/_cat/indices?") => json!([
                        {"index":"orders","health":"green","docs.count":"12345","store.size":"2mb"},
                        {"index":"users","health":"yellow","docs.count":"802","store.size":"100kb"}
                    ])
                    .to_string(),
                    "/orders" => json!({"orders":{"aliases":{},"settings":{}}}).to_string(),
                    "/_search" => {
                        let body =
                            String::from_utf8_lossy(buffer.get(body_start..).unwrap_or_default())
                                .to_string();
                        if body.contains("\"query\":\"delayed\"") {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        if body.contains("\"query\":\"large\"") {
                            json!({"took":3,"hits":{"total":{"value":1},"hits":[{"_source":{"payload":"x".repeat(5 * 1024 * 1024)}}]}}).to_string()
                        } else {
                            json!({"took":3,"hits":{"total":{"value":2},"hits":[]}}).to_string()
                        }
                    }
                    _ => "{\"error\":\"not found\"}".to_owned(),
                };
                let bytes = response.as_bytes();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    bytes.len(),
                    response
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write HTTP response");
                let _ = socket.shutdown().await;
                tx.send(RecordedRequest {
                    method,
                    target,
                    authorization,
                    body,
                })
                .expect("record HTTP request");
            });
        }
    });
    records
}

async fn spawn_http_fixture(listener: TcpListener) -> Arc<std::sync::Mutex<Vec<RecordedRequest>>> {
    spawn_http_fixture_with_version(listener, "9.1.0").await
}

#[tokio::test]
async fn async_search_can_be_cancelled_while_running() {
    let harness = harness(true).await;
    let opened = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open resource");
    let job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"delayed"}),
        })
        .await
        .expect("start delayed job");
    assert_eq!(JobState::Running, job.state);

    harness
        .client
        .cancel_job(&extension_protocol::job::JobCancelParams {
            job_id: job.job_id.clone(),
        })
        .await
        .expect("cancel job");
    let status = harness
        .client
        .job_status(&JobStatusParams {
            job_id: job.job_id.clone(),
        })
        .await
        .expect("cancelled job status");
    assert_eq!(JobState::Cancelled, status.state);

    let result = harness
        .client
        .job_result(&JobResultParams {
            job_id: job.job_id.clone(),
        })
        .await
        .expect_err("cancelled job result");
    assert!(matches!(
        result,
        extension_host::HostError::Protocol(ref error)
            if error.code == extension_protocol::error::error_codes::REQUEST_CANCELLED
    ));
    harness
        .client
        .close_job(&JobCloseParams { job_id: job.job_id })
        .await
        .expect("close cancelled job");
    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: opened.resource_id,
        })
        .await
        .expect("close resource");
    harness.session.shutdown().await;
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(header: &str) -> Option<usize> {
    header.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}
struct TestHarness {
    client: TestPluginClient,
    session: Arc<ProcessRpcSession>,
    root: tempfile::TempDir,
    records: Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
    port: u16,
    cloned_session: extension_host::ProcessRpcSession,
}

async fn harness(secret_allowed: bool) -> TestHarness {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind HTTP fixture");
    let port = listener.local_addr().expect("HTTP fixture address").port();
    let records = spawn_http_fixture(listener).await;
    build_harness(secret_allowed, port, records).await
}

async fn harness_with_port(secret_allowed: bool, port: u16) -> TestHarness {
    build_harness(
        secret_allowed,
        port,
        Arc::new(std::sync::Mutex::new(Vec::new())),
    )
    .await
}

async fn build_harness(
    secret_allowed: bool,
    port: u16,
    records: Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
) -> TestHarness {
    let root = tempfile::tempdir().expect("extension temp root");
    let bin_dir = root.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin directory");
    copy_executable(
        Path::new(env!("CARGO_BIN_EXE_elasticsearch-provider")),
        &bin_dir.join("elasticsearch-provider"),
    );
    let spawn = SpawnConfig::new(bin_dir.join("elasticsearch-provider"))
        .with_program_root(root.path())
        .with_ready_timeout(Duration::from_secs(5));
    let negotiation =
        NegotiationConfig::new("0.15.2", "elasticsearch-e2e").offer_api("extension", "1.0");
    let config = ProcessRpcSessionConfig::new(spawn, negotiation)
        .with_request_timeout(Duration::from_secs(5))
        .with_shutdown_grace_ms(2_500)
        .with_label("com.navop.elasticsearch::main")
        .with_host_api(Arc::new(HostApiHandler::new(Arc::new(TestHostApi {
            secret_allowed,
        }))));
    let session = Arc::new(
        ProcessRpcSession::start(config)
            .await
            .expect("start provider"),
    );
    let cloned_session = session.clone_session();
    assert!(!cloned_session.is_closed());
    let client = TestPluginClient::new(UniversalPluginClient::new(Arc::clone(&session)), port);
    TestHarness {
        client,
        session,
        root,
        records,
        port,
        cloned_session,
    }
}

fn open_params(port: u16) -> ResourceOpenParams {
    ResourceOpenParams {
        resource_type: "elasticsearch".into(),
        config: json!({
            "url": format!("http://127.0.0.1:{port}"),
            "credential_ref": "secret://elasticsearch/api_key"
        }),
        metadata: None,
    }
}

fn assert_resource_closed(error: HostError) {
    assert!(matches!(
        error,
        HostError::Protocol(ref protocol) if protocol.code == error_codes::RESOURCE_CLOSED
    ));
}

async fn wait_for_job_success(
    client: &extension_host::UniversalPluginClient,
    job_id: &str,
) -> extension_protocol::job::JobStatusResult {
    for _ in 0..100 {
        let status = client
            .job_status(&JobStatusParams {
                job_id: job_id.to_owned(),
            })
            .await
            .expect("job status");
        if status.state == JobState::Succeeded {
            return status;
        }
        assert_eq!(JobState::Running, status.state, "job failed: {status:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job did not complete within the test deadline");
}

#[tokio::test]
async fn provider_performs_authenticated_read_only_http_operations() {
    let harness = harness(true).await;
    let opened = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open resource");
    uuid::Uuid::parse_str(&opened.resource_id).expect("opaque resource UUID");
    assert_eq!(
        Some(&json!({
            "client":"elasticsearch-rs",
            "client_version":"9.1.0-alpha.1",
            "server_major":9,
            "network":true,
            "operations":"read-only"
        })),
        opened.metadata.as_ref()
    );

    harness
        .client
        .ping_resource(&ResourcePingParams {
            resource_id: opened.resource_id.clone(),
        })
        .await
        .expect("ping");
    let listed = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: opened.resource_id.clone(),
            method: "elasticsearch/index/list".into(),
            params: Value::Null,
        })
        .await
        .expect("list");
    let ResultRef::Inline { value } = listed.result else {
        panic!("inline list")
    };
    assert_eq!(
        "orders",
        value["indices"][0]["name"].as_str().expect("name")
    );

    let fetched = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: opened.resource_id.clone(),
            method: "elasticsearch/index/get".into(),
            params: json!({"name":"orders"}),
        })
        .await
        .expect("get index");
    let ResultRef::Inline { value } = fetched.result else {
        panic!("inline index")
    };
    assert!(value.get("orders").is_some());

    let searched = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: opened.resource_id.clone(),
            method: "elasticsearch/search".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("search");
    let ResultRef::Inline { value } = searched.result else {
        panic!("inline search")
    };
    assert_eq!(
        2,
        value["raw"]["hits"]["total"]["value"]
            .as_i64()
            .expect("hits")
    );

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: opened.resource_id,
        })
        .await
        .expect("close");
    harness.session.shutdown().await;
    assert!(harness.session.is_closed());
    assert!(harness.cloned_session.is_closed());

    let records = harness.records.lock().expect("records lock");
    assert_eq!(4, records.len());
    assert!(
        records
            .iter()
            .all(|record| record.authorization.as_deref() == Some("ApiKey token-value"))
    );
    assert_eq!(
        ("GET", "/"),
        (records[0].method.as_str(), records[0].target.as_str())
    );
    assert_eq!("GET", records[1].method.as_str());
    assert!(records[1].target.starts_with("/_cat/indices?"));
    assert_eq!(
        ("GET", "/orders"),
        (records[2].method.as_str(), records[2].target.as_str())
    );
    assert_eq!(
        ("POST", "/_search"),
        (records[3].method.as_str(), records[3].target.as_str())
    );
    assert!(
        records
            .iter()
            .all(|record| !record.body.contains("token-value"))
    );
    drop(records);
    drop(harness.root);
}

#[tokio::test]
async fn multiple_resources_remain_isolated_when_one_closes() {
    let harness = harness(true).await;
    let first = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open first resource");
    let second = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open second resource");
    assert_ne!(first.resource_id, second.resource_id);
    uuid::Uuid::parse_str(&first.resource_id).expect("first resource UUID");
    uuid::Uuid::parse_str(&second.resource_id).expect("second resource UUID");

    let first_blob = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: first.resource_id.clone(),
            method: "elasticsearch/search".into(),
            params: json!({"query":"large"}),
        })
        .await
        .expect("use first resource");
    let ResultRef::Blob { id: first_blob_id } = first_blob.result else {
        panic!("first resource should own a blob")
    };
    let second_blob = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: second.resource_id.clone(),
            method: "elasticsearch/search".into(),
            params: json!({"query":"large"}),
        })
        .await
        .expect("use second resource");
    let ResultRef::Blob { id: second_blob_id } = second_blob.result else {
        panic!("second resource should own a blob")
    };
    let first_job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(first.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"delayed"}),
        })
        .await
        .expect("start first resource job");
    let second_job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(second.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("start second resource job");

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: first.resource_id.clone(),
        })
        .await
        .expect("close first resource");
    let first_ping = harness
        .client
        .ping_resource(&ResourcePingParams {
            resource_id: first.resource_id.clone(),
        })
        .await
        .expect_err("closed first resource");
    assert_resource_closed(first_ping);
    let first_invoke = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: first.resource_id.clone(),
            method: "elasticsearch/index/list".into(),
            params: Value::Null,
        })
        .await
        .expect_err("closed first resource cannot invoke");
    assert_resource_closed(first_invoke);
    let first_job_start = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(first.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect_err("closed first resource cannot start a job");
    assert_resource_closed(first_job_start);
    let first_close = harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: first.resource_id,
        })
        .await
        .expect_err("closed first resource cannot close twice");
    assert_resource_closed(first_close);
    let first_job_status = harness
        .client
        .job_status(&JobStatusParams {
            job_id: first_job.job_id,
        })
        .await
        .expect_err("first resource job released");
    assert_resource_closed(first_job_status);
    let first_blob_read = harness
        .client
        .read_blob(&BlobReadParams {
            blob_id: first_blob_id,
            max_bytes: Some(1024),
        })
        .await
        .expect_err("first resource blob released");
    assert_resource_closed(first_blob_read);

    harness
        .client
        .ping_resource(&ResourcePingParams {
            resource_id: second.resource_id.clone(),
        })
        .await
        .expect("second resource remains open");
    let second_blob_read = harness
        .client
        .read_blob(&BlobReadParams {
            blob_id: second_blob_id,
            max_bytes: Some(1024),
        })
        .await
        .expect("second resource blob remains open");
    assert_eq!(1024, second_blob_read.bytes_read);
    wait_for_job_success(&harness.client, &second_job.job_id).await;
    harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: second.resource_id.clone(),
            method: "elasticsearch/index/get".into(),
            params: json!({"name":"orders"}),
        })
        .await
        .expect("second resource remains usable");

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: second.resource_id,
        })
        .await
        .expect("close second resource");
    harness.session.shutdown().await;
}

#[tokio::test]
async fn network_permission_is_enforced_before_provider_rpc() {
    let harness = harness(true).await;
    let mut params = open_params(harness.port);
    params.config["url"] = json!(format!(
        "http://127.0.0.1:{}",
        harness.port.saturating_sub(1)
    ));
    let error = harness
        .client
        .open_resource(&params)
        .await
        .expect_err("network denied");
    let HostError::Protocol(protocol) = error else {
        panic!("protocol error expected: {error:?}")
    };
    assert_eq!(
        extension_protocol::error::error_codes::PERMISSION_DENIED,
        protocol.code
    );
    assert!(harness.records.lock().expect("records lock").is_empty());
    harness.session.shutdown().await;
}

#[tokio::test]
async fn async_search_events_use_bounded_pull_streams() {
    let harness = harness(true).await;
    let opened = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open resource");

    let stream = harness
        .client
        .open_event_stream(&EventOpenParams {
            conn_id: None,
            kind: "elasticsearch/search/events".into(),
            capacity: Some(1),
        })
        .await
        .expect("open event stream");
    let first_job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("start first streamed job");
    let status = wait_for_job_success(&harness.client, &first_job.job_id).await;

    let first = harness
        .client
        .read_event_stream(&EventReadParams {
            stream_id: stream.stream_id.clone(),
            max_events: Some(1),
            wait_ms: Some(0),
        })
        .await
        .expect("read event stream");
    assert_eq!(1, first.events.len());
    assert!(!first.closed);
    assert_eq!(0, first.dropped_count);
    let first_event = first.events[0].clone();
    assert_eq!(
        "job/completed",
        first_event["type"].as_str().expect("event type")
    );
    assert_eq!(
        first_job.job_id,
        first_event["job_id"].as_str().expect("job id")
    );
    assert_eq!(
        u64::from(u8::from(
            status.progress_percent.expect("terminal progress")
        )),
        first_event["progress_percent"]
            .as_u64()
            .expect("event progress")
    );
    assert!(matches!(first_event["result"]["result"], Value::Object(_)));
    let serialized_events = serde_json::to_string(&first).expect("serialize events");
    assert!(!serialized_events.contains("token-value"));

    let second_job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"bob"}),
        })
        .await
        .expect("start second streamed job");
    wait_for_job_success(&harness.client, &second_job.job_id).await;
    let third_job = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("start third streamed job");
    wait_for_job_success(&harness.client, &third_job.job_id).await;
    let second = harness
        .client
        .read_event_stream(&EventReadParams {
            stream_id: stream.stream_id.clone(),
            max_events: Some(1),
            wait_ms: Some(0),
        })
        .await
        .expect("read bounded event stream");
    assert_eq!(1, second.events.len());
    assert!(!second.closed);
    assert_eq!(1, second.dropped_count);
    assert_eq!(
        third_job.job_id,
        second.events[0]["job_id"]
            .as_str()
            .expect("newest buffered job id")
    );

    harness
        .client
        .close_event_stream(&EventCloseParams {
            stream_id: stream.stream_id.clone(),
        })
        .await
        .expect("close event stream");
    let closed = harness
        .client
        .read_event_stream(&EventReadParams {
            stream_id: stream.stream_id,
            max_events: Some(1),
            wait_ms: Some(0),
        })
        .await
        .expect("closed stream is terminal");
    assert!(closed.closed);
    assert!(closed.events.is_empty());

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: opened.resource_id,
        })
        .await
        .expect("close resource");
    harness.session.shutdown().await;
}

#[tokio::test]
async fn async_search_uses_managed_job_and_blob_lifecycles() {
    let harness = harness(true).await;
    let opened = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open resource");

    let small = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("start small job");
    assert_eq!(JobState::Running, small.state);
    let small_status = wait_for_job_success(&harness.client, &small.job_id).await;
    assert_eq!(
        Some(ProgressPercent::new(100).expect("valid progress")),
        small_status.progress_percent
    );
    let small_result = harness
        .client
        .job_result(&JobResultParams {
            job_id: small.job_id.clone(),
        })
        .await
        .expect("small job result");
    assert!(matches!(small_result.result, ResultRef::Inline { .. }));
    harness
        .client
        .close_job(&JobCloseParams {
            job_id: small.job_id,
        })
        .await
        .expect("close small job");

    let large = harness
        .client
        .start_job(&JobStartParams {
            resource_id: Some(opened.resource_id.clone()),
            method: "elasticsearch/search/async".into(),
            params: json!({"query":"large"}),
        })
        .await
        .expect("start large job");
    wait_for_job_success(&harness.client, &large.job_id).await;
    let large_result = harness
        .client
        .job_result(&JobResultParams {
            job_id: large.job_id.clone(),
        })
        .await
        .expect("large job result");
    let ResultRef::Blob { id: blob_id } = large_result.result else {
        panic!("large job result should use a blob");
    };
    harness
        .client
        .close_job(&JobCloseParams {
            job_id: large.job_id.clone(),
        })
        .await
        .expect("close large job");
    let error = harness
        .client
        .read_blob(&BlobReadParams {
            blob_id,
            max_bytes: Some(1024),
        })
        .await
        .expect_err("job-owned blob released");
    assert!(matches!(
        error,
        extension_host::HostError::Protocol(ref protocol)
            if protocol.code == extension_protocol::error::error_codes::RESOURCE_CLOSED
    ));

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: opened.resource_id,
        })
        .await
        .expect("close resource");
    harness.session.shutdown().await;
}

#[tokio::test]
async fn large_search_results_stream_through_bounded_blobs() {
    let harness = harness(true).await;
    let opened = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect("open resource");

    let small = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: opened.resource_id.clone(),
            method: "elasticsearch/search".into(),
            params: json!({"query":"alice"}),
        })
        .await
        .expect("small search");
    assert!(matches!(small.result, ResultRef::Inline { .. }));

    let large = harness
        .client
        .invoke_resource(&ResourceInvokeParams {
            resource_id: opened.resource_id.clone(),
            method: "elasticsearch/search".into(),
            params: json!({"query":"large"}),
        })
        .await
        .expect("large search");
    let ResultRef::Blob { id: blob_id } = large.result else {
        panic!("large search should return a blob");
    };

    let first = harness
        .client
        .read_blob(&BlobReadParams {
            blob_id: blob_id.clone(),
            max_bytes: Some(64 * 1024),
        })
        .await
        .expect("first blob chunk");
    assert_eq!(64 * 1024, first.bytes_read);
    assert!(!first.done);

    let mut read_bytes = first.bytes_read as u64;
    let mut completed = first.done;
    while !completed {
        let chunk = harness
            .client
            .read_blob(&BlobReadParams {
                blob_id: blob_id.clone(),
                max_bytes: Some(MAX_BLOB_CHUNK_BYTES),
            })
            .await
            .expect("remaining blob chunk");
        assert!(chunk.bytes_read > 0);
        assert!(chunk.bytes_read <= MAX_BLOB_CHUNK_BYTES);
        read_bytes += chunk.bytes_read as u64;
        completed = chunk.done;
    }
    assert!(read_bytes > 64 * 1024);

    harness
        .client
        .close_blob(&BlobCloseParams {
            blob_id: blob_id.clone(),
        })
        .await
        .expect("close blob");
    let closed = harness
        .client
        .read_blob(&BlobReadParams {
            blob_id,
            max_bytes: Some(1024),
        })
        .await
        .expect_err("closed blob");
    assert!(matches!(
        closed,
        extension_host::HostError::Protocol(ref error)
            if error.code == extension_protocol::error::error_codes::RESOURCE_CLOSED
    ));

    harness
        .client
        .close_resource(&ResourceCloseParams {
            resource_id: opened.resource_id,
        })
        .await
        .expect("close resource");
    harness.session.shutdown().await;

    let records = harness.records.lock().expect("records lock");
    let request_bodies = records
        .iter()
        .map(|record| record.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!request_bodies.contains("token-value"));
    drop(records);
    drop(harness.root);
}

#[tokio::test]
async fn secret_permission_is_enforced_before_lookup() {
    let harness = harness(false).await;
    let error = harness
        .client
        .open_resource(&open_params(harness.port))
        .await
        .expect_err("secret denied");
    let HostError::Protocol(protocol) = error else {
        panic!("protocol error expected: {error:?}")
    };
    assert_eq!(
        extension_protocol::error::error_codes::PERMISSION_DENIED,
        protocol.code
    );
    assert!(harness.records.lock().expect("records lock").is_empty());
    harness.session.shutdown().await;
}

#[tokio::test]
async fn provider_rejects_elasticsearch_8_servers() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _records = spawn_http_fixture_with_version(listener, "8.19.0").await;
    let harness = harness_with_port(true, port).await;

    let error = harness
        .client
        .open_resource(&open_params(port))
        .await
        .expect_err("Elasticsearch 8 must be rejected by the ES 9 provider");
    let HostError::Protocol(error) = error else {
        panic!("expected protocol error");
    };
    assert_eq!(
        extension_protocol::error::error_codes::SERVER_INCOMPATIBLE,
        error.code
    );
    harness.session.shutdown().await;
}
