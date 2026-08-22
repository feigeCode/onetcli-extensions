use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PreviewResult {
    pub records: Vec<ImportRecord>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportRecord {
    pub id: String,
    pub importer_id: String,
    pub source_label: String,
    pub source_id: Option<String>,
    pub kind: String,
    pub display_name: String,
    pub database: Option<serde_json::Value>,
    pub ssh: Option<SshImportRecord>,
    pub port_forwarding: Option<serde_json::Value>,
    pub quick_command: Option<QuickCommandImportRecord>,
    pub password_status: String,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshImportRecord {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub group_path: Option<String>,
    pub auth_method: SshImportAuthMethod,
    pub init_script: Option<String>,
    pub jump_server: Option<serde_json::Value>,
    pub proxy: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshImportAuthMethod {
    Password {
        password: Option<String>,
    },
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },
    Agent,
    AutoPublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QuickCommandImportRecord {
    pub name: String,
    pub command: String,
    pub group_name: Option<String>,
    pub shortcut: Option<String>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub connection_source_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CredentialMetadata {
    pub username: Option<String>,
    pub encrypted_password: bool,
}

pub(crate) fn ssh_record(
    source_id: String,
    group_path: Option<String>,
    name: String,
    fields: &BTreeMap<String, String>,
    port: Option<u16>,
    warnings: &mut Vec<ImportWarning>,
) -> Option<ImportRecord> {
    let protocol = field(fields, "Protocol Name").unwrap_or_default();
    if !is_ssh_protocol(&protocol) {
        return None;
    }
    let Some(host) = field(fields, "Hostname") else {
        warnings.push(warning(
            "securecrt_ssh_hostname_missing",
            format!("A SecureCRT SSH session without a hostname was skipped: {source_id}"),
        ));
        return None;
    };
    let username = field(fields, "Username").unwrap_or_default();
    let encrypted_password = has_value(fields, "Password V2")
        || has_value(fields, "Password")
        || has_value(fields, "Credential Encrypted Password");
    let identity = field(fields, "Identity Filename V2")
        .or_else(|| field(fields, "Identity File"))
        .filter(|value| !value.is_empty());
    let auth_method = match (identity, encrypted_password) {
        (Some(key_path), _) => SshImportAuthMethod::PrivateKey {
            key_path,
            passphrase: None,
        },
        (None, true) => SshImportAuthMethod::Password { password: None },
        (None, false) => SshImportAuthMethod::AutoPublicKey,
    };
    let warnings = encrypted_password
        .then(|| {
            warning(
                "securecrt_encrypted_password_not_imported",
                "SecureCRT encrypted password was not imported because this importer does not decrypt configuration passwords.",
            )
        })
        .into_iter()
        .collect();
    Some(ImportRecord {
        id: record_id("ssh", &source_id),
        importer_id: "securecrt".to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: Some(source_id),
        kind: "ssh".to_string(),
        display_name: name.clone(),
        database: None,
        ssh: Some(SshImportRecord {
            name,
            host,
            port,
            username,
            group_path,
            auth_method,
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        port_forwarding: None,
        quick_command: None,
        password_status: if encrypted_password {
            "unsupported"
        } else {
            "missing"
        }
        .to_string(),
        warnings,
    })
}

pub(crate) fn quick_command_record(
    source_id: String,
    group_name: String,
    name: String,
    command: String,
    sort_order: i32,
) -> ImportRecord {
    ImportRecord {
        id: record_id("quick_command", &source_id),
        importer_id: "securecrt".to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: Some(source_id),
        kind: "quick_command".to_string(),
        display_name: name.clone(),
        database: None,
        ssh: None,
        port_forwarding: None,
        quick_command: Some(QuickCommandImportRecord {
            name,
            command,
            group_name: Some(group_name),
            shortcut: None,
            description: Some("Imported from SecureCRT quick commands".to_string()),
            sort_order,
            connection_source_id: None,
        }),
        password_status: "missing".to_string(),
        warnings: Vec::new(),
    }
}

pub(crate) fn field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields
        .get(&key.to_ascii_lowercase())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn enrich_from_credential(
    fields: &mut BTreeMap<String, String>,
    credentials: &BTreeMap<String, CredentialMetadata>,
) {
    let Some(title) = field(fields, "Credential Title") else {
        return;
    };
    let key = title.to_ascii_lowercase();
    if let Some(credential) = credentials.get(&key) {
        if field(fields, "Username").is_none() {
            if let Some(username) = credential
                .username
                .as_ref()
                .filter(|username| !username.is_empty())
            {
                fields.insert("username".to_string(), username.clone());
            }
        }
        if credential.encrypted_password {
            fields.insert(
                "credential encrypted password".to_string(),
                "present".to_string(),
            );
        }
    }
}

pub(crate) fn warning(code: impl Into<String>, message: impl Into<String>) -> ImportWarning {
    ImportWarning {
        code: code.into(),
        message: message.into(),
    }
}

fn has_value(fields: &BTreeMap<String, String>, key: &str) -> bool {
    field(fields, key).is_some()
}

fn is_ssh_protocol(protocol: &str) -> bool {
    matches!(
        protocol.trim().to_ascii_lowercase().as_str(),
        "ssh1" | "ssh2" | "ssh"
    )
}

pub(crate) fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "record".to_string()
    } else {
        out
    }
}

fn record_id(kind: &str, source_id: &str) -> String {
    let identity = format!("{kind}\0{source_id}");
    format!(
        "securecrt:{}:{:016x}",
        slug(source_id),
        stable_hash(&identity)
    )
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
