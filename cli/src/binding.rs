//! A project binds stable IDs to one API origin; it never chooses another tenant.
use crate::error::CliError;
use reqwest::header::{HeaderMap, HeaderValue};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use uuid::Uuid;

const DIRECTORY: &str = ".anywhere";
const FILE: &str = "project.json";
const MAX_BYTES: u64 = 4096;
pub use control_api::ProjectBinding as Binding;
fn invalid(detail: impl Into<String>) -> CliError {
    CliError::Binding {
        detail: detail.into(),
    }
}
pub fn read(dir: &Path) -> Result<Option<Binding>, CliError> {
    let path = dir.join(DIRECTORY).join(FILE);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(invalid(format!("cannot read {}: {error}", path.display()))),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| invalid(format!("cannot read {}: {error}", path.display())))?;
    if bytes.len() > MAX_BYTES as usize {
        return Err(invalid("the project binding exceeds 4096 bytes"));
    }
    let binding: Binding = serde_json::from_slice(&bytes).map_err(|_| invalid("the project binding is invalid JSON; inspect .anywhere/project.json and bind the project explicitly again"))?;
    naming::Username::parse(&binding.workspace).map_err(|e| invalid(e.to_string()))?;
    naming::AppName::parse(&binding.app).map_err(|e| invalid(e.to_string()))?;
    crate::settings::uploads::api_origin(&binding.api)?;
    Ok(Some(binding))
}
pub fn headers(headers: &mut HeaderMap, api: &str, dir: &Path) -> Result<(), CliError> {
    if let Some(binding) = read(dir)? {
        if binding.api.trim_end_matches('/') != api.trim_end_matches('/') {
            return Err(invalid(
                "this project is bound to another API origin; select its credentials or bind the project explicitly again",
            ));
        }
        for (name, id) in [
            ("if-workspace", binding.workspace_id),
            ("if-match", binding.app_id),
        ] {
            let value = HeaderValue::from_str(&format!("\"{id}\""))
                .map_err(|_| invalid("the project binding contains an invalid UUID"))?;
            headers.insert(name, value);
        }
    }
    Ok(())
}
pub fn write(dir: &Path, binding: &Binding, replace: bool) -> Result<(), CliError> {
    let folder = dir.join(DIRECTORY);
    match fs::symlink_metadata(&folder) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(invalid(".anywhere must be an ordinary directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&folder).map_err(|e| invalid(format!("cannot create .anywhere: {e}")))?
        }
        Err(error) => return Err(invalid(format!("cannot inspect .anywhere: {error}"))),
    }
    if let Some(existing) = read(dir)? {
        if existing == *binding {
            return Ok(());
        }
        if !replace {
            return Err(invalid(
                "the project already has a different binding; use `anywhere workspaces bind --replace` after inspecting the destination",
            ));
        }
    }
    let path = folder.join(FILE);
    let temporary = folder.join(format!("project-{}.tmp", Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| invalid(format!("cannot create the project binding: {e}")))?;
    let bytes = serde_json::to_vec_pretty(binding).map_err(|e| invalid(e.to_string()))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| invalid(format!("cannot save the project binding: {e}")))?;
    if replace {
        fs::rename(&temporary, &path)
            .map_err(|e| invalid(format!("cannot replace the project binding: {e}")))?;
    } else {
        fs::hard_link(&temporary, &path).map_err(|e| {
            invalid(format!(
                "cannot install the project binding without replacing another writer: {e}"
            ))
        })?;
        fs::remove_file(&temporary).map_err(|e| {
            invalid(format!(
                "the binding was saved, but its temporary file could not be removed: {e}"
            ))
        })?;
    }
    File::open(folder)
        .and_then(|file| file.sync_all())
        .map_err(|e| invalid(format!("cannot persist the project binding directory: {e}")))?;
    Ok(())
}
pub fn remove(dir: &Path) -> Result<(), CliError> {
    fs::remove_file(dir.join(DIRECTORY).join(FILE))
        .map_err(|e| invalid(format!("cannot remove the project binding: {e}")))
}
