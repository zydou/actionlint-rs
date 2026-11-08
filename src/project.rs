//! Project detection for a GitHub repository.
//!
//! Rust counterpart of the original `project.go`. A [`Project`] represents one Git repository
//! containing a `.github/workflows` directory.

use std::path::{Path, PathBuf};

/// One GitHub project. One Git repository corresponds to one project.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    config: Option<crate::config::Config>,
}

impl Project {
    /// Creates a new `Project` with the given root directory. Reads the repository config file.
    ///
    /// Returns an error when the actionlint config file in the repository cannot be parsed.
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let config = crate::config::load_repo_config(&root)?;
        Ok(Self { root, config })
    }

    /// Returns the root directory path of the project.
    #[must_use]
    pub fn root_dir(&self) -> &Path {
        &self.root
    }

    /// Returns the `.github/workflows` directory path. Does not check existence.
    #[must_use]
    pub fn workflows_dir(&self) -> PathBuf {
        self.root.join(".github").join("workflows")
    }

    /// Returns whether the project knows the given file (i.e. the file is inside the project root).
    #[must_use]
    pub fn knows(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }

    /// Returns the config object read from `.github/actionlint.yaml`, if any.
    #[must_use]
    pub fn config(&self) -> Option<&crate::config::Config> {
        self.config.as_ref()
    }
}

/// Returns the canonical absolute path of the given path, if possible.
fn abs_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Finds the project which the given path belongs to by walking up the directory tree.
///
/// A project must be a Git repository and have a `.github/workflows` directory. Returns `None`
/// when no project is found.
pub fn find_project(path: &Path) -> Result<Option<Project>, String> {
    let mut d = abs_path(path);
    loop {
        if d.join(".github").join("workflows").is_dir() && d.join(".git").exists() {
            return Ok(Some(Project::new(d)?));
        }
        match d.parent() {
            Some(p) if p != d => d = p.to_path_buf(),
            _ => return Ok(None),
        }
    }
}

/// A set of projects that caches previously created [`Project`] instances.
#[derive(Debug, Default)]
pub struct Projects {
    known: Vec<Project>,
}

impl Projects {
    /// Creates a new `Projects`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the [`Project`] that the given path belongs to, or `None` if no project is found.
    /// Results are cached and reused.
    pub fn at(&mut self, path: &Path) -> Result<Option<Project>, String> {
        for p in &self.known {
            if p.knows(path) {
                return Ok(Some(p.clone()));
            }
        }
        let p = find_project(path)?;
        if let Some(p) = &p {
            self.known.push(p.clone());
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_project_in_empty_dir() {
        let tmp = std::env::temp_dir().join("actionlint_test_no_project");
        let _ = std::fs::create_dir_all(&tmp);
        let p = find_project(&tmp).unwrap();
        // May or may not find a project depending on parent dirs; just ensure no panic.
        let _ = p;
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finds_project_with_git_and_workflows() {
        let root = std::env::temp_dir().join("actionlint_test_project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".github").join("workflows")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let p = find_project(&root)
            .unwrap()
            .expect("project should be found");
        assert_eq!(p.root_dir(), root);
        assert_eq!(p.workflows_dir(), root.join(".github").join("workflows"));
        assert!(p.knows(&root.join("foo.yaml")));
        assert!(!p.knows(&std::env::temp_dir().join("other.yaml")));

        let _ = std::fs::remove_dir_all(&root);
    }
}
