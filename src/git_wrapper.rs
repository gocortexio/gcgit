use anyhow::{Result, Context};
use git2::{Repository, Status, StatusOptions, Signature};


pub struct GitWrapper {
    repo: Repository,
}

impl GitWrapper {
    pub fn new(path: &str) -> Result<Self> {
        let repo = Repository::open(path)
            .or_else(|_| Repository::init(path))
            .context("Failed to open or initialize Git repository")?;

        Ok(Self { repo })
    }

    pub fn new_for_instance(instance_name: &str) -> Result<Self> {
        let instance_path = std::path::Path::new(instance_name);
        
        // Ensure the instance directory exists
        if !instance_path.exists() {
            return Err(anyhow::anyhow!("Instance directory '{}' does not exist", instance_name));
        }

        let repo = Repository::open(instance_name)
            .or_else(|_| Repository::init(instance_name))
            .context("Failed to open or initialize Git repository for instance")?;

        Ok(Self { repo })
    }

    pub fn get_changed_files_from_main(&self) -> Result<Vec<String>> {
        let mut changed_files = Vec::new();

        // Get current HEAD
        let head = self.repo.head()
            .context("Failed to get HEAD reference")?;
        let head_commit = head.peel_to_commit()
            .context("Failed to peel HEAD to commit")?;

        // Try to get main branch
        let main_ref = self.repo.find_reference("refs/heads/main")
            .or_else(|_| self.repo.find_reference("refs/heads/master"));

        if let Ok(main_ref) = main_ref {
            let main_commit = main_ref.peel_to_commit()
                .context("Failed to peel main branch to commit")?;

            // Get diff between main and HEAD
            let main_tree = main_commit.tree()
                .context("Failed to get main branch tree")?;
            let head_tree = head_commit.tree()
                .context("Failed to get HEAD tree")?;

            let diff = self.repo.diff_tree_to_tree(Some(&main_tree), Some(&head_tree), None)
                .context("Failed to create diff")?;

            diff.foreach(
                &mut |delta, _progress| {
                    if let Some(path) = delta.new_file().path() {
                        if let Some(path_str) = path.to_str() {
                            changed_files.push(path_str.to_string());
                        }
                    }
                    true
                },
                None,
                None,
                None,
            ).context("Failed to process diff")?;
        } else {
            // No main branch, get all tracked files
            let mut status_options = StatusOptions::new();
            status_options.include_untracked(false);
            
            let statuses = self.repo.statuses(Some(&mut status_options))
                .context("Failed to get repository status")?;

            for status in statuses.iter() {
                if let Some(path) = status.path() {
                    changed_files.push(path.to_string());
                }
            }
        }

        Ok(changed_files)
    }

    pub fn is_file_deleted(&self, file_path: &str) -> Result<bool> {
        let mut status_options = StatusOptions::new();
        status_options.pathspec(file_path);
        
        let statuses = self.repo.statuses(Some(&mut status_options))
            .context("Failed to get file status")?;

        for status in statuses.iter() {
            if status.path() == Some(file_path) {
                return Ok(status.status().contains(Status::WT_DELETED) || 
                         status.status().contains(Status::INDEX_DELETED));
            }
        }

        Ok(false)
    }

    #[allow(dead_code)]
    pub fn get_all_yaml_files(&self) -> Result<Vec<String>> {
        let mut yaml_files = Vec::new();
        
        let mut status_options = StatusOptions::new();
        status_options.include_untracked(false);
        
        let statuses = self.repo.statuses(Some(&mut status_options))
            .context("Failed to get repository status")?;

        for status in statuses.iter() {
            if let Some(path) = status.path() {
                if path.ends_with(".yaml") || path.ends_with(".yml") {
                    yaml_files.push(path.to_string());
                }
            }
        }

        Ok(yaml_files)
    }

    pub fn get_repository_status(&self) -> Result<Vec<(String, Status)>> {
        let mut file_statuses = Vec::new();
        
        let statuses = self.repo.statuses(None)
            .context("Failed to get repository status")?;

        for status in statuses.iter() {
            if let Some(path) = status.path() {
                file_statuses.push((path.to_string(), status.status()));
            }
        }

        Ok(file_statuses)
    }

