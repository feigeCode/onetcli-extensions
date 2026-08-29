use serde::Serialize;
use std::collections::BTreeMap;

const MAX_MOBATXTERM_SESSIONS: usize = 8192;

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
pub struct ImportWarning {
    pub code: String,
    pub message: String,
}

pub fn preview_records_from_sources<'a, I>(
    sources: I,
    _include_passwords: bool,
) -> Vec<ImportRecord>
where
    I: IntoIterator<Item = (String, &'a [u8])>,
{
    let mut records = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (path, bytes) in sources {
        let Some(text) = decode_text(bytes) else {
            continue;
        };
        for record in parse_ini_sessions(&path, &text) {
            if seen.insert(record.id.clone()) {
                records.push(record);
            }
        }
        if records.len() >= MAX_MOBATXTERM_SESSIONS {
            break;
        }
    }
    records
}

pub fn is_supported_source_path(path: &str) -> bool {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file_name.eq_ignore_ascii_case("mobaxterm.ini") || has_extension(file_name, "mxtsessions")
}

fn parse_ini_sessions(path: &str, text: &str) -> Vec<ImportRecord> {
    let mut records = Vec::new();
    for (section, fields) in parse_sections(text) {
        if !is_bookmark_section(&section) {
            continue;
        }
        let group = normalize_group(fields.metadata.get("subrep"));
        for (key, value) in fields.sessions {
            if let Some(record) = session_record(path, &key, &value, group.as_deref()) {
                records.push(record);
            }
        }
    }
    records
}

#[derive(Default)]
struct SectionFields {
    sessions: Vec<(String, String)>,
    metadata: BTreeMap<String, String>,
}

fn parse_sections(text: &str) -> Vec<(String, SectionFields)> {
    let mut sections: Vec<(String, SectionFields)> = Vec::new();
    let mut current: Option<(String, SectionFields)> = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some((name.trim().to_string(), SectionFields::default()));
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if let Some((_, fields)) = current.as_mut() {
            if key.eq_ignore_ascii_case("subrep") || key.eq_ignore_ascii_case("imgnum") {
                fields
                    .metadata
                    .insert(key.to_ascii_lowercase(), value.to_string());
            } else {
                fields.sessions.push((key.to_string(), value.to_string()));
            }
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn is_bookmark_section(section: &str) -> bool {
    let section = section.trim();
    section.eq_ignore_ascii_case("bookmarks")
        || section.eq_ignore_ascii_case("bookmarks2")
        || section.eq_ignore_ascii_case("bookmark")
        || (section.to_ascii_lowercase().starts_with("bookmarks_")
            && section["bookmarks_".len()..]
                .chars()
                .all(|ch| ch.is_ascii_digit()))
}

fn normalize_group(subrep: Option<&String>) -> Option<String> {
    let normalized = subrep
        .unwrap_or(&String::new())
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn session_record(path: &str, key: &str, value: &str, group: Option<&str>) -> Option<ImportRecord> {
    let (label, key_group) = session_label_and_group(key);
    let group = key_group.as_deref().or(group);
    let mut warnings = Vec::new();

    let name = label.clone();
    let source_id = format!("{path}#{key}");
    let id = format!("mobaxterm:{}", slug(&source_id));

    let (host, port, username, password_present) =
        if let Some(fields) = parse_standard_session(value) {
            fields
        } else {
            parse_legacy_session(value)?
        };

    if host.is_empty() {
        return None;
    }

    let auth_method = if password_present {
        warnings.push(ImportWarning {
            code: "mobaxterm_encrypted_password".to_string(),
            message: "MobaXterm stores passwords with its own encryption; the stored password \
                      was not imported."
                .to_string(),
        });
        SshImportAuthMethod::Password { password: None }
    } else {
        SshImportAuthMethod::AutoPublicKey
    };

    let password_status = if password_present {
        "unsupported"
    } else {
        "missing"
    };

    Some(ImportRecord {
        id,
        importer_id: "mobaxterm".to_string(),
        source_label: "MobaXterm".to_string(),
        source_id: Some(source_id),
        kind: "ssh".to_string(),
        display_name: name.clone(),
        database: None,
        ssh: Some(SshImportRecord {
            name,
            host,
            port,
            username,
            group_path: group.map(str::to_string),
            auth_method,
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        port_forwarding: None,
        password_status: password_status.to_string(),
        warnings,
    })
}

// Standard MobaXterm session: `#109#0%HOST%PORT%USER%...`
// where the second field is the session type (0=SSH, 7=SSH gateway...).
fn parse_standard_session(value: &str) -> Option<(String, Option<u16>, String, bool)> {
    let outer = value.split('#').collect::<Vec<_>>();
    if outer.len() < 3 || !is_standard_prefix(value) {
        return None;
    }
    let fields = outer[2].split('%').collect::<Vec<_>>();
    let session_type = fields.first().copied().unwrap_or_default().trim();
    if session_type != "0" {
        // Non-SSH session types are not imported.
        return Some((String::new(), None, String::new(), false));
    }
    let host = fields
        .get(1)
        .copied()
        .unwrap_or_default()
        .trim()
        .to_string();
    let port = fields
        .get(2)
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .or(Some(22));
    let username = fields
        .get(3)
        .copied()
        .unwrap_or_default()
        .trim()
        .to_string();
    let username = if username.is_empty() || username == "<default>" {
        String::new()
    } else {
        username
    };
    let password_present = fields.get(4).is_some_and(|value| !value.trim().is_empty());
    Some((host, port, username, password_present))
}

fn is_standard_prefix(value: &str) -> bool {
    let trimmed = value.trim();
    let rest = trimmed
        .strip_prefix(";")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let rest = rest
        .strip_prefix("logout")
        .map(str::trim_start)
        .unwrap_or(rest);
    rest.starts_with('#')
        && rest[1..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .count()
            > 0
}

// Older/legacy layouts accepted by MobaXterm, e.g.
// `deploy@legacy.example.com:2222#ssh` or `user@host#ssh#22`.
fn parse_legacy_session(value: &str) -> Option<(String, Option<u16>, String, bool)> {
    let tokens = value
        .split('#')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    let ssh_token_index = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("ssh"))?;

    let mut host = String::new();
    let mut username = String::new();
    let mut port = None;

    for token in tokens.iter().take(ssh_token_index) {
        if let Some(target) = parse_target(token) {
            host = target.host;
            username = target.username.unwrap_or_default();
            port = target.port;
            break;
        }
    }
    if host.is_empty() {
        return None;
    }
    if let Some(numeric) = tokens
        .iter()
        .skip(ssh_token_index + 1)
        .find_map(|token| token.parse::<u16>().ok())
    {
        port = Some(numeric);
    }
    Some((host, port, username, false))
}

struct ParsedTarget {
    host: String,
    username: Option<String>,
    port: Option<u16>,
}

fn parse_target(value: &str) -> Option<ParsedTarget> {
    let value = value
        .trim()
        .strip_prefix("ssh://")
        .unwrap_or_else(|| value.trim());
    let value = value.split('/').next().unwrap_or(value);
    let (user_info, host_port) = value
        .rsplit_once('@')
        .map_or((None, value), |(user, host)| (Some(user), host));
    let username = user_info
        .map(|user| user.split_once(':').map_or(user, |(user, _)| user))
        .map(str::to_string)
        .filter(|user| !user.is_empty());
    let (host, port) = parse_host_port(host_port)?;
    Some(ParsedTarget {
        host,
        username,
        port,
    })
}

fn parse_host_port(value: &str) -> Option<(String, Option<u16>)> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = suffix
            .strip_prefix(':')
            .and_then(|value| value.parse().ok());
        return Some((host.to_string(), port));
    }
    if value.matches(':').count() == 1 {
        let (host, port) = value.rsplit_once(':')?;
        if let Ok(port) = port.parse::<u16>() {
            return Some((host.to_string(), Some(port)));
        }
    }
    Some((value.to_string(), None))
}

fn session_label_and_group(key: &str) -> (String, Option<String>) {
    let parts = key.replace('\\', "/");
    let parts = parts.split('/').filter(|part| !part.is_empty());
    let mut label = String::new();
    let mut group_parts = Vec::new();
    for part in parts {
        if !label.is_empty() {
            group_parts.push(label.clone());
        }
        label = part.to_string();
    }
    let group = if group_parts.is_empty() {
        None
    } else {
        Some(group_parts.join("/"))
    };
    (label, group)
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&bytes[2..], u16::from_le_bytes);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&bytes[2..], u16::from_be_bytes);
    }
    std::str::from_utf8(bytes)
        .ok()
        .map(|text| text.trim_start_matches('\u{feff}').to_string())
}

fn decode_utf16(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| from_bytes(*chunk));
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

fn has_extension(path: &str, extension: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case(extension))
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "session".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standard_session(host: &str, port: &str, user: &str, password: &str) -> String {
        format!(
            "#109#0%{host}%{port}%{user}%{password}%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10%0%0%-1#0# #-1"
        )
    }

    #[test]
    fn parses_standard_ssh_session_with_group() {
        let ini = "[Bookmarks]\nSubRep=\nImgNum=42\n\
                   root-server=#109#0%root.example.com%22%<default>%%-1%\n\n\
                   [Bookmarks_1]\nSubRep=Production\\\\Linux\nImgNum=41\n\
                   web-server=#109#0%10.0.0.20%2222%deploy%%-1%\n";
        let records = parse_ini_sessions("MobaXterm.ini", ini);

        assert_eq!(records.len(), 2);
        let first = &records[0];
        assert_eq!(first.display_name, "root-server");
        assert_eq!(first.ssh.as_ref().unwrap().host, "root.example.com");
        assert_eq!(first.ssh.as_ref().unwrap().port, Some(22));
        assert_eq!(first.ssh.as_ref().unwrap().username, "");
        assert_eq!(first.ssh.as_ref().unwrap().group_path, None);
        let second = &records[1];
        assert_eq!(
            second.ssh.as_ref().unwrap().group_path.as_deref(),
            Some("Production/Linux")
        );
        assert_eq!(second.ssh.as_ref().unwrap().port, Some(2222));
    }

    #[test]
    fn flags_encrypted_passwords_as_unsupported() {
        let ini = format!(
            "[Bookmarks]\nSubRep=\nserver={}\n",
            standard_session("10.0.0.1", "22", "root", "encrypted")
        );
        let records = parse_ini_sessions("MobaXterm.ini", &ini);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].password_status, "unsupported");
        assert_eq!(
            records[0]
                .warnings
                .iter()
                .find(|w| w.code == "mobaxterm_encrypted_password")
                .map(|w| w.code.clone()),
            Some("mobaxterm_encrypted_password".to_string())
        );
        assert!(matches!(
            records[0].ssh.as_ref().unwrap().auth_method,
            SshImportAuthMethod::Password { password: None }
        ));
    }

    #[test]
    fn skips_non_ssh_session_types() {
        let ini =
            "[Bookmarks]\nSubRep=\ntelnet=#91#4%10.0.0.5%23%\nssh=#109#0%10.0.0.6%22%root%%\n";
        let records = parse_ini_sessions("MobaXterm.ini", ini);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].ssh.as_ref().unwrap().host, "10.0.0.6");
    }

    #[test]
    fn parses_legacy_user_host_layout() {
        let ini = "[Bookmarks]\nLegacy\\\\server=deploy@legacy.example.com:2222#ssh\n";
        let records = parse_ini_sessions("MobaXterm.ini", ini);

        assert_eq!(records.len(), 1);
        let ssh = records[0].ssh.as_ref().unwrap();
        assert_eq!(ssh.host, "legacy.example.com");
        assert_eq!(ssh.port, Some(2222));
        assert_eq!(ssh.username, "deploy");
        assert_eq!(ssh.group_path.as_deref(), Some("Legacy"));
    }

    #[test]
    fn deduplicates_records_with_identical_ids() {
        let ini = "[Bookmarks]\nSubRep=\nserver=#109#0%prod.example.com%22%root%%\n";
        let records = preview_records_from_sources(
            vec![
                ("MobaXterm.ini".to_string(), ini.as_bytes()),
                ("MobaXterm.ini".to_string(), ini.as_bytes()),
            ],
            false,
        );

        assert_eq!(records.len(), 1);
    }

    #[test]
    fn recognizes_supported_paths() {
        assert!(is_supported_source_path(
            "C:/Users/x/Documents/MobaXterm/MobaXterm.ini"
        ));
        assert!(is_supported_source_path("sessions.MXTSESSIONS"));
        assert!(!is_supported_source_path("random.ini"));
    }

    #[test]
    fn session_missing_host_is_skipped() {
        let ini = "[Bookmarks]\nSubRep=\nbroken=#109#0%%22%root%%\n";
        let records = parse_ini_sessions("MobaXterm.ini", ini);
        assert!(records.is_empty());
    }
}
