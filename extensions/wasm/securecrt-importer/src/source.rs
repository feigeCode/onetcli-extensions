#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    SessionIni,
    FolderData,
    Xml,
    ButtonBar,
}

pub fn classify_source(path: &str) -> Option<SourceKind> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower_name = name.to_ascii_lowercase();
    if lower_name.ends_with(".xml") {
        return Some(SourceKind::Xml);
    }
    if matches!(
        lower_name.as_str(),
        "__folderdata__.ini" | "__folderdata.ini"
    ) {
        return Some(SourceKind::FolderData);
    }
    if !lower_name.ends_with(".ini") || is_template(&lower_name) {
        return None;
    }
    if lower_name.starts_with("buttonbar") || is_command_manager_list(path, &lower_name) {
        Some(SourceKind::ButtonBar)
    } else {
        Some(SourceKind::SessionIni)
    }
}

pub(crate) fn normalize_workspace_path(path: &str) -> Option<String> {
    let parts = path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| matches!(*part, "." | "..")) {
        return None;
    }
    Some(parts.join("/"))
}

pub(crate) fn session_file_group_path(path: &str) -> Option<String> {
    session_relative_path(path, true)
}

pub(crate) fn session_directory_group_path(path: &str) -> Option<String> {
    session_relative_path(path, false)
}

fn session_relative_path(path: &str, exclude_leaf: bool) -> Option<String> {
    let components = path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let sessions_index = components
        .iter()
        .position(|part| part.eq_ignore_ascii_case("Sessions"))?;
    let end = components.len().saturating_sub(usize::from(exclude_leaf));
    if sessions_index + 1 >= end {
        return None;
    }
    normalize_workspace_path(&components[sessions_index + 1..end].join("/"))
}

pub(crate) fn quick_command_group_name(path: &str, declared_group: &str) -> String {
    let Some(folder) = command_manager_folder(path) else {
        return declared_group.to_string();
    };
    if declared_group.eq_ignore_ascii_case("default") {
        folder
    } else {
        format!("{folder}/{declared_group}")
    }
}

pub(crate) fn command_manager_folder(path: &str) -> Option<String> {
    let parts = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if !parts
        .last()
        .is_some_and(|name| name.eq_ignore_ascii_case("__Commands__.ini"))
    {
        return None;
    }
    let commands_index = parts
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("Commands"))?;
    let folders = parts.get(commands_index + 1..parts.len().saturating_sub(1))?;
    (!folders.is_empty()).then(|| folders.join("/"))
}

fn is_command_manager_list(path: &str, lower_name: &str) -> bool {
    lower_name == "__commands__.ini"
        && path
            .split(['/', '\\'])
            .rev()
            .skip(1)
            .any(|part| part.eq_ignore_ascii_case("Commands"))
}

fn is_template(name: &str) -> bool {
    matches!(
        name,
        "default.ini" | "default_rdp.ini" | "default_serial.ini" | "default_localshell.ini"
    )
}
