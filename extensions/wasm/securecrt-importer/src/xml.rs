use std::collections::BTreeMap;

use quick_xml::{
    Reader,
    escape::unescape,
    events::{BytesStart, Event},
};

use super::button_bar::{BUTTON_BAR_KEY, decode_command};
use super::model::{
    CredentialMetadata, ImportRecord, ImportWarning, enrich_from_credential, quick_command_record,
    ssh_record, warning,
};

const MAX_XML_DEPTH: usize = 64;
const SESSIONS_KEY: &str = "Sessions";
const SEND_ACTION: &str = "SEND";
const COMMAND_KEY: &str = "Command";
const FUNCTION_KEY: &str = "Function";
const BUTTON_NAME_KEY: &str = "Name";
const BUTTON_BARS_KEY: &str = "Button Bars";

#[derive(Default)]
struct KeyFrame {
    name: String,
    fields: BTreeMap<String, String>,
    field_names: BTreeMap<String, String>,
    field_order: Vec<String>,
    child_buttons: Vec<KeyFrame>,
}

pub(crate) fn parse_export(
    path: &str,
    text: &str,
    credentials: &BTreeMap<String, CredentialMetadata>,
    warnings: &mut Vec<ImportWarning>,
) -> Vec<ImportRecord> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut state = XmlState::default();
    let mut parse_failed = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => state.start(&reader, &event),
            Ok(Event::Empty(event)) => state.empty(&reader, &event),
            Ok(Event::Text(text)) => state.text(&text),
            Ok(Event::End(event)) => state.end(path, event.name().as_ref(), credentials, warnings),
            Ok(Event::Eof) => {
                if !state.stack.is_empty()
                    || state.current_field.is_some()
                    || state.ignored_depth > 0
                {
                    parse_failed = true;
                    warnings.push(warning(
                        "securecrt_xml_parse_failed",
                        format!("SecureCRT XML export could not be parsed: {path}"),
                    ));
                }
                break;
            }
            Err(_) => {
                parse_failed = true;
                warnings.push(warning(
                    "securecrt_xml_parse_failed",
                    format!("SecureCRT XML export could not be parsed: {path}"),
                ));
                break;
            }
            _ => {}
        }
    }

    if let Some(depth_warning) = state.depth_warning.take() {
        warnings.push(depth_warning);
    }
    if parse_failed {
        Vec::new()
    } else {
        state.records
    }
}

#[derive(Default)]
struct XmlState {
    stack: Vec<KeyFrame>,
    current_field: Option<String>,
    records: Vec<ImportRecord>,
    depth_exceeded: bool,
    ignored_depth: usize,
    button_count: usize,
    depth_warning: Option<ImportWarning>,
}

impl XmlState {
    fn start(&mut self, reader: &Reader<&[u8]>, event: &BytesStart<'_>) {
        if self.ignored_depth > 0 {
            if tag_name(event.name().as_ref()) == "key" {
                self.ignored_depth += 1;
            }
            return;
        }
        match tag_name(event.name().as_ref()).as_str() {
            "key" => self.push_key(reader, event),
            "string" | "dword" => self.current_field = attribute(reader, event, "name"),
            _ => {}
        }
    }

    fn empty(&mut self, reader: &Reader<&[u8]>, event: &BytesStart<'_>) {
        let tag = tag_name(event.name().as_ref());
        if matches!(tag.as_str(), "string" | "dword") {
            if let Some(name) = attribute(reader, event, "name") {
                self.insert_field(name, String::new());
            }
        }
    }

