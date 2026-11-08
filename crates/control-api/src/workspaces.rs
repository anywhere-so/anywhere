//! Workspace membership and app capabilities, independent of personal plans.
use std::fmt;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Personal,
    Team,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Owner,
    Member,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Workspace {
    pub id: Uuid,
    pub namespace: String,
    pub title: String,
    pub kind: WorkspaceKind,
    pub role: WorkspaceRole,
    pub developer: bool,
    pub can_create: bool,
    pub active: bool,
    /// Funded seat capacity is visible only to workspace owners.
    pub paid_developers: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceList {
    pub workspaces: Vec<Workspace>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceInvitationPreview {
    pub namespace: String,
    pub title: String,
    pub invited_by: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCreate {
    pub namespace: String,
    pub title: String,
}

/// A name a team could take now: nobody's account has it or held it within
/// its 30-day release hold, and nobody else's checkout holds it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceName {
    pub namespace: String,
    pub available: bool,
    /// The portal's page where a person pays for the team with this name.
    pub checkout_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceNamePath {
    pub namespace: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspacePath {
    pub workspace: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceMemberPath {
    pub workspace: String,
    pub user: Uuid,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceInvitationPath {
    pub workspace: String,
    pub invitation: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceMember {
    pub user: Uuid,
    pub username: String,
    pub role: WorkspaceRole,
    pub developer: bool,
    pub joined_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceMembers {
    pub members: Vec<WorkspaceMember>,
    pub invitations: Vec<WorkspaceInvitation>,
    pub activity: Vec<WorkspaceActivity>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceActivity {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub target: String,
    /// IDs and permission or spending changes only; no invitation secret or app content.
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMemberUpdate {
    pub role: WorkspaceRole,
    pub developer: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceInvite {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceInvitation {
    pub id: Uuid,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub accepted: bool,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceInvitationCreated {
    pub operation: crate::Operation,
    pub invitation: WorkspaceInvitation,
    /// The fragment carries the invitation secret; never log this URL.
    pub url: String,
}

impl fmt::Debug for WorkspaceInvitationCreated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceInvitationCreated")
            .field("operation", &self.operation)
            .field("invitation_id", &self.invitation.id)
            .field("url", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceInvitationAccept {
    pub secret: String,
}

impl fmt::Debug for WorkspaceInvitationAccept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceInvitationAccept")
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppCapabilities {
    pub develop: bool,
    pub manage: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceApp {
    pub id: Uuid,
    pub app: crate::App,
    pub capabilities: AppCapabilities,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceApps {
    pub apps: Vec<WorkspaceApp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAppSharing {
    pub everyone: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceAppPath {
    pub workspace: String,
    pub app: String,
}

/// What a person can do with a team app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppAccessLevel {
    /// They can't open it.
    None,
    /// They open and use it.
    Use,
    /// They also deploy, exec, pull, read its logs and set its secrets.
    Develop,
}

/// Why a person has their access to a team app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppAccessSource {
    /// They own the workspace, which no grant changes.
    Owner,
    /// They made the app, which no grant changes.
    Creator,
    /// Their per-app grant.
    Grant,
    /// The app is visible to everyone in the workspace.
    Workspace,
}

/// One workspace member's effective access to a team app, and why.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceAppGrant {
    pub user: Uuid,
    pub username: String,
    /// Their role in the workspace.
    pub role: WorkspaceRole,
    /// Whether they made the app.
    pub creator: bool,
    /// Whether they hold a funded vibecoder seat, which developing needs.
    pub seat: bool,
    /// What they can do with the app now.
    pub access: AppAccessLevel,
    /// Whether they manage its sharing and deletion: owners and its creator.
    pub manage: bool,
    /// Why they have `access`; absent when it's `none`.
    pub source: Option<AppAccessSource>,
    /// Their per-app grant, which overrides the workspace default; never
    /// counts for owners or the creator.
    pub grant: Option<crate::ShareRole>,
    /// What they'd have with no grant.
    pub default_access: AppAccessLevel,
}

/// Who uses and develops a team app. On an app visible to everyone, each
/// member uses it and a seated one develops it, unless a grant says otherwise;
/// on a restricted one, only owners, its creator and people granted reach it.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceAppAccess {
    pub everyone: bool,
    pub people: Vec<WorkspaceAppGrant>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceAppMemberPath {
    pub workspace: String,
    pub app: String,
    pub user: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAppGrantUpdate {
    pub role: crate::ShareRole,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkspaceCheckoutPath {
    pub workspace: String,
    pub session: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AppIdentity {
    pub workspace_id: Uuid,
    pub workspace: String,
    pub app_id: Uuid,
    pub app: String,
}

/// Local project metadata, never a credential and never a mutable global default.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectBinding {
    pub api: String,
    pub workspace_id: Uuid,
    pub workspace: String,
    pub app_id: Uuid,
    pub app: String,
}

/// Stable app identity captured by the deploy's transaction, before its name
/// can ever be reused. IDs are response headers, not mutable client context.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AppIdentityHeaders {
    #[serde(rename = "app-id")]
    pub app_id: Uuid,
    #[serde(rename = "workspace-id")]
    pub workspace_id: Uuid,
    #[serde(rename = "workspace-namespace")]
    pub workspace_namespace: String,
}
