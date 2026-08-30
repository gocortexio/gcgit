// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process;

/// Instance lock to prevent concurrent operations on the same instance directory
/// Uses a .lock file containing the current process ID
#[derive(Debug)]
pub struct InstanceLock {
    lock_path: PathBuf,
    acquired: bool,
}

impl InstanceLock {
    /// Attempt to acquire a lock on the specified instance directory
    /// Returns error if another process holds the lock
    pub fn acquire(instance_name: &str) -> Result<Self> {
        let lock_path = PathBuf::from(instance_name).join(".gcgit.lock");

        // Two attempts: the first claims the lock, and if it is already held by a
        // process that no longer exists the stale file is cleared and the claim is
        // retried once.
        for attempt in 0..2 {
            match Self::try_create(&lock_path) {
                Ok(()) => {
                    return Ok(Self {
                        lock_path,
                        acquired: true,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 1 {
                        return Err(anyhow::anyhow!(
                            "Instance '{instance_name}' is locked by another gcgit process. \
                             Wait for that operation to finish, or remove {} if no gcgit process is running.",
                            lock_path.display()
                        ));
                    }

                    let holder = fs::read_to_string(&lock_path)
                        .ok()
                        .and_then(|c| c.trim().parse::<u32>().ok());

                    match holder {
                        Some(pid) if Self::is_process_running(pid) => {
                            return Err(anyhow::anyhow!(
                                "Instance '{instance_name}' is locked by another gcgit process (PID {pid}). \
                                 Wait for that operation to finish, or remove {} if the process is stuck.",
                                lock_path.display()
                            ));
                        }
                        Some(pid) => {
                            eprintln!(
                                "[INFO] Removing stale lock file from terminated process {pid}"
                            );
                        }
                        None => {
                            eprintln!(
                                "[WARN] Removing unreadable lock file at {}",
                                lock_path.display()
                            );
                        }
                    }

                    fs::remove_file(&lock_path).with_context(|| {
                        format!(
                            "Failed to remove stale lock file at {}",
                            lock_path.display()
                        )
                    })?;
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("Failed to create lock file at {}", lock_path.display())
                    })
                }
            }
        }

        unreachable!("the loop either returns or retries exactly once")
    }

    /// Create the lock file, failing if it already exists.
    ///
    /// `create_new` performs the existence check and the creation as one atomic
    /// operation. Checking for the file and then writing it left a window in which
    /// two processes could both pass the check and both believe they held the lock.
    fn try_create(lock_path: &PathBuf) -> std::io::Result<()> {
        use std::io::Write;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)?;
        write!(file, "{}", process::id())
    }

    /// Check if a process with the given PID is currently running
    /// Platform-specific implementation
    #[cfg(unix)]
    fn is_process_running(pid: u32) -> bool {
        // On Linux, which is the primary deployment target, /proc answers this
        // without spawning anything. Elsewhere fall back to signal 0 via kill.
        let proc_entry = PathBuf::from("/proc").join(pid.to_string());
        if proc_entry.exists() {
            return true;
        }
        if PathBuf::from("/proc/self").exists() {
            // /proc is mounted and the entry is absent, so the process is gone.
            return false;
        }

        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.code() == Some(0))
            .unwrap_or(false)
    }

    /// Check if a process with the given PID is currently running
    /// Platform-specific implementation for Windows
    #[cfg(windows)]
    fn is_process_running(pid: u32) -> bool {
        use std::process::Command;

        // Use tasklist to check if process exists
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH", "/FO", "CSV"])
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    }

    /// Check if a process with the given PID is currently running
    /// Fallback implementation for other platforms
    #[cfg(not(any(unix, windows)))]
    fn is_process_running(_pid: u32) -> bool {
        // Conservative approach: assume process is still running
        // User will need to manually remove stale locks
        true
    }
}

impl Drop for InstanceLock {
    /// Automatically release the lock when the InstanceLock goes out of scope
    fn drop(&mut self) {
        if self.acquired {
            if let Err(e) = fs::remove_file(&self.lock_path) {
                eprintln!(
                    "[ERROR] Failed to remove lock file {}: {}",
                    self.lock_path.display(),
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_lock_acquire_and_release() {
        let test_instance = "test_lock_instance";

        // Clean up if exists
        let _ = fs::remove_dir_all(test_instance);
        fs::create_dir(test_instance).unwrap();

        // Acquire lock
        let lock = InstanceLock::acquire(test_instance).unwrap();

        // Lock file should exist
        assert!(PathBuf::from(test_instance).join(".gcgit.lock").exists());

        // Drop lock
        drop(lock);

        // Lock file should be removed
        assert!(!PathBuf::from(test_instance).join(".gcgit.lock").exists());

        // Clean up
        let _ = fs::remove_dir_all(test_instance);
    }

    #[test]
    fn stale_lock_from_a_dead_process_is_reclaimed() {
        let test_instance = "test_stale_lock_instance";
        let _ = fs::remove_dir_all(test_instance);
        fs::create_dir(test_instance).unwrap();

        // PID 1 always exists, so use a value that cannot be live: write a PID that
        // is almost certainly free, then confirm the lock is taken over.
        let lock = PathBuf::from(test_instance).join(".gcgit.lock");
        fs::write(&lock, "4294967294").unwrap();

        let acquired = InstanceLock::acquire(test_instance);
        assert!(
            acquired.is_ok(),
            "a stale lock must be reclaimed: {acquired:?}"
        );
        drop(acquired);
        assert!(!lock.exists());

        let _ = fs::remove_dir_all(test_instance);
    }

    #[test]
    fn unreadable_lock_content_is_reclaimed() {
        let test_instance = "test_garbage_lock_instance";
        let _ = fs::remove_dir_all(test_instance);
        fs::create_dir(test_instance).unwrap();

        let lock = PathBuf::from(test_instance).join(".gcgit.lock");
        fs::write(&lock, "not-a-pid").unwrap();

        assert!(InstanceLock::acquire(test_instance).is_ok());

        let _ = fs::remove_dir_all(test_instance);
    }

    #[test]
    fn test_concurrent_lock_prevention() {
        let test_instance = "test_concurrent_instance";

        // Clean up if exists
        let _ = fs::remove_dir_all(test_instance);
        fs::create_dir(test_instance).unwrap();

        // Acquire first lock
        let _lock1 = InstanceLock::acquire(test_instance).unwrap();

        // Attempt to acquire second lock should fail
        let result = InstanceLock::acquire(test_instance);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("locked by another"));

        // Clean up
        let _ = fs::remove_dir_all(test_instance);
    }
}
