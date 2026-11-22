use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_ENTRIES: usize = 4096;
pub const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
pub const EXTENSION: &str = "craftplugin";

const FILE_KIND_MASK: u32 = 0o170_000;
const REGULAR_FILE: u32 = 0o100_000;
const DIRECTORY: u32 = 0o040_000;

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: not a valid .craftplugin archive: {1}")]
    Archive(PathBuf, String),
    #[error("the package entry `{0}` has an absolute path or leaves the plugin folder")]
    UnsafePath(String),
    #[error("the package entry `{0}` is a link or special file; only plain files and folders are allowed")]
    NotPlain(String),
    #[error("the package has more than {MAX_ENTRIES} entries")]
    TooManyEntries,
    #[error("the package unpacks to more than {} MiB", MAX_UNPACKED_BYTES / (1024 * 1024))]
    TooLarge,
}

fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['\\', ':', '\0']) || name.starts_with('/') {
        return None;
    }
    let mut relative = PathBuf::new();
    for part in name.trim_end_matches('/').split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        relative.push(part);
    }
    Some(relative)
}

pub fn unpack(archive: &Path, destination: &Path) -> Result<(), PackageError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |e| PackageError::Io(path, e)
    };
    let file = File::open(archive).map_err(io(archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| PackageError::Archive(archive.to_path_buf(), e.to_string()))?;
    if zip.len() > MAX_ENTRIES {
        return Err(PackageError::TooManyEntries);
    }
    std::fs::create_dir_all(destination).map_err(io(destination))?;
    let mut written = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| PackageError::Archive(archive.to_path_buf(), e.to_string()))?;
        let name = entry.name().map_err(|e| PackageError::Archive(archive.to_path_buf(), e.to_string()))?.to_string();
        let relative = safe_relative(&name).ok_or_else(|| PackageError::UnsafePath(name.clone()))?;
        let kind = entry.unix_mode().map(|mode| mode & FILE_KIND_MASK);
        let is_dir = entry.is_dir();
        if entry.is_symlink() || !matches!(kind, None | Some(REGULAR_FILE) | Some(DIRECTORY)) || (kind == Some(DIRECTORY)) != is_dir && kind.is_some() {
            return Err(PackageError::NotPlain(name));
        }
        let target = destination.join(relative);
        if is_dir {
            std::fs::create_dir_all(&target).map_err(io(&target))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(io(&target))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer).map_err(io(&target))?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > MAX_UNPACKED_BYTES {
                return Err(PackageError::TooLarge);
            }
            out.write_all(&buffer[..read]).map_err(io(&target))?;
        }
    }
    Ok(())
}
