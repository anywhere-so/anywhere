//! Optional stable app identity preconditions. They only narrow access.
use http::HeaderMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct ProjectPreconditions {
    pub workspace: Option<Uuid>,
    pub app: Option<Uuid>,
}
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum ProjectPreconditionError {
    #[error("send each project precondition once, as a quoted UUID")]
    Invalid,
    #[error(
        "this namespace no longer names the bound workspace; bind the project explicitly again"
    )]
    WorkspaceChanged,
    #[error(
        "this name no longer names the bound app; inspect the replacement and bind the project explicitly before changing it"
    )]
    AppChanged,
}
impl ProjectPreconditionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "app_binding_invalid",
            Self::WorkspaceChanged => "workspace_binding_changed",
            Self::AppChanged => "app_binding_changed",
        }
    }
}
impl ProjectPreconditions {
    pub fn headers(headers: &HeaderMap) -> Result<Self, ProjectPreconditionError> {
        let parse = |name: &str| -> Result<Option<Uuid>, ProjectPreconditionError> {
            let values = headers.get_all(name);
            let mut values = values.iter();
            let Some(value) = values.next() else {
                return Ok(None);
            };
            if values.next().is_some() {
                return Err(ProjectPreconditionError::Invalid);
            }
            let text = value
                .to_str()
                .map_err(|_| ProjectPreconditionError::Invalid)?;
            let text = text
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(ProjectPreconditionError::Invalid)?;
            Uuid::parse_str(text)
                .map(Some)
                .map_err(|_| ProjectPreconditionError::Invalid)
        };
        Ok(Self {
            workspace: parse("if-workspace")?,
            app: parse("if-match")?,
        })
    }
    pub fn check_workspace(&self, workspace: Uuid) -> Result<(), ProjectPreconditionError> {
        if self.workspace.is_some_and(|id| id != workspace) {
            return Err(ProjectPreconditionError::WorkspaceChanged);
        }
        Ok(())
    }
    pub fn check(&self, workspace: Uuid, app: Uuid) -> Result<(), ProjectPreconditionError> {
        self.check_workspace(workspace)?;
        if self.app.is_some_and(|id| id != app) {
            return Err(ProjectPreconditionError::AppChanged);
        }
        Ok(())
    }
}

pub fn workspace_invitation_shaped(secret: &str) -> bool {
    secret.len() == 64
        && secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