    /// Check if there are any uncommitted changes (staged or unstaged) in the repository
    #[allow(dead_code)]
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        let statuses = self.repo.statuses(None)
            .context("Failed to get repository status")?;

        for status in statuses.iter() {
            let status_flags = status.status();
            // Check for any changes: staged, unstaged modifications, new files, etc.
            if !status_flags.is_empty() && !status_flags.contains(Status::IGNORED) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if specific files have changes after adding them to staging
    /// Returns (has_changes, count_of_changed_files, list_of_changed_files)
    pub fn has_changes_after_add(&self, files: &[String]) -> Result<(bool, usize, Vec<String>)> {
        // Add files to staging
        self.add_files(files)?;
        
        // Check if there are any staged changes and collect them
        let statuses = self.repo.statuses(None)
            .context("Failed to get repository status")?;

        let mut changed_files = Vec::new();
        for status in statuses.iter() {
            let status_flags = status.status();
            // Check for staged changes (INDEX_NEW, INDEX_MODIFIED, etc.)
            if status_flags.contains(Status::INDEX_NEW) || 
               status_flags.contains(Status::INDEX_MODIFIED) ||
               status_flags.contains(Status::INDEX_DELETED) {
                if let Some(path) = status.path() {
                    changed_files.push(path.to_string());
                }
            }
        }

        let changed_count = changed_files.len();
        Ok((changed_count > 0, changed_count, changed_files))
    }

    pub fn add_files(&self, files: &[String]) -> Result<()> {
        let mut index = self.repo.index()
            .context("Failed to get repository index")?;

        for file in files {
            index.add_path(std::path::Path::new(file))
                .with_context(|| format!("Failed to add file to index: {}", file))?;
        }

        index.write()
            .context("Failed to write index")?;

        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<()> {
        let mut index = self.repo.index()
            .context("Failed to get repository index")?;
        let tree_id = index.write_tree()
            .context("Failed to write tree")?;
        let tree = self.repo.find_tree(tree_id)
            .context("Failed to find tree")?;

        // Try to get signature from Git config, fallback to default if not available
        let signature = match self.repo.signature() {
            Ok(sig) => sig,
            Err(_) => {
                // Fallback to gcgit default signature if Git config is not set
                Signature::now("gcgit", "gcgit@localhost")
                    .context("Failed to create fallback signature")?
            }
        };

        // Handle both initial commit and subsequent commits
        match self.repo.head() {
            Ok(head) => {
                // Repository has commits, create commit with parent
                let parent_commit = head.peel_to_commit()
                    .context("Failed to peel HEAD to commit")?;
                self.repo.commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &[&parent_commit],
                ).context("Failed to create commit")?;
            }
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
                // Repository is empty, create initial commit
                self.repo.commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &[],
                ).context("Failed to create initial commit")?;
            }
            Err(e) => return Err(anyhow::anyhow!("Failed to get HEAD reference: {}", e)),
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_modified_files_in_instance(&self, instance_name: &str) -> Result<Vec<String>> {
        let statuses = self.get_repository_status()?;
        let mut modified_files = Vec::new();

        for (path, status) in statuses {
            if path.starts_with(&format!("{}/", instance_name)) && 
               (path.ends_with(".yaml") || path.ends_with(".yml")) &&
               (status.contains(Status::WT_MODIFIED) || 
                status.contains(Status::WT_NEW) || 
                status.contains(Status::INDEX_MODIFIED) ||
                status.contains(Status::INDEX_NEW)) {
                modified_files.push(path);
            }
        }

        Ok(modified_files)
    }

    pub fn get_modified_files_in_current_repo(&self) -> Result<Vec<String>> {
        let statuses = self.get_repository_status()?;
        let mut modified_files = Vec::new();

        for (path, status) in statuses {
            if (path.ends_with(".yaml") || path.ends_with(".yml")) &&
               (status.contains(Status::WT_MODIFIED) || 
                status.contains(Status::WT_NEW) || 
                status.contains(Status::INDEX_MODIFIED) ||
                status.contains(Status::INDEX_NEW)) {
                modified_files.push(path);
            }
        }

        Ok(modified_files)
    }
}
