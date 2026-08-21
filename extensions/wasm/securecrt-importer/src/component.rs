wit_bindgen::generate!({
    path: "../../../wit",
    world: "connection-importer",
});

use onet::extension::{
    connection_import::{CandidateFile, DirectoryEntry, HostError},
    connection_import_host,
};

use crate::securecrt::{
    ImportWarning, PreviewResult, classify_source, preview_records_from_sources,
};

const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_SOURCE_FILES: usize = 4096;

#[derive(Default)]
struct CollectedSources {
    files: Vec<(String, Vec<u8>)>,
    warnings: Vec<ImportWarning>,
}

struct SecureCrtImporter;

impl Guest for SecureCrtImporter {
    fn descriptor() -> String {
        serde_json::json!({
            "id": "securecrt",
            "display_name": "SecureCRT",
            "description": "Import SecureCRT SSH sessions and SEND button-bar commands.",
            "icon": "terminal",
            "vendor": "VanDyke",
            "supported_platforms": ["macos", "windows", "linux"],
            "output_kinds": ["ssh", "quick_command"],
            "capabilities": {
                "supports_scan": true,
                "supports_password_import": false,
                "supports_manual_file_pick": true,
                "supports_incremental_preview": false
            }
        })
        .to_string()
    }

    fn scan() -> String {
        let candidates = connection_import_host::list_candidate_files("securecrt");
        let collected = read_sources(&candidates);
        let mut preview = parse_sources(&collected, false);
        preview.warnings.extend(collected.warnings);
        let availability = if preview.records.is_empty() {
            serde_json::json!("no_data")
        } else {
            serde_json::json!({ "available": { "estimated_count": preview.records.len() } })
        };
        serde_json::json!({
            "importer_id": "securecrt",
            "availability": availability,
            "discovered_files": candidates.into_iter().map(|candidate| serde_json::json!({
                "candidate_id": candidate.id,
                "display_path": candidate.path
            })).collect::<Vec<_>>(),
            "warnings": preview.warnings
        })
        .to_string()
    }

    fn preview(options: ImportOptions) -> String {
        let candidates = connection_import_host::list_candidate_files("securecrt");
        let collected = read_sources(&candidates);
        let mut preview = parse_sources(&collected, options.include_passwords);
        preview.warnings.extend(collected.warnings);
        serde_json::to_string(&preview).unwrap_or_else(|_| {
            r#"{"records":[],"warnings":[{"code":"securecrt_preview_serialize_failed","message":"SecureCRT preview could not be serialized."}]}"#.to_string()
        })
    }
}

fn read_sources(candidates: &[CandidateFile]) -> CollectedSources {
    let mut collected = CollectedSources::default();
    for candidate in candidates {
        if let Ok(entries) = connection_import_host::read_directory(&candidate.id) {
            collect_directory(&candidate.id, "", entries, 0, &mut collected);
        } else {
            collect_single_file(candidate, &mut collected);
        }
    }
    collected.files.sort_by(|left, right| left.0.cmp(&right.0));
    collected
}

fn collect_directory(
    candidate_id: &str,
    prefix: &str,
    mut entries: Vec<DirectoryEntry>,
    depth: usize,
    collected: &mut CollectedSources,
) {
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    for entry in entries {
        let Some(relative) = safe_relative_path(prefix, &entry.name) else {
            push_warning(
                collected,
                "securecrt_unsafe_path_skipped",
                "An unsafe SecureCRT configuration path was skipped.",
            );
            continue;
        };
        if entry.is_dir {
            collect_child_directory(candidate_id, &relative, depth, collected);
            continue;
        }
        if classify_source(&relative).is_some() {
            collect_child_file(candidate_id, relative, collected);
        }
    }
}

fn collect_child_directory(
    candidate_id: &str,
    relative: &str,
    depth: usize,
    collected: &mut CollectedSources,
) {
    if depth >= MAX_DIRECTORY_DEPTH {
        push_warning(
            collected,
            "securecrt_directory_depth_exceeded",
            "A SecureCRT configuration directory exceeded the supported nesting depth.",
        );
        return;
    }
    match connection_import_host::read_candidate_directory(candidate_id, relative) {
        Ok(entries) => collect_directory(candidate_id, relative, entries, depth + 1, collected),
        Err(error) => push_host_warning(collected, "securecrt_read_directory_failed", error),
    }
}

fn collect_child_file(candidate_id: &str, relative: String, collected: &mut CollectedSources) {
    if collected.files.len() >= MAX_SOURCE_FILES {
        push_warning(
            collected,
            "securecrt_source_limit_reached",
            "Additional SecureCRT configuration files were skipped.",
        );
        return;
    }
    match connection_import_host::read_candidate_child_file(candidate_id, &relative) {
        Ok(bytes) => collected.files.push((relative, bytes)),
        Err(error) => push_host_warning(collected, "securecrt_read_file_failed", error),
    }
}

fn collect_single_file(candidate: &CandidateFile, collected: &mut CollectedSources) {
    if classify_source(&candidate.path).is_none() {
        push_warning(
            collected,
            "securecrt_read_directory_failed",
            "A SecureCRT configuration directory could not be read.",
        );
        return;
    }
    match connection_import_host::read_file(&candidate.id) {
        Ok(bytes) => collected.files.push((candidate.path.clone(), bytes)),
        Err(error) => push_host_warning(collected, "securecrt_read_file_failed", error),
    }
}

fn parse_sources(collected: &CollectedSources, include_passwords: bool) -> PreviewResult {
    preview_records_from_sources(
        collected
            .files
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.as_slice())),
        include_passwords,
    )
}

fn safe_relative_path(prefix: &str, name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    })
}

fn push_host_warning(collected: &mut CollectedSources, code: &str, error: HostError) {
    push_warning(
        collected,
        code,
        format!("SecureCRT configuration access failed ({}).", error.code),
    );
}

fn push_warning(
    collected: &mut CollectedSources,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    collected.warnings.push(ImportWarning {
        code: code.into(),
        message: message.into(),
    });
}

export!(SecureCrtImporter);
