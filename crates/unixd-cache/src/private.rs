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

/// Opens or creates a `0600` lock file without following a symlink. A failed
/// open is [`Error::UnsafeRoot`] only when something other than a regular
/// file is at `path`; otherwise it keeps its errno.
pub(crate) fn open_lock(path: &Path) -> Result<File, Error> {
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let file = match rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR) {
        Ok(fd) => File::from(fd),
        Err(errno) => {
            return Err(match fs::symlink_metadata(path) {
                Ok(metadata) if !metadata.file_type().is_file() => Error::UnsafeRoot,
                _ => Error::io(&errno.into()),
            });
        }
    };
    let metadata = file.metadata().map_err(|error| Error::io(&error))?;
    if !metadata.file_type().is_file() || !owned(&metadata) {
        return Err(Error::UnsafeRoot);
    }
    rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR).map_err(|errno| Error::io(&errno.into()))?;
    Ok(file)
}

/// Locks the file at `path` exclusively. A holder deletes its file before
/// unlocking, so a waiter can end up holding a deleted file; it then retries
/// until `path` names the file it holds.
pub(crate) fn lock_exclusive(path: &Path) -> Result<File, Error> {
    loop {
        let file = open_lock(path)?;
        file.lock().map_err(|_| Error::Lock)?;
        let held = file.metadata().map_err(|error| Error::io(&error))?;
        match fs::symlink_metadata(path) {
            Ok(named) if named.dev() == held.dev() && named.ino() == held.ino() => {
                return Ok(file);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(&error)),
        }
    }
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
/// Opening a symlink or a socket fails with an errno that varies by platform,
/// so a failed open checks the file type before reporting an error.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let mut file = match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(errno) => {
            return match fs::symlink_metadata(path) {
                Ok(metadata) if !metadata.file_type().is_file() => Ok(None),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                _ => Err(Error::io(&errno.into())),
            };
        }
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
