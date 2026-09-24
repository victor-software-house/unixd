//! Private directories, private lock files, and reads that never follow a
//! symlink. Every function refuses what another user owns.

use std::fs::{self, File, Metadata, Permissions};
use std::io::{self, Read as _};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

use rustix::fs::{Mode, OFlags};

use crate::Error;

pub(crate) fn ensure_root(root: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(root) {
        Ok(_) => validate_root(root),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|error| Error::io(&error))?;
            validate_root(root)
        }
        Err(error) => Err(Error::io(&error)),
    }
}

/// Refuses a symlink or another user's directory, then sets `0700`.
pub(crate) fn validate_root(root: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(root).map_err(|_| Error::UnsafeRoot)?;
    if !metadata.file_type().is_dir() || !owned(&metadata) {
        return Err(Error::UnsafeRoot);
    }
    set_mode(root, 0o700)
}

/// Refuses a symlink or another user's directory. When another caller creates
/// the directory first, checks theirs.
pub(crate) fn ensure_dir(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && owned(&metadata) => set_mode(path, 0o700),
        Ok(_) => Err(Error::UnsafeRoot),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => set_mode(path, 0o700),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => ensure_dir(path),
            Err(error) => Err(Error::io(&error)),
        },
        Err(error) => Err(Error::io(&error)),
    }
}

/// Opens or creates a `0600` lock file without following a symlink.
pub(crate) fn open_lock(path: &Path) -> Result<File, Error> {
    let file = File::from(
        rustix::fs::open(
            path,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| Error::UnsafeRoot)?,
    );
    let metadata = file.metadata().map_err(|error| Error::io(&error))?;
    if !metadata.file_type().is_file() || !owned(&metadata) {
        return Err(Error::UnsafeRoot);
    }
    rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR).map_err(|_| Error::Lock)?;
    Ok(file)
}

/// Removes anything at `path` that is not a regular file.
pub(crate) fn regular(path: &Path) -> Result<Option<Metadata>, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => {
            remove(path)?;
            Ok(None)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(&error)),
    }
}

/// Reads a regular file without following a symlink. Anything else reads as
/// `None` and stays in place. `NONBLOCK` keeps a FIFO from blocking the open.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let mut file = match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::LOOP) => return Ok(None),
        Err(errno) => return Err(Error::io(&errno.into())),
    };
    let metadata = file.metadata().map_err(|error| Error::io(&error))?;
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| Error::io(&error))?;
    Ok(Some(bytes))
}

/// Removes whatever is at `path` without following a symlink.
pub(crate) fn remove(path: &Path) -> Result<(), Error> {
    let result = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) => Err(error),
    };
    match result {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(Error::io(&error)),
        _ => Ok(()),
    }
}

/// Makes a rename inside the directory survive a crash.
pub(crate) fn sync_dir(path: &Path) -> Result<(), Error> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| Error::io(&error))
}

fn owned(metadata: &Metadata) -> bool {
    metadata.uid() == rustix::process::geteuid().as_raw()
}

fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    fs::set_permissions(path, Permissions::from_mode(mode)).map_err(|error| Error::io(&error))
}
