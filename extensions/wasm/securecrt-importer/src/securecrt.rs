#[path = "button_bar.rs"]
mod button_bar;
#[path = "ini.rs"]
mod ini;
#[path = "model.rs"]
mod model;
#[path = "source.rs"]
mod source;
#[path = "text.rs"]
mod text;
#[path = "xml.rs"]
mod xml;

use std::collections::{BTreeMap, BTreeSet};

use model::CredentialMetadata;
pub use model::{
    ImportRecord, ImportWarning, PreviewResult, QuickCommandImportRecord, SshImportAuthMethod,
};
pub(crate) use source::session_directory_group_path;
pub use source::{SourceKind, classify_source};

pub fn preview_records_from_sources<'a, I>(sources: I, include_passwords: bool) -> PreviewResult
where
    I: IntoIterator<Item = (String, &'a [u8])>,
{
    // SecureCRT configuration passwords are encrypted with product-specific
    // mechanisms. They are never decrypted or returned, regardless of this
    // generic host option.
    let _ = include_passwords;
    let mut result = PreviewResult::default();
    let mut seen = BTreeSet::new();
    let mut credentials = BTreeMap::new();
    let mut decoded_sources = Vec::new();
    for (path, bytes) in sources {
        let Some(kind) = classify_source(&path) else {
            continue;
        };
        let Some(text) = text::decode_text(bytes) else {
            result.warnings.push(model::warning(
                "securecrt_source_decode_failed",
                format!("SecureCRT source could not be decoded: {path}"),
            ));
            continue;
        };
        if is_credential_path(&path) {
            let credential_fields = ini::parse_fields(&text);
            credentials.insert(
                credential_key(&path),
                CredentialMetadata {
                    username: model::field(&credential_fields, "Username"),
                    encrypted_password: model::field(&credential_fields, "Password V2").is_some()
                        || model::field(&credential_fields, "Password").is_some(),
                },
            );
            continue;
        }
        decoded_sources.push((path, text, kind));
    }

    for (path, text, kind) in decoded_sources {
        let parsed = match kind {
            SourceKind::SessionIni => {
                { ini::parse_session(&path, &text, &credentials, &mut result.warnings) }
                    .into_iter()
                    .collect()
            }
            SourceKind::FolderData => folder_data_records(&path, &text),
            SourceKind::Xml => xml::parse_export(&path, &text, &credentials, &mut result.warnings),
            SourceKind::ButtonBar => {
                button_bar::parse_button_bar(&path, &text, &mut result.warnings)
            }
        };
        for record in parsed {
            if seen.insert(record.id.clone()) {
                result.records.push(record);
            } else {
                result.warnings.push(model::warning(
                    "securecrt_duplicate_record_id",
                    format!(
                        "A duplicate SecureCRT import record was skipped: {}",
                        record.source_id.as_deref().unwrap_or(&record.id)
                    ),
                ));
            }
        }
    }
    result
}

pub(crate) fn append_workspace_records<I>(result: &mut PreviewResult, groups: I)
where
    I: IntoIterator<Item = String>,
{
    let mut seen = result
        .records
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    for path in groups {
        let Some(path) = source::normalize_workspace_path(&path) else {
            continue;
        };
        let record = model::workspace_record(format!("Sessions/{path}"), path);
        if seen.insert(record.id.clone()) {
            result.records.push(record);
        }
    }
}

fn folder_data_records(path: &str, text: &str) -> Vec<ImportRecord> {
    let parent = source::session_file_group_path(path);
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    for folder in ini::parse_folder_names(text) {
        let workspace_path = match parent.as_deref() {
            Some(parent) => source::normalize_workspace_path(&format!("{parent}/{folder}")),
            None => source::normalize_workspace_path(&folder),
        };
        let Some(workspace_path) = workspace_path else {
            continue;
        };
        for workspace_path in workspace_path_ancestors(&workspace_path) {
            if seen.insert(workspace_path.clone()) {
                records.push(model::workspace_record(
                    format!("{}#{workspace_path}", path.replace('\\', "/")),
                    workspace_path,
                ));
            }
        }
    }
    records
}

fn workspace_path_ancestors(path: &str) -> Vec<String> {
    let Some(path) = source::normalize_workspace_path(path) else {
        return Vec::new();
    };
    let mut current = String::new();
    path.split('/')
        .map(|part| {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            current.clone()
        })
        .collect()
}

fn is_credential_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized
        .split('/')
        .rev()
        .skip(1)
        .any(|part| part.eq_ignore_ascii_case("Credentials"))
}

fn credential_key(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .trim_end_matches(".ini")
        .trim_end_matches(".INI")
        .to_ascii_lowercase()
}
