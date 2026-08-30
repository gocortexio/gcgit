// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result};
use git2::{Repository, Signature, Status};
use std::path::PathBuf;

pub struct GitWrapper {
    repo: Repository,
    /// Path from the repository working directory to the instance directory.
    ///
    /// Empty when the repository is the instance directory itself. Set when gcgit is
    /// operating inside a repository that already surrounds the instance, in which
    /// case every path it stages has to be expressed relative to that repository's
    /// root rather than to the instance.
    prefix: PathBuf,
}

impl GitWrapper {
    /// Open the repository that should hold this instance's files.
    ///
    /// Resolution order:
    ///
    /// 1. A repository at the instance directory itself. This is what earlier releases
    ///    created, so existing layouts keep working unchanged.
    /// 2. A repository that already contains the instance directory. This is the
    ///    continuous integration case: the checkout is the backup repository, and
    ///    creating a nested one inside it would leave the pulled files untracked by the
    ///    repository that is actually pushed.
    /// 3. Otherwise a new repository at the instance directory.
    pub fn new_for_instance(instance_name: &str) -> Result<Self> {
        let instance_path = std::path::Path::new(instance_name);

        if !instance_path.exists() {
            return Err(anyhow::anyhow!(
                "Instance directory '{instance_name}' does not exist"
            ));
        }

        if instance_path.join(".git").exists() {
            let repo = Repository::open(instance_path)
                .context("Failed to open the Git repository in the instance directory")?;
            return Ok(Self {
                repo,
                prefix: PathBuf::new(),
            });
        }

        if let Some((repo, prefix)) = Self::discover_enclosing(instance_path)? {
            return Ok(Self { repo, prefix });
        }

        let repo = Repository::init(instance_path)
            .context("Failed to initialise a Git repository for the instance")?;
        Ok(Self {
            repo,
            prefix: PathBuf::new(),
        })
    }

    /// Find a repository that already contains `instance_path`, and work out where the
    /// instance sits inside it.
    ///
    /// A bare repository is rejected: there is no working tree to write files into.
    fn discover_enclosing(
        instance_path: &std::path::Path,
    ) -> Result<Option<(Repository, PathBuf)>> {
        let absolute = std::fs::canonicalize(instance_path)
            .with_context(|| format!("Failed to resolve {}", instance_path.display()))?;

        let repo = match Repository::discover(&absolute) {
            Ok(repo) => repo,
            Err(_) => return Ok(None),
        };

        let Some(workdir) = repo.workdir().map(|w| w.to_path_buf()) else {
            return Ok(None);
        };
        let workdir = std::fs::canonicalize(&workdir).unwrap_or(workdir);

        let Ok(prefix) = absolute.strip_prefix(&workdir) else {
            return Ok(None);
        };

        Ok(Some((repo, prefix.to_path_buf())))
    }

    /// Express a path that is relative to the instance directory as one relative to
    /// the repository working directory.
    fn repo_path(&self, instance_relative: &str) -> PathBuf {
        if self.prefix.as_os_str().is_empty() {
            PathBuf::from(instance_relative)
        } else {
            self.prefix.join(instance_relative)
        }
    }

    /// True when the files live inside a repository that surrounds the instance.
    pub fn uses_enclosing_repository(&self) -> bool {
        !self.prefix.as_os_str().is_empty()
    }

    /// Describe where commits are being written, for operator output.
    pub fn location(&self) -> String {
        match self.repo.workdir() {
            Some(dir) => dir.display().to_string(),
            None => "bare repository".to_string(),
        }
    }

    pub fn get_repository_status(&self) -> Result<Vec<(String, Status)>> {
        let mut file_statuses = Vec::new();

        let statuses = self
            .repo
            .statuses(None)
            .context("Failed to get repository status")?;

        for status in statuses.iter() {
            if let Ok(path) = status.path() {
                file_statuses.push((path.to_string(), status.status()));
            }
        }

        Ok(file_statuses)
    }

