use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::config::NoemaConfig;
use crate::error::{NoemaError, Result};
use crate::hippocampus::Candidate;
use crate::ids::{CandidateId, MemoryId, ProjectId, TenantId, UserId};
use crate::memory::{MemoryKind, MemoryRecord, RecallMode, Scope, Visibility};
use crate::memorypack::{MemoryPack, MemoryPackItem};
use crate::paths::NoemaPaths;
use crate::project::project_id_from_path;
use crate::recall::recall;
use crate::sensitivity::{Principal, SensitivityLevel};
use crate::store::{read_memory, write_memory};

#[derive(Debug, Clone)]
pub struct NoemaEngine {
    pub root: PathBuf,
    pub paths: NoemaPaths,
}

#[derive(Debug, Clone)]
pub struct RecallRequest {
    pub principal: Principal,
    pub query: String,
    pub cwd: Option<PathBuf>,
    pub budget_tokens: usize,
    pub host: String,
}

#[derive(Debug, Clone)]
pub struct RememberTextRequest {
    pub principal: Principal,
    pub text: String,
    pub scope: Scope,
    pub project_path: Option<PathBuf>,
    pub kind: MemoryKind,
    pub sensitivity: SensitivityLevel,
    pub tags: Vec<String>,
    pub entities: Vec<String>,
}

impl NoemaEngine {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Ok(Self {
            paths: NoemaPaths::new(&root),
            root,
        })
    }

    pub fn from_config(config: &NoemaConfig) -> Result<Self> {
        Self::new(&config.storage.local_root)
    }

    pub fn init_personal(&self, user: &UserId) -> Result<()> {
        self.paths.init_personal_layout(user)
    }

    pub fn remember_text(&self, request: RememberTextRequest) -> Result<MemoryId> {
        let tenant = request.principal.tenant_id.clone();
        if tenant.as_str() != "personal" {
            return Err(NoemaError::PolicyDenied(
                "local engine only accepts personal tenant before enterprise boundary".into(),
            ));
        }
        if matches!(
            request.sensitivity,
            SensitivityLevel::Confidential
                | SensitivityLevel::Restricted
                | SensitivityLevel::Secret
        ) {
            return Err(NoemaError::PolicyDenied(
                "personal mode stores public/internal memories only".into(),
            ));
        }
        let user = request.principal.user_id.clone();
        self.init_personal(&user)?;
        let mut candidate = Candidate::new(
            CandidateId::new(format!("cand_{}", Uuid::new_v4())),
            request.text,
        );
        candidate.tenant_id = tenant.clone();
        candidate.owner_user_id = user.clone();
        candidate.scope = request.scope;
        candidate.kind = request.kind;
        candidate.sensitivity = request.sensitivity;
        candidate.tags = request.tags;
        candidate.entities = request.entities;
        candidate.project_id = request.project_path.as_deref().map(project_id_from_path);

        let memory = memory_from_candidate(&tenant, &user, &candidate);
        let path = memory_path(&self.paths, &tenant, &user, &memory)?;
        write_memory(&path, &memory)?;
        Ok(memory.id)
    }

    pub fn recall(&self, request: RecallRequest) -> Result<MemoryPack> {
        let tenant = request.principal.tenant_id.clone();
        let user = request.principal.user_id.clone();
        let project = request.cwd.as_deref().map(project_id_from_path);
        let memories = load_recall_memories(&self.paths, &tenant, &user, project.as_ref())?;
        let scored = recall(
            &request.query,
            &request.principal,
            project.as_ref(),
            &memories,
        );
        let mut pack = MemoryPack::empty(tenant);
        for score in scored.into_iter().take(8) {
            if let Some(memory) = memories
                .iter()
                .find(|memory| memory.id.as_str() == score.id)
            {
                pack.memories.push(MemoryPackItem {
                    id: memory.id.clone(),
                    scope: format!("{:?}", memory.scope).to_lowercase(),
                    kind: format!("{:?}", memory.kind).to_lowercase(),
                    text: Some(memory.body.clone()),
                    withheld_by_policy: memory.recall_policy.mode == RecallMode::Never,
                    score: score.score,
                });
            }
        }
        Ok(pack)
    }
}

fn memory_from_candidate(tenant: &TenantId, user: &UserId, candidate: &Candidate) -> MemoryRecord {
    let memory_id = MemoryId::new(candidate.id.as_str().replacen("cand_", "mem_", 1));
    let mut memory = MemoryRecord::new_user_preference(
        memory_id,
        tenant.clone(),
        user.clone(),
        candidate.body.clone(),
    );
    memory.scope = candidate.scope;
    memory.project_id = candidate.project_id.clone();
    memory.kind = candidate.kind;
    memory.visibility = match candidate.scope {
        Scope::Project => Visibility::Project,
        Scope::Team => Visibility::Team,
        Scope::Org => Visibility::Org,
        Scope::User => Visibility::Private,
    };
    memory.sensitivity = candidate.sensitivity;
    memory.tags = candidate.tags.clone();
    memory.entities = candidate.entities.clone();
    memory
}

fn memory_path(
    paths: &NoemaPaths,
    tenant: &TenantId,
    user: &UserId,
    memory: &MemoryRecord,
) -> Result<PathBuf> {
    let dir = match memory.scope {
        Scope::Project => {
            let project = memory.project_id.as_ref().ok_or_else(|| {
                NoemaError::InvalidRecord("project memory missing project_id".into())
            })?;
            paths.project_cortex_dir(tenant, project)
        }
        Scope::User | Scope::Team | Scope::Org => paths.user_cortex_dir(tenant, user),
    };
    Ok(dir.join(format!("{}.md", memory.id)))
}

fn load_recall_memories(
    paths: &NoemaPaths,
    tenant: &TenantId,
    user: &UserId,
    project: Option<&ProjectId>,
) -> Result<Vec<MemoryRecord>> {
    let mut out = Vec::new();
    load_memory_dir(&paths.user_cortex_dir(tenant, user), &mut out)?;
    if let Some(project) = project {
        load_memory_dir(&paths.project_cortex_dir(tenant, project), &mut out)?;
    }
    Ok(out)
}

fn load_memory_dir(dir: &Path, out: &mut Vec<MemoryRecord>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) == Some("md") {
            out.push(read_memory(&entry.path())?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{TenantId, UserId};
    use crate::sensitivity::Principal;

    #[test]
    fn engine_recall_returns_memorypack_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let engine = NoemaEngine::new(dir.path()).unwrap();
        let principal = Principal::personal("kay", "zode");
        engine.init_personal(&UserId::new("kay")).unwrap();
        engine
            .remember_text(RememberTextRequest {
                principal: principal.clone(),
                text: "Prefer Rust for Noema.".to_string(),
                scope: crate::memory::Scope::User,
                project_path: None,
                kind: crate::memory::MemoryKind::Preference,
                sensitivity: crate::sensitivity::SensitivityLevel::Internal,
                tags: vec!["rust".to_string()],
                entities: vec!["Noema".to_string()],
            })
            .unwrap();

        let pack = engine
            .recall(RecallRequest {
                principal,
                query: "rust memory".to_string(),
                cwd: None,
                budget_tokens: 1200,
                host: "zode".to_string(),
            })
            .unwrap();

        assert_eq!(pack.tenant_id, TenantId::new("personal"));
        assert!(pack.to_markdown().contains("Relevant Memories"));
    }
}
