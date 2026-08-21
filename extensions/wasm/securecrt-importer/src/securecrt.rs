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
