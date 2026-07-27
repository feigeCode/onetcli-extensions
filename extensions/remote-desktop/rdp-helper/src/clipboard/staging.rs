use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;

use super::path::{ensure_not_link_or_reparse, is_link_or_reparse};

const STALE_TRANSFER_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TRANSFER_DIRECTORY_PREFIX: &str = "transfer-";

pub struct TransferDirectory {
    root_canonical: PathBuf,
    path: PathBuf,
    retained: bool,
}

impl TransferDirectory {
    pub fn create(root: &Path, transfer_id: u64) -> anyhow::Result<Self> {
        let root_canonical = prepare_root(root)?;
        let prefix = format!("{TRANSFER_DIRECTORY_PREFIX}{transfer_id:016x}-");
        let temporary = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in(root)
            .context("remote clipboard staging directory could not be created")?;
        let path = temporary.keep();
        protect_directory(&path)?;
        verify_inside_root(&path, &root_canonical)?;
        Ok(Self {
            root_canonical,
            path,
            retained: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn retain(&mut self) {
        self.retained = true;
    }
}

impl Drop for TransferDirectory {
    fn drop(&mut self) {
        if !self.retained {
            remove_transfer_directory(&self.path, &self.root_canonical);
        }
    }
}

impl fmt::Debug for TransferDirectory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferDirectory")
            .field("root", &"<redacted>")
            .field("retained", &self.retained)
            .finish()
    }
}

pub fn cleanup_stale_transfers(root: &Path) {
    let Ok(root_canonical) = prepare_root(root) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if is_stale_transfer(&entry) {
            remove_transfer_directory(&entry.path(), &root_canonical);
        }
    }
}

fn prepare_root(root: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        root.is_absolute(),
        "remote clipboard staging root must be absolute"
    );
    if !root.exists() {
        std::fs::create_dir_all(root)
            .context("remote clipboard staging root could not be created")?;
    }
    let metadata = ensure_not_link_or_reparse(root)?;
    anyhow::ensure!(
        metadata.is_dir(),
        "remote clipboard staging root is not a directory"
    );
    protect_directory(root)?;
    std::fs::canonicalize(root).context("remote clipboard staging root cannot be canonicalized")
}

fn protect_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .context("remote clipboard staging permissions could not be set")?;
    }
    Ok(())
}

fn verify_inside_root(path: &Path, root_canonical: &Path) -> anyhow::Result<()> {
    let metadata = ensure_not_link_or_reparse(path)?;
    anyhow::ensure!(
        metadata.is_dir(),
        "remote clipboard staging path is not a directory"
    );
    let canonical = std::fs::canonicalize(path)
        .context("remote clipboard staging path cannot be canonicalized")?;
    anyhow::ensure!(
        canonical.starts_with(root_canonical) && canonical != root_canonical,
        "remote clipboard staging path escapes its root"
    );
    Ok(())
}

fn is_stale_transfer(entry: &std::fs::DirEntry) -> bool {
    let name = entry.file_name();
    let Some(name) = name.to_str() else {
        return false;
    };
    if !valid_transfer_directory_name(name) {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
        return false;
    };
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return false;
    }
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= STALE_TRANSFER_TTL)
}

fn valid_transfer_directory_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(TRANSFER_DIRECTORY_PREFIX) else {
        return false;
    };
    let Some((id, random)) = rest.split_once('-') else {
        return false;
    };
    id.len() == 16
        && id.chars().all(|character| character.is_ascii_hexdigit())
        && !random.is_empty()
        && random
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn remove_transfer_directory(path: &Path, root_canonical: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return;
    }
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return;
    };
    if canonical.starts_with(root_canonical) && canonical != root_canonical {
        let _ = std::fs::remove_dir_all(path);
    }
}