    /// Stage `added` and `removed`, then report which of those paths actually changed.
    ///
    /// Returns (has_changes, count_of_changed_files, list_of_changed_files).
    ///
    /// The status walk is restricted to the paths this call staged. Walking the whole
    /// repository would count anything a previous operation left staged, inflating the
    /// reported change count and describing it with a commit message about this pull.
    pub fn has_changes_after_add(
        &self,
        added: &[String],
        removed: &[String],
    ) -> Result<(bool, usize, Vec<String>)> {
        self.add_files(added)?;
        self.remove_files(removed)?;

        // Compare against repository-relative paths: when the instance sits inside a
        // surrounding repository, status reports paths from that repository's root.
        let staged: std::collections::HashSet<String> = added
            .iter()
            .chain(removed.iter())
            .map(|path| self.repo_path(path).to_string_lossy().into_owned())
            .collect();

        let statuses = self
            .repo
            .statuses(None)
            .context("Failed to get repository status")?;

        let mut changed_files = Vec::new();
        for status in statuses.iter() {
            let status_flags = status.status();
            // Check for staged changes (INDEX_NEW, INDEX_MODIFIED, etc.)
            if status_flags.contains(Status::INDEX_NEW)
                || status_flags.contains(Status::INDEX_MODIFIED)
                || status_flags.contains(Status::INDEX_DELETED)
            {
                if let Ok(path) = status.path() {
                    if staged.contains(path) {
                        changed_files.push(path.to_string());
                    }
                }
            }
        }

        let changed_count = changed_files.len();
        Ok((changed_count > 0, changed_count, changed_files))
    }

    pub fn add_files(&self, files: &[String]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        let mut index = self
            .repo
            .index()
            .context("Failed to get repository index")?;

        for file in files {
            let path = self.repo_path(file);
            index
                .add_path(&path)
                .with_context(|| format!("Failed to add file to index: {}", path.display()))?;
        }

        index.write().context("Failed to write index")?;

        Ok(())
    }

