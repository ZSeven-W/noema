use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{MemoryId, ProjectId, TeamId, TenantId, UserId};
use crate::sensitivity::{DataClass, SensitivityLevel};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    Project,
    Team,
    Org,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Cortex,
    Deep,
    Tombstone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Decision,
    Constraint,
    Fact,
    Reference,
    Workflow,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallMode {
    Normal,
    Never,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Project,
    Team,
    Org,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Write,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclEntry {
    pub principal: String,
    pub access: AccessLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLink {
    pub rel: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySource {
    pub kind: String,
    pub agent: String,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    #[default]
    Active,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptionMeta {
    pub scheme: String,
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallPolicy {
    #[serde(default = "default_recall_mode")]
    pub mode: RecallMode,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub allow_external_llm: bool,
}

fn default_recall_mode() -> RecallMode {
    RecallMode::Normal
}

impl Default for RecallPolicy {
    fn default() -> Self {
        Self {
            mode: RecallMode::Normal,
            allowed_hosts: Vec::new(),
            allow_external_llm: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: MemoryId,
    pub tenant_id: TenantId,
    pub owner_user_id: UserId,
    pub scope: Scope,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub team_id: Option<TeamId>,
    pub layer: Layer,
    pub kind: MemoryKind,
    #[serde(default)]
    pub visibility: Visibility,
    #[serde(default)]
    pub acl: Vec<AclEntry>,
    pub confidence: f32,
    pub importance: f32,
    pub sensitivity: SensitivityLevel,
    #[serde(default)]
    pub data_classes: Vec<DataClass>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub links: Vec<MemoryLink>,
    #[serde(default)]
    pub source: MemorySource,
    #[serde(default)]
    pub status: MemoryStatus,
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub recall_policy: RecallPolicy,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub encryption: Option<EncryptionMeta>,
    pub body: String,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl MemoryRecord {
    pub fn new_user_preference(
        id: MemoryId,
        tenant_id: TenantId,
        owner_user_id: UserId,
        body: impl Into<String>,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            tenant_id,
            owner_user_id,
            scope: Scope::User,
            project_id: None,
            team_id: None,
            layer: Layer::Cortex,
            kind: MemoryKind::Preference,
            visibility: Visibility::Private,
            acl: Vec::new(),
            confidence: 1.0,
            importance: 0.5,
            sensitivity: SensitivityLevel::Internal,
            data_classes: Vec::new(),
            tags: Vec::new(),
            entities: Vec::new(),
            links: Vec::new(),
            source: MemorySource {
                kind: "manual".to_string(),
                agent: "noema-cli".to_string(),
                uri: None,
            },
            status: MemoryStatus::Active,
            use_count: 0,
            recall_policy: RecallPolicy {
                mode: RecallMode::Normal,
                allowed_hosts: Vec::new(),
                allow_external_llm: false,
            },
            created_at: now,
            updated_at: now,
            last_used_at: None,
            encryption: None,
            body: body.into(),
        }
    }
}
