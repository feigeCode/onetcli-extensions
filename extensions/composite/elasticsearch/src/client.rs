use std::time::Duration;

use elasticsearch::{
    Elasticsearch, SearchParts,
    auth::Credentials,
    cat::CatIndicesParts,
    cluster::ClusterHealthParts,
    http::{
        response::Response,
        transport::{SingleNodeConnectionPool, TransportBuilder},
    },
    indices::{IndicesGetMappingParts, IndicesGetParts},
    params::Bytes,
};
use extension_protocol::error::{ProtocolError, error_codes};
use serde_json::{Value, json};

use crate::error::{ProviderResult, boxed_error, invalid_params};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ElasticsearchResource {
    client: Elasticsearch,
}

pub(crate) fn build_client(
    url: &str,
    credentials: Option<Credentials>,
) -> Result<ElasticsearchResource, Box<ProtocolError>> {
    let url = url
        .parse::<elasticsearch::http::Url>()
        .map_err(|error| invalid_params(format!("invalid Elasticsearch URL: {error}")))?;
    let pool = SingleNodeConnectionPool::new(url);
    let mut transport = TransportBuilder::new(pool)
        .disable_proxy()
        .timeout(HTTP_TIMEOUT)
        .enable_meta_header(true);
    if let Some(credentials) = credentials {
        transport = transport.auth(credentials);
    }
    let transport = transport
        .build()
        .map_err(|error| boxed_error(error_codes::INTERNAL_ERROR, error.to_string()))?;
    Ok(ElasticsearchResource {
        client: Elasticsearch::new(transport),
    })
}

pub(crate) async fn execute(
    resource: &ElasticsearchResource,
    method_name: &str,
    params: &Value,
) -> ProviderResult {
    let response = match method_name {
        "elasticsearch/cluster/info" => resource.client.info().send().await,
        "elasticsearch/cluster/health" => {
            resource
                .client
                .cluster()
                .health(ClusterHealthParts::None)
                .send()
                .await
        }
        "elasticsearch/index/list" => {
            resource
                .client
                .cat()
                .indices(CatIndicesParts::None)
                .format("json")
                .bytes(Bytes::B)
                .h(&["index", "health", "docs.count", "store.size"])
                .send()
                .await
        }
        "elasticsearch/index/get" => {
            let index = index_name(params)?;
            resource
                .client
                .indices()
                .get(IndicesGetParts::Index(&[index.as_str()]))
                .send()
                .await
        }
        "elasticsearch/index/mapping" => {
            let index = index_name(params)?;
            resource
                .client
                .indices()
                .get_mapping(IndicesGetMappingParts::Index(&[index.as_str()]))
                .send()
                .await
        }
        "elasticsearch/search" => execute_search(resource, params).await,
        _ => {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch method `{method_name}`"),
            ));
        }
    }
    .map_err(map_client_error)?;
    response_json(response).await
}

pub(crate) async fn validate_connection(
    resource: &ElasticsearchResource,
) -> Result<(), Box<ProtocolError>> {
    let value = execute(resource, "elasticsearch/cluster/info", &Value::Null).await?;
    let version = value
        .pointer("/version/number")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            boxed_error(
                error_codes::DATA_INVALID_ENCODING,
                "missing Elasticsearch version",
            )
        })?;
    if version.split('.').next() != Some("9") {
        return Err(boxed_error(
            error_codes::SERVER_INCOMPATIBLE,
            format!("Elasticsearch 9.x is required; server reported {version}"),
        ));
    }
    Ok(())
}

async fn execute_search(
    resource: &ElasticsearchResource,
    params: &Value,
) -> Result<Response, elasticsearch::Error> {
    let indices = params
        .get("indices")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let body = search_body(params).unwrap_or_else(|_| json!({ "query": { "match_all": {} } }));
    if indices.is_empty() {
        resource
            .client
            .search(SearchParts::None)
            .body(body)
            .send()
            .await
    } else {
        resource
            .client
            .search(SearchParts::Index(&indices))
            .body(body)
            .send()
            .await
    }
}

async fn response_json(response: Response) -> ProviderResult {
    let status = response.status_code();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTTP_BODY_BYTES as u64)
    {
        return Err(boxed_error(
            error_codes::DATA_VALUE_OUT_OF_RANGE,
            "Elasticsearch response exceeds the provider limit",
        ));
    }
    let body = response.bytes().await.map_err(map_client_error)?;
    if body.len() > MAX_HTTP_BODY_BYTES {
        return Err(boxed_error(
            error_codes::DATA_VALUE_OUT_OF_RANGE,
            "Elasticsearch response exceeds the provider limit",
        ));
    }
    let value = serde_json::from_slice::<Value>(&body).map_err(|_| {
        boxed_error(
            error_codes::DATA_INVALID_ENCODING,
            "Elasticsearch returned invalid JSON",
        )
    })?;
    if status.is_success() {
        return Ok(value);
    }
    let code = match status.as_u16() {
        401 => error_codes::AUTH_FAILED,
        403 => error_codes::PERMISSION_DENIED,
        404 => error_codes::SQL_OBJECT_NOT_FOUND,
        408 | 504 => error_codes::IO_TIMEOUT,
        _ => error_codes::IO_CONNECTION_REFUSED,
    };
    let reason = value
        .pointer("/error/reason")
        .and_then(Value::as_str)
        .unwrap_or("Elasticsearch request failed");
    Err(boxed_error(code, format!("HTTP {status}: {reason}")))
}

fn map_client_error(error: elasticsearch::Error) -> Box<ProtocolError> {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let code = if lower.contains("timed out") {
        error_codes::IO_TIMEOUT
    } else if lower.contains("certificate") {
        error_codes::TLS_CERT_INVALID
    } else {
        error_codes::IO_CONNECTION_REFUSED
    };
    boxed_error(code, format!("Elasticsearch SDK request failed: {message}"))
}

fn index_name(params: &Value) -> Result<String, Box<ProtocolError>> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid_params("index name is required"))?;
    if name.len() > 256 || name.contains('/') || name.contains('?') || name.contains('#') {
        return Err(invalid_params("invalid index name"));
    }
    Ok(name.to_owned())
}

pub(crate) fn search_body(params: &Value) -> Result<Value, Box<ProtocolError>> {
    if let Some(body) = params.get("body") {
        if !body.is_object() {
            return Err(invalid_params("search body must be a JSON object"));
        }
        return Ok(body.clone());
    }
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| invalid_params("non-empty search query is required"))?;
    if query == "*" {
        return Ok(json!({ "query": { "match_all": {} } }));
    }
    Ok(json!({
        "query": {
            "simple_query_string": {
                "query": query,
                "fields": ["*"]
            }
        }
    }))
}

pub(crate) fn normalize_result(method_name: &str, value: Value) -> Value {
    match method_name {
        "elasticsearch/index/list" => normalize_indices(value),
        "elasticsearch/index/get" => value,
        "elasticsearch/search" => normalize_search(value),
        _ => value,
    }
}

fn normalize_indices(value: Value) -> Value {
    let Some(indices) = value.as_array() else {
        return json!({ "indices": value });
    };
    let normalized: Vec<Value> = indices
        .iter()
        .map(|index| {
            json!({
                "name": index.get("index").or_else(|| index.get("name")).cloned().unwrap_or(Value::Null),
                "health": index.get("health").cloned().unwrap_or(Value::Null),
                "docs": index.get("docs.count").cloned().unwrap_or(Value::Null),
                "size_bytes": index.get("store.size").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    json!({ "indices": normalized })
}

pub(crate) fn normalize_search(value: Value) -> Value {
    json!({ "raw": value })
}
