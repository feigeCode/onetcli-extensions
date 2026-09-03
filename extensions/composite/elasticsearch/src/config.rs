use elasticsearch::auth::Credentials;
use extension_protocol::{conn::SecretRef, resource::ResourceOpenParams};
use serde::Deserialize;
use serde_json::{Value, from_value};
use std::collections::BTreeMap;

use crate::error::invalid_params;

pub(crate) const RESOURCE_TYPE: &str = "elasticsearch";

#[derive(Debug, Deserialize)]
struct OpenConfig {
    url: String,
    #[serde(default)]
    auth: AuthConfig,
    #[serde(default)]
    credential_ref: Option<String>,
    #[serde(default)]
    auth_type: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    credential_refs: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AuthConfig {
    #[default]
    None,
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        encoded: String,
    },
    Bearer {
        token: String,
    },
    CredentialRef {
        kind: String,
        credential_ref: String,
        #[serde(default)]
        username: Option<String>,
    },
}

pub(crate) enum PendingCredentials {
    None,
    Direct(Credentials),
    Reference {
        kind: String,
        reference: SecretRef,
        username: Option<String>,
    },
}

pub(crate) fn parse_open_params(
    params: Value,
) -> Result<(String, PendingCredentials), Box<extension_protocol::error::ProtocolError>> {
    let params: ResourceOpenParams =
        from_value(params).map_err(|error| invalid_params(error.to_string()))?;
    if params.resource_type != RESOURCE_TYPE {
        return Err(invalid_params(format!(
            "resource type must be `{RESOURCE_TYPE}`"
        )));
    }
    let config: OpenConfig = from_value(params.config)
        .map_err(|error| invalid_params(format!("invalid Elasticsearch config: {error}")))?;
    let url = config
        .url
        .trim()
        .parse::<elasticsearch::http::Url>()
        .map_err(|_| invalid_params("a valid `http` or `https` `url` is required"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_params(
            "`url` must be an HTTP(S) endpoint without path, query, or fragment",
        ));
    }

    let credentials = match config.auth {
        AuthConfig::None => match config
            .credential_ref
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            Some(reference) => PendingCredentials::Reference {
                kind: "api_key".into(),
                reference: SecretRef::new(reference.clone()),
                username: None,
            },
            None => credentials_from_fields(&config)?,
        },
        AuthConfig::Basic { username, password } => {
            if username.is_empty() || password.is_empty() {
                return Err(invalid_params(
                    "basic authentication requires username and password",
                ));
            }
            PendingCredentials::Direct(Credentials::Basic(username, password))
        }
        AuthConfig::ApiKey { encoded } => {
            if encoded.trim().is_empty() {
                return Err(invalid_params("API key must not be empty"));
            }
            PendingCredentials::Direct(Credentials::EncodedApiKey(encoded))
        }
        AuthConfig::Bearer { token } => {
            if token.trim().is_empty() {
                return Err(invalid_params("bearer token must not be empty"));
            }
            PendingCredentials::Direct(Credentials::Bearer(token))
        }
        AuthConfig::CredentialRef {
            kind,
            credential_ref,
            username,
        } => {
            if credential_ref.trim().is_empty() {
                return Err(invalid_params("credential_ref must not be empty"));
            }
            PendingCredentials::Reference {
                kind,
                reference: SecretRef::new(credential_ref),
                username,
            }
        }
    };
    Ok((url.to_string(), credentials))
}

fn credentials_from_fields(
    config: &OpenConfig,
) -> Result<PendingCredentials, Box<extension_protocol::error::ProtocolError>> {
    match config.auth_type.as_deref().unwrap_or("none") {
        "none" => Ok(PendingCredentials::None),
        "api_key" => reference(config, "api_key", None),
        "basic" => reference(config, "password", config.username.clone()),
        "bearer" => reference(config, "token", None),
        value => Err(invalid_params(format!(
            "unsupported authentication type `{value}`"
        ))),
    }
}

fn reference(
    config: &OpenConfig,
    field: &str,
    username: Option<String>,
) -> Result<PendingCredentials, Box<extension_protocol::error::ProtocolError>> {
    let value = config
        .credential_refs
        .get(field)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid_params(format!("missing credential reference for `{field}`")))?;
    Ok(PendingCredentials::Reference {
        kind: config.auth_type.clone().unwrap_or_default(),
        reference: SecretRef::new(value),
        username,
    })
}

pub(crate) fn stored_credentials(
    kind: &str,
    username: Option<&str>,
    secret: &str,
) -> Option<Credentials> {
    match kind {
        "api_key" => Some(Credentials::EncodedApiKey(secret.to_string())),
        "bearer" => Some(Credentials::Bearer(secret.to_string())),
        "basic" => Some(Credentials::Basic(
            username?.to_string(),
            secret.to_string(),
        )),
        _ => None,
    }
}