    fn text(&mut self, text: &quick_xml::events::BytesText<'_>) {
        let Some(name) = self.current_field.clone() else {
            return;
        };
        let Ok(decoded) = text.decode() else {
            return;
        };
        let value = unescape(&decoded)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| decoded.into_owned());
        self.insert_field(name, value.trim().to_string());
    }

    fn end(
        &mut self,
        path: &str,
        raw_tag: &[u8],
        credentials: &BTreeMap<String, CredentialMetadata>,
        warnings: &mut Vec<ImportWarning>,
    ) {
        if self.ignored_depth > 0 {
            if tag_name(raw_tag) == "key" {
                self.ignored_depth -= 1;
            }
            return;
        }
        match tag_name(raw_tag).as_str() {
            "string" | "dword" => self.current_field = None,
            "key" => self.finish_key(path, credentials, warnings),
            _ => {}
        }
    }

    fn push_key(&mut self, reader: &Reader<&[u8]>, event: &BytesStart<'_>) {
        if self.stack.len() >= MAX_XML_DEPTH {
            self.ignored_depth = 1;
            if !self.depth_exceeded {
                self.depth_warning = Some(warning(
                    "securecrt_xml_depth_exceeded",
                    format!(
                        "SecureCRT XML export exceeded the supported nesting depth: {MAX_XML_DEPTH}"
                    ),
                ));
                self.depth_exceeded = true;
            }
            return;
        }
        self.stack.push(KeyFrame {
            name: attribute(reader, event, "name").unwrap_or_default(),
            fields: BTreeMap::new(),
            field_names: BTreeMap::new(),
            field_order: Vec::new(),
            child_buttons: Vec::new(),
        });
    }

    fn insert_field(&mut self, name: String, value: String) {
        if let Some(frame) = self.stack.last_mut() {
            let normalized = name.to_ascii_lowercase();
            if !frame.field_names.contains_key(&normalized) {
                frame.field_order.push(normalized.clone());
            }
            frame.field_names.insert(normalized.clone(), name.clone());
            frame.fields.insert(normalized, value);
        }
    }

    fn finish_key(
        &mut self,
        path: &str,
        credentials: &BTreeMap<String, CredentialMetadata>,
        warnings: &mut Vec<ImportWarning>,
    ) {
        let Some(mut frame) = self.stack.pop() else {
            return;
        };
        let current_name = frame.name.clone();
        let names: Vec<_> = self
            .stack
            .iter()
            .map(|frame| frame.name.clone())
            .chain(std::iter::once(current_name))
            .collect();
        let Some(session_index) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(SESSIONS_KEY))
        else {
            if let Some(bar_index) = button_bar_index(&names) {
                if names.len() > bar_index + 2 {
                    if let Some(parent) = self.stack.last_mut() {
                        parent.child_buttons.push(frame);
                    }
                    return;
                }
            }
            self.finish_button_bar(path, frame, names, warnings);
            return;
        };
        if names.len() <= session_index + 1 {
            return;
        }
        enrich_from_credential(&mut frame.fields, credentials);
        let source_id = format!("{path}#{}", names[session_index..].join("/"));
        let group_path = names
            .get(session_index + 1..names.len() - 1)
            .map(|parts| parts.join("/"))
            .filter(|group| !group.is_empty());
        let port = xml_port(&frame.fields, &source_id, warnings);
        if let Some(record) = ssh_record(
            source_id,
            group_path,
            frame.name,
            &frame.fields,
            port,
            warnings,
        ) {
            self.records.push(record);
        }
    }

    fn finish_button_bar(
        &mut self,
        path: &str,
        frame: KeyFrame,
        names: Vec<String>,
        warnings: &mut Vec<ImportWarning>,
    ) {
        let Some(bar_index) = button_bar_index(&names) else {
            return;
        };
        if names.len() <= bar_index + 1 {
            return;
        }

        let group_name = names[bar_index + 1].to_string();
        let mut key_occurrences = BTreeMap::<String, usize>::new();
        for button in &frame.child_buttons {
            let button_key = button.name.clone();
            let occurrence = key_occurrences
                .entry(button_key.to_ascii_lowercase())
                .and_modify(|count| *count += 1)
                .or_insert(1);
            let source_key = if *occurrence == 1 {
                button_key.clone()
            } else {
                format!("{button_key}~{occurrence}")
            };
            let label =
                field(&button.fields, BUTTON_NAME_KEY).unwrap_or_else(|| button_key.clone());
            let command = field(&button.fields, COMMAND_KEY);
            let action = field(&button.fields, FUNCTION_KEY).unwrap_or_default();
            if !action.eq_ignore_ascii_case(SEND_ACTION) {
                warnings.push(warning(
                    "securecrt_button_action_not_imported",
                    format!("A non-SEND action in SecureCRT button bar {group_name} was skipped."),
                ));
                continue;
            }
            if let Some(command) = command.filter(|command| !command.is_empty()) {
                self.push_quick_command(path, &group_name, &source_key, label, command);
            }
        }

        let button_keys = ordered_names(&frame.field_order, BUTTON_NAME_KEY);
        for button_key in button_keys {
            let prefix = button_key.to_ascii_lowercase();
            let label = field(&frame.fields, &format!("{prefix}/{BUTTON_NAME_KEY}"))
                .unwrap_or_else(|| button_key.clone());
            let command = field(&frame.fields, &format!("{prefix}/{COMMAND_KEY}"));
            let action =
                field(&frame.fields, &format!("{prefix}/{FUNCTION_KEY}")).unwrap_or_default();
            if !action.eq_ignore_ascii_case(SEND_ACTION) {
                warnings.push(warning(
                    "securecrt_button_action_not_imported",
                    format!("A non-SEND action in SecureCRT button bar {group_name} was skipped."),
                ));
                continue;
            }
            if let Some(command) = command.filter(|command| !command.is_empty()) {
                self.push_quick_command(path, &group_name, &button_key, label, command);
            }
        }
    }

    fn push_quick_command(
        &mut self,
        path: &str,
        group_name: &str,
        button_key: &str,
        button_name: String,
        command: String,
    ) {
        let sort_order = i32::try_from(self.button_count).unwrap_or(i32::MAX);
        let source_id = format!("{path}#Button Bar/{group_name}/{button_key}");
        self.records.push(quick_command_record(
            source_id,
            group_name.to_string(),
            button_name,
            decode_command(&command),
            sort_order,
        ));
        self.button_count += 1;
    }
}

