#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    SessionIni,
    Xml,
    ButtonBar,
}

pub fn classify_source(path: &str) -> Option<SourceKind> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower_name = name.to_ascii_lowercase();
    if lower_name.ends_with(".xml") {
        return Some(SourceKind::Xml);
    }
    if !lower_name.ends_with(".ini") || is_template(&lower_name) {
        return None;
    }
    if lower_name.starts_with("buttonbar") {
        Some(SourceKind::ButtonBar)
    } else {
        Some(SourceKind::SessionIni)
    }
}

fn is_template(name: &str) -> bool {
    matches!(
        name,
        "default.ini"
            | "default_rdp.ini"
            | "default_serial.ini"
            | "default_localshell.ini"
            | "__folderdata__.ini"
            | "__folderdata.ini"
    )
}
