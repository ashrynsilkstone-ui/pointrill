use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

pub(crate) const MAX_JSON: u64 = 128 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn config_home() -> Result<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .context("Kein Benutzer-Konfigurationsordner verfügbar")
}

pub(crate) fn config_dir() -> Result<PathBuf> {
    Ok(config_home()?.join("Pointrill"))
}

pub(crate) fn read_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    // The nonblocking open lets us reject pipes without waiting for a writer.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Keine reguläre Datei",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei ist zu groß",
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_file(path, MAX_JSON)
        .with_context(|| format!("Kann {} nicht lesen", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Ungültige JSON-Datei {}", path.display()))
}

pub(crate) fn write_json<T: Serialize>(path: &Path, data: &T, replace: bool) -> Result<()> {
    write_file(path, replace, |file| {
        serde_json::to_writer_pretty(&mut *file, data)?;
        file.write_all(b"\n")?;
        Ok(())
    })
}

pub(crate) fn write_file(
    path: &Path,
    replace: bool,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let result = (|| -> Result<()> {
        let parent = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if replace {
            fs::create_dir_all(parent)?;
        }
        let temp = parent.join(format!(
            ".pointrill-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let result = (|| {
            write(&mut file)?;
            file.sync_all()?;
            if replace {
                Ok(fs::rename(&temp, path)?)
            } else {
                fs::hard_link(&temp, path)
                    .context("Exportdatei existiert bereits oder kann nicht angelegt werden")
            }
        })();
        if result.is_err() || !replace {
            let _ = fs::remove_file(temp);
        }
        result
    })();
    result.with_context(|| format!("Kann {} nicht speichern", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reads_reject_special_files() {
        let error = read_file(Path::new("/dev/null"), MAX_JSON).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