fn button_bar_index(names: &[String]) -> Option<usize> {
    names.iter().position(|name| is_button_bar_key(name))
}

fn is_button_bar_key(name: &str) -> bool {
    name.eq_ignore_ascii_case(BUTTON_BAR_KEY) || name.eq_ignore_ascii_case(BUTTON_BARS_KEY)
}

fn ordered_names(field_order: &[String], leaf: &str) -> Vec<String> {
    let suffix = format!("/{leaf}").to_ascii_lowercase();
    let mut names = Vec::new();
    for key in field_order
        .iter()
        .filter(|key| key.ends_with(&suffix))
        .filter_map(|key| key.strip_suffix(&suffix))
        .map(|prefix| prefix.rsplit('/').next().unwrap_or(prefix).to_string())
    {
        if !names
            .iter()
            .any(|name: &String| name.eq_ignore_ascii_case(&key))
        {
            names.push(key);
        }
    }
    names
}

fn xml_port(
    fields: &BTreeMap<String, String>,
    source_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Option<u16> {
    for key in ["[ssh2] port", "[ssh1] port", "[ssh] port", "port"] {
        if let Some(value) = fields.get(key) {
            if let Ok(port) = value.trim().parse::<u16>() {
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

fn field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields
        .get(&key.to_ascii_lowercase())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn attribute(reader: &Reader<&[u8]>, event: &BytesStart<'_>, key: &str) -> Option<String> {
    event.attributes().flatten().find_map(|attribute| {
        (tag_name(attribute.key.as_ref()) == key)
            .then(|| attribute.decode_and_unescape_value(reader.decoder()).ok())
            .flatten()
            .map(|value| value.into_owned())
    })
}

fn tag_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}
