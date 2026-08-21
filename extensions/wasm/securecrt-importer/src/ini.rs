use std::collections::BTreeMap;

use super::model::{
    CredentialMetadata, ImportRecord, ImportWarning, enrich_from_credential, field, ssh_record,
    warning,
};

pub(crate) fn parse_session(
    path: &str,
    text: &str,
    credentials: &BTreeMap<String, CredentialMetadata>,
    warnings: &mut Vec<ImportWarning>,
) -> Option<ImportRecord> {
    let mut fields = parse_fields(text);
    enrich_from_credential(&mut fields, credentials);
    let protocol = field(&fields, "Protocol Name").unwrap_or_default();
    let source_id = path.replace('\\', "/");
    let port = protocol_port(&fields, &protocol, &source_id, warnings);
    ssh_record(
        source_id,
        session_group_path(path),
        session_name(path),
        &fields,
        port,
        warnings,
    )
}

pub(crate) fn parse_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = key
            .trim()
            .strip_prefix("S:\"")
            .or_else(|| key.trim().strip_prefix("D:\""))
            .and_then(|key| key.strip_suffix('"'))
        else {
            continue;
        };
        fields.insert(key.to_ascii_lowercase(), decode_value(value));
    }
    fields
}

fn protocol_port(
    fields: &BTreeMap<String, String>,
    protocol: &str,
    source_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Option<u16> {
    let protocol = protocol.to_ascii_lowercase();
    let keys = [format!("[{protocol}] port"), "port".to_string()];
    for key in keys {
        if let Some(value) = fields.get(&key) {
            if let Some(port) = parse_hex_port(value) {
                return Some(port);
            }
            warnings.push(warning(
                "securecrt_ssh_port_invalid",
                format!(
                    "A SecureCRT SSH session had an invalid port and defaulted to 22: {source_id}"
                ),
            ));
            return Some(22);
        }
    }
    Some(22)
}

fn parse_hex_port(value: &str) -> Option<u16> {
    u32::from_str_radix(value.trim(), 16)
        .ok()
        .and_then(|port| u16::try_from(port).ok())
}

fn decode_value(value: &str) -> String {
    let value = value.trim();
    let mut parts = value.split_whitespace();
    let Some(_length) = parts.next() else {
        return String::new();
    };
    let Some(hex) = parts.next() else {
        return value.to_string();
    };
    if parts.next().is_some() || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return value.to_string();
    }
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect();
    if !bytes.len().is_multiple_of(2) {
        return value.to_string();
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .unwrap_or_else(|_| value.to_string())
        .trim_end_matches('\0')
        .to_string()
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

fn session_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.strip_suffix(".ini")
        .or_else(|| name.strip_suffix(".INI"))
        .unwrap_or(name)
        .to_string()
}

fn session_group_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let components: Vec<_> = normalized.split('/').collect();
    let sessions_index = components
        .iter()
        .position(|part| part.eq_ignore_ascii_case("Sessions"))?;
    let file_index = components.len().checked_sub(1)?;
    let group = components
        .get(sessions_index + 1..file_index)?
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    (!group.is_empty()).then_some(group)
}
