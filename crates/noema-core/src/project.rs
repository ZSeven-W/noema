use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ids::ProjectId;

pub fn project_id_from_path(path: &Path) -> ProjectId {
    if let Some(id) = project_id_from_git(path) {
        return id;
    }

    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    ProjectId::new(format!("path_{:016x}", hasher.finish()))
}

fn project_id_from_git(path: &Path) -> Option<ProjectId> {
    let common_dir = git_common_dir(path)?;
    let remote = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("config")
        .arg("--get")
        .arg("remote.origin.url")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|remote| !remote.is_empty());

    let identity = match remote {
        Some(remote) => format!("git:{remote}:{}", common_dir.display()),
        None => format!("git:{}", common_dir.display()),
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity.hash(&mut hasher);
    Some(ProjectId::new(format!("git_{:016x}", hasher.finish())))
}

fn git_common_dir(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--git-common-dir")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let common = PathBuf::from(raw);
    if common.is_absolute() {
        Some(common)
    } else {
        Some(path.join(common))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_fallback_is_stable_for_same_input() {
        let a = project_id_from_path(Path::new("/tmp/example"));
        let b = project_id_from_path(Path::new("/tmp/example"));
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("path_"));
    }
}
