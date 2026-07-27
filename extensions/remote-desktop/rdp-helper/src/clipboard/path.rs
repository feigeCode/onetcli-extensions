use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ironrdp::cliprdr::is_windows_device_name;

pub const MAX_CLIPBOARD_ENTRY_COUNT: usize = 10_000;
pub const MAX_CLIPBOARD_DIRECTORY_DEPTH: usize = 32;
pub const MAX_SINGLE_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_TOTAL_TRANSFER_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_WIRE_PATH_UTF16_UNITS: usize = 259;

const WINDOWS_INVALID_CHARACTERS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativeClipboardPath {
    components: Vec<String>,
}

impl RelativeClipboardPath {
    pub fn from_local_components(components: Vec<String>) -> anyhow::Result<Self> {
        validate_components(&components)?;
        let path = Self { components };
        path.validate_wire_length()?;
        Ok(path)
    }

    pub fn from_wire(name: &str, parent: Option<&str>) -> anyhow::Result<Self> {
        let mut components = match parent {
            Some(parent) => split_wire_parent(parent)?,
            None => Vec::new(),
        };
        validate_component(name)?;
        components.push(name.to_string());
        Self::from_local_components(components)
    }

    pub fn name(&self) -> &str {
        self.components.last().map(String::as_str).unwrap_or("")
    }

    pub fn parent_wire_path(&self) -> Option<String> {
        let parent = self
            .components
            .get(..self.components.len().saturating_sub(1))?;
        (!parent.is_empty()).then(|| parent.join("\\"))
    }

    pub fn as_path_buf(&self) -> PathBuf {
        self.components.iter().collect()
    }

    pub fn normalized_key(&self) -> String {
        self.components
            .iter()
            .map(|component| component.to_lowercase())
            .collect::<Vec<_>>()
            .join("\\")
    }

    pub fn parent_key(&self) -> Option<String> {
        let parent = self
            .components
            .get(..self.components.len().saturating_sub(1))?;
        (!parent.is_empty()).then(|| {
            parent
                .iter()
                .map(|component| component.to_lowercase())
                .collect::<Vec<_>>()
                .join("\\")
        })
    }

    pub fn top_level_key(&self) -> String {
        self.components
            .first()
            .map(|component| component.to_lowercase())
            .unwrap_or_default()
    }

    pub fn top_level_name(&self) -> &str {
        self.components.first().map(String::as_str).unwrap_or("")
    }

    pub fn child(&self, name: String) -> anyhow::Result<Self> {
        let mut components = self.components.clone();
        components.push(name);
        Self::from_local_components(components)
    }

    fn validate_wire_length(&self) -> anyhow::Result<()> {
        let wire_path = self.components.join("\\");
        anyhow::ensure!(
            wire_path.encode_utf16().count() <= MAX_WIRE_PATH_UTF16_UNITS,
            "clipboard path exceeds the RDP wire limit"
        );
        Ok(())
    }
}

pub fn validate_top_level_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    let mut names = std::collections::HashSet::new();
    for path in paths {
        anyhow::ensure!(path.is_absolute(), "clipboard paths must be absolute");
        let name = utf8_file_name(path)?;
        validate_component(name)?;
        anyhow::ensure!(
            names.insert(name.to_lowercase()),
            "clipboard paths contain duplicate top-level names"
        );
    }
    Ok(())
}

pub fn utf8_file_name(path: &Path) -> anyhow::Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .context("clipboard path has no UTF-8 file name")
}

pub fn ensure_not_link_or_reparse(path: &Path) -> anyhow::Result<std::fs::Metadata> {
    let metadata =
        std::fs::symlink_metadata(path).context("clipboard path metadata is unavailable")?;
    anyhow::ensure!(
        !is_link_or_reparse(&metadata),
        "clipboard path cannot be a link or reparse point"
    );
    Ok(metadata)
}

pub fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

fn split_wire_parent(parent: &str) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(!parent.is_empty(), "clipboard parent path is empty");
    anyhow::ensure!(
        !parent.contains('/'),
        "clipboard parent path contains an invalid separator"
    );
    parent
        .split('\\')
        .map(|component| {
            validate_component(component)?;
            Ok(component.to_string())
        })
        .collect()
}

fn validate_components(components: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(!components.is_empty(), "clipboard path is empty");
    anyhow::ensure!(
        components.len() <= MAX_CLIPBOARD_DIRECTORY_DEPTH,
        "clipboard path exceeds the directory depth limit"
    );
    for component in components {
        validate_component(component)?;
    }
    Ok(())
}

fn validate_component(component: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !component.is_empty() && component != "." && component != "..",
        "clipboard path contains an invalid component"
    );
    anyhow::ensure!(
        !component.ends_with(' ') && !component.ends_with('.'),
        "clipboard path has an unsafe trailing character"
    );
    anyhow::ensure!(
        !component.chars().any(char::is_control),
        "clipboard path contains a control character"
    );
    anyhow::ensure!(
        !component
            .chars()
            .any(|character| WINDOWS_INVALID_CHARACTERS.contains(&character)),
        "clipboard path contains a Windows-invalid character"
    );
    anyhow::ensure!(
        !is_windows_device_name(component),
        "clipboard path uses a reserved Windows device name"
    );
    Ok(())
}
