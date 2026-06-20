use std::path::PathBuf;

use crate::error::Result;
use crate::ids::{ProjectId, TenantId, UserId};

#[derive(Debug, Clone)]
pub struct NoemaPaths {
    pub root: PathBuf,
}

impl NoemaPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn tenant_dir(&self, tenant: &TenantId) -> PathBuf {
        self.root.join("tenants").join(tenant.as_str())
    }

    pub fn user_cortex_dir(&self, tenant: &TenantId, user: &UserId) -> PathBuf {
        self.tenant_dir(tenant)
            .join("users")
            .join(user.as_str())
            .join("cortex")
    }

    pub fn project_cortex_dir(&self, tenant: &TenantId, project: &ProjectId) -> PathBuf {
        self.tenant_dir(tenant)
            .join("projects")
            .join(project.as_str())
            .join("cortex")
    }

    pub fn init_personal_layout(&self, user: &UserId) -> Result<()> {
        let tenant = TenantId::new("personal");
        let tenant_dir = self.tenant_dir(&tenant);
        for dir in [
            tenant_dir.join("hippocampus").join("snapshots"),
            tenant_dir.join("hippocampus").join("archive"),
            tenant_dir.join("users").join(user.as_str()).join("cortex"),
            tenant_dir.join("users").join(user.as_str()).join("deep"),
            tenant_dir.join("projects"),
            tenant_dir.join("indexes"),
            tenant_dir.join("audit"),
            tenant_dir.join("trash"),
            self.root.join("schema"),
            self.root.join("manifests"),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn tenant_scoped_paths_are_canonical() {
        let paths = NoemaPaths::new("/tmp/noema");
        let tenant = TenantId::new("personal");
        let user = UserId::new("kay");
        let project = ProjectId::new("zode_abc");
        assert_eq!(
            paths.user_cortex_dir(&tenant, &user),
            Path::new("/tmp/noema/tenants/personal/users/kay/cortex")
        );
        assert_eq!(
            paths.project_cortex_dir(&tenant, &project),
            Path::new("/tmp/noema/tenants/personal/projects/zode_abc/cortex")
        );
    }
}
