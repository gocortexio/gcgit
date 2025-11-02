use anyhow::{Result, Context};
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
        
        // Check if lock file exists
        if lock_path.exists() {
            // Read the PID from the lock file
            match fs::read_to_string(&lock_path) {
                Ok(contents) => {
                    if let Ok(locked_pid) = contents.trim().parse::<u32>() {
                        // Check if the process is still running
                        if Self::is_process_running(locked_pid) {
                            return Err(anyhow::anyhow!(
                                "Instance '{}' is locked by another gcgit process (PID {}). \
                                 Wait for the other operation to complete or remove {}.lock if the process is stuck.",
                                instance_name,
                                locked_pid,
                                instance_name
                            ));
                        } else {
                            // Stale lock file - process is no longer running
                            eprintln!("WARNING: Removing stale lock file from terminated process {locked_pid}");
                            fs::remove_file(&lock_path)
                                .context("Failed to remove stale lock file")?;
                        }
                    }
                },
                Err(_) => {
                    // Lock file is corrupted or unreadable - remove it
                    eprintln!("WARNING: Removing corrupted lock file");
                    let _ = fs::remove_file(&lock_path);
                }
            }
        }
        
        // Write our PID to the lock file
        let current_pid = process::id();
        fs::write(&lock_path, current_pid.to_string())
            .with_context(|| format!("Failed to create lock file at {}", lock_path.display()))?;
        
        Ok(Self {
            lock_path,
            acquired: true,
        })
    }
    
    /// Check if a process with the given PID is currently running
    /// Platform-specific implementation
    #[cfg(unix)]
    fn is_process_running(pid: u32) -> bool {
        // Send signal 0 to check if process exists without affecting it
        let output = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output();
        
        match output {
            Ok(output) => output.status.code() == Some(0),
            Err(_) => false,
        }
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
            },
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
                eprintln!("WARNING: Failed to remove lock file {}: {}", self.lock_path.display(), e);
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
        assert!(result.unwrap_err().to_string().contains("locked by another"));
        
        // Clean up
        let _ = fs::remove_dir_all(test_instance);
    }
}
