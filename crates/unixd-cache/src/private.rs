//! Private directories, private lock files, and reads that never follow a
//! symlink. Every function refuses what another user owns.

use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

use rustix::fs::{Mode, OFlags};

use crate::Error;

/// Creates `root` and its parents when missing, then requires it to be a
/// private directory.
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

/// Requires `root` to be a real directory owned by this user, and makes it
/// `0700`.
pub(crate) fn validate_root(root: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(root).map_err(|_| Error::UnsafeRoot)?;
    if !metadata.file_type().is_dir() || !owned(&metadata) {
        return Err(Error::UnsafeRoot);
    }
    set_mode(root, 0o700)
}

/// Creates `path` as a `0700` directory, or requires the existing one to be a
/// real directory owned by this user.
pub(crate) fn ensure_dir(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && owned(&metadata) => set_mode(path, 0o700),
        Ok(_) => Err(Error::UnsafeRoot),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| Error::io(&error))?;
            set_mode(path, 0o700)
        }
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

/// The metadata of a regular file at `path`, or `None` when nothing is there.
/// Anything else at `path` is removed and reported as `None`.
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

/// Reads a regular file, or returns `None` when there is none.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    if regular(path)?.is_none() {
        return Ok(None);
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(&error)),
    }
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

/// Flushes a directory, so a rename inside it survives a crash.
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
