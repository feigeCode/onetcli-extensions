use super::model::{ImportRecord, ImportWarning, quick_command_record, warning};

const GROUP_PREFIX: &str = "Z:\"";
const SEND_ACTION: &str = "SEND";
pub(crate) const BUTTON_BAR_KEY: &str = "Button Bar";

pub(crate) fn parse_button_bar(
    path: &str,
    text: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Vec<ImportRecord> {
    let mut group_name = fallback_group_name(path);
    let mut records = Vec::new();

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(name) = parse_group_name(line) {
            group_name = name;
            continue;
        }
        parse_button_line(path, line, &group_name, &mut records, warnings);
    }

    records
}

fn parse_button_line(
    path: &str,
    line: &str,
    group_name: &str,
    records: &mut Vec<ImportRecord>,
    warnings: &mut Vec<ImportWarning>,
) {
    let fields = parse_csv_fields(line);
    let Some(action) = fields.first().filter(|action| !action.is_empty()) else {
        return;
    };
    if !action.eq_ignore_ascii_case(SEND_ACTION) {
        warnings.push(warning(
            "securecrt_button_action_not_imported",
            format!("A non-SEND action in SecureCRT button bar {group_name} was skipped."),
        ));
        return;
    }

    let command = fields.get(1).map(String::as_str).unwrap_or_default();
    if command.is_empty() {
        return;
    }
    let name = fields
        .get(2)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("Command");
    let sort_order = i32::try_from(records.len()).unwrap_or(i32::MAX);
    let source_id = format!("{path}#{group_name}/{sort_order}");
    records.push(quick_command_record(
        source_id,
        group_name.to_string(),
        name.to_string(),
        decode_command(command),
        sort_order,
    ));
}

fn parse_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field.trim().to_string());
                field.clear();
            }
            _ => field.push(ch),
        }
    }
    fields.push(field.trim().to_string());
    fields
}

fn parse_group_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix(GROUP_PREFIX)?;
    let (name, _) = rest.split_once("\"=")?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn fallback_group_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.strip_suffix(".ini")
        .or_else(|| name.strip_suffix(".INI"))
        .unwrap_or(name)
        .to_string()
}

pub(crate) fn decode_command(value: &str) -> String {
    let chars: Vec<_> = value.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let Some((decoded, consumed)) = decode_escape(&chars[index..]) else {
            out.push(chars[index]);
            index += 1;
            continue;
        };
        out.push_str(decoded);
        index += consumed;
    }
    out
}

fn decode_escape(chars: &[char]) -> Option<(&'static str, usize)> {
    if chars.first() != Some(&'\\') {
        return None;
    }
    let (escape, consumed) = match chars {
        ['\\', '\\', escape, ..] => (*escape, 3),
        ['\\', escape, ..] => (*escape, 2),
        _ => return None,
    };
    match escape {
        'r' | 'R' => Some(("\r", consumed)),
        'p' | 'P' => Some((r"\p", consumed)),
        _ => None,
    }
}
