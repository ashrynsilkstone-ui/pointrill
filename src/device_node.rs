use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

pub(crate) fn open(path: &Path, sysfs_dev: &Path, writable: bool) -> io::Result<File> {
    let device = fs::read_to_string(sysfs_dev)?;
    let expected = device
        .trim()
        .split_once(':')
        .and_then(|(major, minor)| Some(libc::makedev(major.parse().ok()?, minor.parse().ok()?)));
    let expected = expected
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Ungültige USB-Gerätenummer"))?;

    let file = OpenOptions::new()
        .read(true)
        .write(writable)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_char_device() || meta.rdev() != expected {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            "Die Linux-Geräteschnittstelle hat sich geändert",
        ));
    }
    Ok(file)
}
