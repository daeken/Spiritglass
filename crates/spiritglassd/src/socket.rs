use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tokio::net::UnixListener;

pub struct SocketGuard {
    path: PathBuf,
    _lock: File,
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn bind(path: &Path) -> Result<(UnixListener, SocketGuard)> {
    let parent = path
        .parent()
        .context("socket must have a parent directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let metadata = fs::symlink_metadata(parent)?;
    ensure!(
        metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0,
        "socket directory must be private (mode 700): {}",
        parent.display()
    );
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(PathBuf::from(lock_path))?;
    lock.try_lock_exclusive()
        .context("another supervisor already owns this socket")?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.file_type().is_socket(),
            "refusing to replace a non-socket file"
        );
        ensure!(
            std::os::unix::net::UnixStream::connect(path).is_err(),
            "socket is already listening"
        );
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind {} (Unix socket paths must be short)", path.display()))?;
    let guard = SocketGuard {
        path: path.to_owned(),
        _lock: lock,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok((listener, guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exclusive_socket_cleans_up_and_has_private_permissions() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("control.sock");
        let (listener, guard) = bind(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(bind(&path).is_err());
        drop(listener);
        drop(guard);
        assert!(!path.exists());
        assert!(bind(&path).is_ok());
    }
    #[tokio::test]
    async fn preserves_non_socket_files_and_rejects_public_directories() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("control.sock");
        fs::write(&path, "keep me").unwrap();
        assert!(bind(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
        fs::remove_file(&path).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(bind(&path).is_err());
    }
}