    /// Stage the removal of files that have already been deleted from the working tree.
    ///
    /// A path that was never tracked is skipped rather than treated as an error: a file
    /// pruned before it was ever committed has nothing to remove from the index.
    pub fn remove_files(&self, files: &[String]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        let mut index = self
            .repo
            .index()
            .context("Failed to get repository index")?;

        for file in files {
            let path = self.repo_path(file);
            if index.get_path(&path, 0).is_none() {
                continue;
            }
            index
                .remove_path(&path)
                .with_context(|| format!("Failed to remove file from index: {}", path.display()))?;
        }

        index.write().context("Failed to write index")?;

        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<()> {
        let mut index = self
            .repo
            .index()
            .context("Failed to get repository index")?;
        let tree_id = index.write_tree().context("Failed to write tree")?;
        let tree = self
            .repo
            .find_tree(tree_id)
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
                let parent_commit = head
                    .peel_to_commit()
                    .context("Failed to peel HEAD to commit")?;
                self.repo
                    .commit(
                        Some("HEAD"),
                        &signature,
                        &signature,
                        message,
                        &tree,
                        &[&parent_commit],
                    )
                    .context("Failed to create commit")?;
            }
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
                // Repository is empty, create initial commit
                self.repo
                    .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
                    .context("Failed to create initial commit")?;
            }
            Err(e) => return Err(anyhow::anyhow!("Failed to get HEAD reference: {e}")),
        }

        Ok(())
    }

    /// Modified YAML files belonging to this instance.
    ///
    /// Scoped by the instance prefix so that, when the instance sits inside a larger
    /// repository, status reports only this instance's files rather than every YAML
    /// file in the checkout.
    pub fn get_modified_files_in_current_repo(&self) -> Result<Vec<String>> {
        let statuses = self.get_repository_status()?;
        let mut modified_files = Vec::new();
        let prefix = self.prefix.to_string_lossy().into_owned();

        for (path, status) in statuses {
            if !prefix.is_empty() && !path.starts_with(&format!("{prefix}/")) {
                continue;
            }
            if (path.ends_with(".yaml") || path.ends_with(".yml"))
                && (status.contains(Status::WT_MODIFIED)
                    || status.contains(Status::WT_NEW)
                    || status.contains(Status::INDEX_MODIFIED)
                    || status.contains(Status::INDEX_NEW))
            {
                modified_files.push(path);
            }
        }

        Ok(modified_files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempRepo {
        dir: PathBuf,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("gcgit_git_{}_{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn wrapper(&self) -> GitWrapper {
            GitWrapper::new_for_instance(self.dir.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn an_instance_inside_a_repository_does_not_create_a_nested_one() {
        // The continuous integration case: the checkout is the backup repository.
        let outer = TempRepo::new("gw_outer_repo");
        Repository::init(&outer.dir).unwrap();
        let instance = outer.dir.join("prod");
        std::fs::create_dir_all(instance.join("xsiam/dashboards")).unwrap();

        let wrapper = GitWrapper::new_for_instance(instance.to_str().unwrap()).unwrap();

        assert!(
            wrapper.uses_enclosing_repository(),
            "should adopt the surrounding repository"
        );
        assert!(
            !instance.join(".git").exists(),
            "must not create a nested repository"
        );
    }

    #[test]
    fn files_are_staged_relative_to_the_enclosing_repository() {
        let outer = TempRepo::new("gw_prefix_repo");
        Repository::init(&outer.dir).unwrap();
        let instance = outer.dir.join("prod");
        std::fs::create_dir_all(instance.join("xsiam/dashboards")).unwrap();
        std::fs::write(instance.join("xsiam/dashboards/a.yaml"), "id: a\n").unwrap();

        let wrapper = GitWrapper::new_for_instance(instance.to_str().unwrap()).unwrap();
        let (changed, count, files) = wrapper
            .has_changes_after_add(&["xsiam/dashboards/a.yaml".to_string()], &[])
            .unwrap();

        assert!(changed);
        assert_eq!(count, 1);
        // The path is reported from the outer repository's root, not the instance.
        assert_eq!(files[0], "prod/xsiam/dashboards/a.yaml");
    }

    #[test]
    fn an_existing_instance_repository_is_still_used() {
        // Layouts created by earlier releases keep their own repository.
        let repo = TempRepo::new("gw_legacy_instance");
        Repository::init(&repo.dir).unwrap();
        let wrapper = GitWrapper::new_for_instance(repo.dir.to_str().unwrap()).unwrap();
        assert!(!wrapper.uses_enclosing_repository());
    }

    #[test]
    fn deleted_files_are_staged_and_reported() {
        // Pruning a locally-stored object that no longer exists on the platform must
        // produce a staged deletion, otherwise the file stays in Git history for ever.
        let repo = TempRepo::new("delete");
        repo.write("xsiam/dashboards/a.yaml", "id: a");
        repo.write("xsiam/dashboards/b.yaml", "id: b");

        let git = repo.wrapper();
        let both = vec![
            "xsiam/dashboards/a.yaml".to_string(),
            "xsiam/dashboards/b.yaml".to_string(),
        ];
        git.has_changes_after_add(&both, &[]).unwrap();
        git.commit("initial").unwrap();

        // b is removed on the platform, so the local file is pruned.
        fs::remove_file(repo.dir.join("xsiam/dashboards/b.yaml")).unwrap();

        let (has_changes, count, changed) = git
            .has_changes_after_add(
                &["xsiam/dashboards/a.yaml".to_string()],
                &["xsiam/dashboards/b.yaml".to_string()],
            )
            .unwrap();

        assert!(has_changes);
        assert_eq!(count, 1, "only the deletion changed: {changed:?}");
        assert_eq!(changed, vec!["xsiam/dashboards/b.yaml".to_string()]);
        git.commit("prune").unwrap();
    }

    #[test]
    fn unrelated_staged_changes_are_not_counted() {
        // The status walk must be scoped to the paths this pull staged. Counting the
        // whole repository attributed unrelated staged work to the pull's commit
        // message and inflated the reported change count.
        let repo = TempRepo::new("scope");
        repo.write("xsiam/dashboards/a.yaml", "id: a");
        let git = repo.wrapper();
        git.has_changes_after_add(&["xsiam/dashboards/a.yaml".to_string()], &[])
            .unwrap();
        git.commit("initial").unwrap();

        // Something outside this pull is staged beforehand.
        repo.write("appsec/policies/unrelated.yaml", "id: unrelated");
        git.add_files(&["appsec/policies/unrelated.yaml".to_string()])
            .unwrap();

        // This pull touches only the dashboard, and it is unchanged.
        let (has_changes, count, changed) = git
            .has_changes_after_add(&["xsiam/dashboards/a.yaml".to_string()], &[])
            .unwrap();

        assert!(
            !has_changes,
            "unrelated staged file leaked into the count: {changed:?}"
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn removing_an_untracked_path_is_not_an_error() {
        // A file pruned before it was ever committed has nothing to remove from the
        // index, which must not abort the pull.
        let repo = TempRepo::new("untracked");
        let git = repo.wrapper();
        git.remove_files(&["never/tracked.yaml".to_string()])
            .unwrap();
    }
}
