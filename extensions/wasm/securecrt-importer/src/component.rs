wit_bindgen::generate!({
    path: "../../../wit",
    world: "connection-importer",
});

use onet::extension::{
    connection_import::{CandidateFile, DirectoryEntry, HostError},
    connection_import_host,
};

use crate::securecrt::{
    ImportWarning, PreviewResult, append_workspace_records, classify_source,
    preview_records_from_sources, session_directory_group_path,
};
use std::collections::BTreeSet;

const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_SOURCE_FILES: usize = 4096;

#[derive(Default)]
struct CollectedSources {
    files: Vec<(String, Vec<u8>)>,
    session_groups: BTreeSet<String>,
    warnings: Vec<ImportWarning>,
}

struct DirectoryCursor<'a> {
    candidate_id: &'a str,
    host_prefix: String,
    logical_prefix: String,
    depth: usize,
}

impl<'a> DirectoryCursor<'a> {
    fn root(candidate: &'a CandidateFile) -> Self {
        Self {
            candidate_id: &candidate.id,
            host_prefix: String::new(),
            logical_prefix: candidate_directory_prefix(&candidate.path),
            depth: 0,
        }
    }

    fn child(&self, name: &str) -> Option<Self> {
        Some(Self {
            candidate_id: self.candidate_id,
            host_prefix: safe_relative_path(&self.host_prefix, name)?,
            logical_prefix: safe_relative_path(&self.logical_prefix, name)?,
            depth: self.depth + 1,
        })
    }
}

struct SecureCrtImporter;

impl Guest for SecureCrtImporter {
    fn descriptor() -> String {
        serde_json::json!({
            "id": "securecrt",
            "display_name": "SecureCRT",
            "description": "Import SecureCRT SSH sessions into grouped workspaces, plus Button Bar and Command Manager commands.",
            "icon": "terminal",
            "vendor": "VanDyke",
            "supported_platforms": ["macos", "windows", "linux"],
            "output_kinds": ["ssh", "quick_command", "workspace"],
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
        let discovered_workspace_paths = discovered_workspace_paths(&preview);
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
            "warnings": preview.warnings,
            "discovered_workspace_paths": discovered_workspace_paths
        })
        .to_string()
    }

    fn preview(options: ImportOptions) -> String {
        let candidates = connection_import_host::list_candidate_files("securecrt");
        let collected = read_sources(&candidates);
        let preview = parse_sources(&collected, options.include_passwords);
        serialize_preview_records(&preview)
    }
}

pub(crate) fn serialize_preview_records(preview: &PreviewResult) -> String {
    serde_json::to_string(&preview.records).unwrap_or_else(|_| "[]".to_string())
}

fn read_sources(candidates: &[CandidateFile]) -> CollectedSources {
    let mut collected = CollectedSources::default();
    for candidate in candidates {
        if let Ok(entries) = connection_import_host::read_directory(&candidate.id) {
            let cursor = DirectoryCursor::root(candidate);
            collect_session_group(&cursor.logical_prefix, &mut collected);
            collect_directory(cursor, entries, &mut collected);
        } else {
            collect_single_file(candidate, &mut collected);
        }
    }
    collected.files.sort_by(|left, right| left.0.cmp(&right.0));
    collected
}

fn collect_directory(
    cursor: DirectoryCursor<'_>,
    mut entries: Vec<DirectoryEntry>,
    collected: &mut CollectedSources,
) {
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    for entry in entries {
        let Some(child) = cursor.child(&entry.name) else {
            push_warning(
                collected,
                "securecrt_unsafe_path_skipped",
                "An unsafe SecureCRT configuration path was skipped.",
            );
            continue;
        };
        if entry.is_dir {
            collect_session_group(&child.logical_prefix, collected);
            collect_child_directory(child, collected);
            continue;
        }
        if classify_source(&child.logical_prefix).is_some() {
            collect_child_file(child, collected);
        }
    }
}

fn collect_child_directory(cursor: DirectoryCursor<'_>, collected: &mut CollectedSources) {
    if cursor.depth > MAX_DIRECTORY_DEPTH {
        push_warning(
            collected,
            "securecrt_directory_depth_exceeded",
            "A SecureCRT configuration directory exceeded the supported nesting depth.",
        );
        return;
    }
    match connection_import_host::read_candidate_directory(cursor.candidate_id, &cursor.host_prefix)
    {
        Ok(entries) => collect_directory(cursor, entries, collected),
        Err(error) => push_host_warning(collected, "securecrt_read_directory_failed", error),
    }
}

fn collect_child_file(cursor: DirectoryCursor<'_>, collected: &mut CollectedSources) {
    if collected.files.len() >= MAX_SOURCE_FILES {
        push_warning(
            collected,
            "securecrt_source_limit_reached",
            "Additional SecureCRT configuration files were skipped.",
        );
        return;
    }
    match connection_import_host::read_candidate_child_file(
        cursor.candidate_id,
        &cursor.host_prefix,
    ) {
        Ok(bytes) => collected.files.push((cursor.logical_prefix, bytes)),
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
    let mut preview = preview_records_from_sources(
        collected
            .files
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.as_slice())),
        include_passwords,
    );
    append_workspace_records(&mut preview, collected.session_groups.iter().cloned());
    preview
}

fn discovered_workspace_paths(preview: &PreviewResult) -> Vec<String> {
    preview
        .records
        .iter()
        .filter_map(|record| match record.kind.as_str() {
            "ssh" => record.ssh.as_ref().and_then(|ssh| ssh.group_path.clone()),
            "workspace" => record
                .workspace
                .as_ref()
                .map(|workspace| workspace.path.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn candidate_directory_prefix(path: &str) -> String {
    let components = path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let config_child = components
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("Config"))
        .and_then(|index| components.get(index + 1).map(|_| index + 1));
    let root_index = config_child
        .filter(|index| is_logical_root(components[*index]))
        .or_else(|| components.iter().position(|part| is_logical_root(part)));
    let Some(root_index) = root_index else {
        return String::new();
    };
    components[root_index..].join("/")
}

fn is_logical_root(component: &str) -> bool {
    ["Sessions", "Commands", "Credentials"]
        .iter()
        .any(|root| component.eq_ignore_ascii_case(root))
}

fn collect_session_group(path: &str, collected: &mut CollectedSources) {
    if let Some(group_path) = session_directory_group_path(path) {
        collected.session_groups.insert(group_path);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_sessions_directory_keeps_host_and_logical_paths_separate() {
        let candidate = CandidateFile {
            id: "manual-file-0".to_string(),
            platform: None,
            path: "/SecureCRT/Config/Sessions".to_string(),
        };
        let group = DirectoryCursor::root(&candidate)
            .child("Production")
            .unwrap();
        let session = group.child("API.ini").unwrap();

        assert_eq!(session.host_prefix, "Production/API.ini");
        assert_eq!(session.logical_prefix, "Sessions/Production/API.ini");
    }

    #[test]
    fn scan_workspace_paths_include_directory_and_folder_data_groups() {
        let collected = CollectedSources {
            files: vec![(
                "Sessions/__FolderData__.ini".to_string(),
                b"Z:\"Folder List V2\"=00000001\n Production/Staging\n".to_vec(),
            )],
            session_groups: ["Operations".to_string()].into_iter().collect(),
            warnings: Vec::new(),
        };

        let preview = parse_sources(&collected, false);

        assert_eq!(
            discovered_workspace_paths(&preview),
            vec!["Operations", "Production", "Production/Staging"]
        );
    }
}

export!(SecureCrtImporter);
