// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;

/// Process-lifetime ownership handle for one persistent storage directory.
///
#[derive(Debug)]
pub struct DataDirectoryOwnership {
    root_path: PathBuf,
    lock_path: PathBuf,
    lock_file: File,
    recovered_stale_owner: bool,
}

#[derive(Debug, Error)]
/// Exclusive persistent-directory ownership failures.
pub enum DataDirectoryOwnershipError {
    /// Another live process already holds the directory lock.
    #[error("storage directory is already owned by another process{owner}")]
    Conflict { owner: String },
    /// The ownership file or operating-system lock could not be used.
    #[error("storage ownership mechanism failed: {reason}")]
    Unavailable { reason: String },
}

impl DataDirectoryOwnership {
    /// Acquire exclusive ownership for `root_path`.
    ///
    pub fn acquire(root_path: &Path) -> Result<Self, DataDirectoryOwnershipError> {
        let parent =
            root_path
                .parent()
                .ok_or_else(|| DataDirectoryOwnershipError::Unavailable {
                    reason: format!(
                        "storage directory {} has no parent directory",
                        root_path.display()
                    ),
                })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            DataDirectoryOwnershipError::Unavailable {
                reason: format!(
                    "failed to create storage parent {}: {error}",
                    parent.display()
                ),
            }
        })?;
        let canonical_parent =
            parent
                .canonicalize()
                .map_err(|error| DataDirectoryOwnershipError::Unavailable {
                    reason: format!(
                        "failed to resolve storage parent {}: {error}",
                        parent.display()
                    ),
                })?;
        let root_name = root_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| DataDirectoryOwnershipError::Unavailable {
                reason: format!(
                    "storage directory {} has no usable directory name",
                    root_path.display()
                ),
            })?;
        let lock_path = canonical_parent.join(format!(".{root_name}.corrobore.lock"));
        let mut lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| DataDirectoryOwnershipError::Unavailable {
                reason: format!(
                    "failed to open ownership file {}: {error}",
                    lock_path.display()
                ),
            })?;

        if let Err(error) = lock_file.try_lock() {
            return match error {
                std::fs::TryLockError::WouldBlock => {
                    let owner = read_owner_metadata(&mut lock_file);
                    Err(DataDirectoryOwnershipError::Conflict { owner })
                }
                std::fs::TryLockError::Error(error) => {
                    Err(DataDirectoryOwnershipError::Unavailable {
                        reason: format!(
                            "failed to lock ownership file {}: {error}",
                            lock_path.display()
                        ),
                    })
                }
            };
        }

        let recovered_stale_owner = !read_owner_metadata(&mut lock_file).is_empty();
        if let Err(error) = publish_owner_metadata(&mut lock_file) {
            let _ = lock_file.unlock();
            return Err(DataDirectoryOwnershipError::Unavailable {
                reason: format!(
                    "failed to publish ownership metadata in {}: {error}",
                    lock_path.display()
                ),
            });
        }

        Ok(Self {
            root_path: root_path.to_path_buf(),
            lock_path,
            lock_file,
            recovered_stale_owner,
        })
    }

    /// Return the exclusively owned persistent root.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Return the sibling lock-file path.
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Report whether stale owner metadata was replaced after acquiring the lock.
    pub fn recovered_stale_owner(&self) -> bool {
        self.recovered_stale_owner
    }
}

impl Drop for DataDirectoryOwnership {
    fn drop(&mut self) {
        if self.lock_file.set_len(0).is_ok() {
            let _ = self.lock_file.sync_data();
        }
        let _ = self.lock_file.unlock();
    }
}

fn read_owner_metadata(file: &mut File) -> String {
    if file.seek(SeekFrom::Start(0)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(512).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let metadata = String::from_utf8_lossy(&bytes);
    let values = metadata
        .lines()
        .filter(|line| line.starts_with("pid=") || line.starts_with("version="))
        .collect::<Vec<_>>();
    if values.is_empty() {
        String::new()
    } else {
        format!(" ({})", values.join(", "))
    }
}

fn publish_owner_metadata(file: &mut File) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(
        file,
        "pid={}\nversion={}",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    )?;
    file.sync_data()
}
