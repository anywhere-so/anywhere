//! Exact public app namespaces. Host names route requests; they grant no authority.
use uuid::Uuid;

/// A canonical, exact HTTPS audience, without a trailing slash or default port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin(String);

impl Origin {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > 320 {
            return Err(Error);
        }
        let uri: http::Uri = text.parse().map_err(|_| Error)?;
        let authority = uri.authority().ok_or(Error)?;
        let host = authority.host();
        let valid_name = !host.is_empty()
            && host.len() <= 253
            && host.parse::<std::net::IpAddr>().is_err()
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            });
        let canonical = match authority.port_u16() {
            Some(0 | 443) => return Err(Error),
            Some(port) => format!("https://{host}:{port}"),
            None => format!("https://{host}"),
        };
        if !valid_name || uri.scheme_str() != Some("https") || text != canonical {
            return Err(Error);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Namespace {
    origin: Origin,
    host: String,
    google: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    GoogleApp(Uuid),
    Address(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("use the exact configured HTTPS apps namespace and a canonical app authority")]
pub struct Error;
impl Error {
    pub fn code(self) -> &'static str {
        "public_apps_namespace_invalid"
    }
}

impl Namespace {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let origin = Origin::parse(text).map_err(|_| Error)?;
        let uri: http::Uri = text.parse().map_err(|_| Error)?;
        let host = uri.host().ok_or(Error)?;
        // A malformed Google namespace must never acquire wildcard semantics.
        let google = host == "appspot.com" || host.ends_with(".appspot.com");
        if google {
            let labels: Vec<_> = host.split('.').collect();
            if labels.len() != 5 || labels[2..] != ["r", "appspot", "com"] {
                return Err(Error);
            }
            let project = labels[0];
            let region = labels[1];
            // app-<32 hex>-dot-<project> must fit in one 63-byte DNS label.
            if !(6..=22).contains(&project.len())
                || !project.as_bytes()[0].is_ascii_lowercase()
                || project.contains("-dot-")
                || !region.as_bytes()[0].is_ascii_lowercase()
                || region.contains("-dot-")
                || uri.port().is_some()
            {
                return Err(Error);
            }
        }
        let namespace = Self {
            origin,
            host: host.to_owned(),
            google,
        };
        Ok(namespace)
    }

    pub fn as_str(&self) -> &str {
        self.origin.as_str()
    }
    pub fn authority(&self) -> &str {
        // Origin checked this exact prefix and the constructor is private.
        &self.as_str()["https://".len()..]
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn is_google(&self) -> bool {
        self.google
    }
    /// Whether each app is a browser site of its own: under Google's aliases,
    /// which replace the project label, under a namespace on the Public Suffix
    /// List, and under localhost for local development. Under any other
    /// namespace every app shares its registrable domain as one site.
    pub fn isolated_sites(&self) -> bool {
        if matches!(self.host.as_str(), "localhost" | "test") {
            return true;
        }
        let suffix = if self.google {
            let Some((_, suffix)) = self.host.split_once('.') else {
                return false;
            };
            suffix
        } else {
            &self.host
        };
        psl::suffix(suffix.as_bytes())
            .is_some_and(|found| found.is_known() && found.as_bytes() == suffix.as_bytes())
    }

    /// Whether a platform/API host lives inside an app's browser site.
    pub fn contains_app_site(&self, host: &str) -> bool {
        let authority = |host: &str| format!("{host}{}", &self.authority()[self.host.len()..]);
        if self.parse_authority(&authority(host)).is_ok() {
            return true;
        }
        if !self.isolated_sites() {
            // Every app's site is then the namespace's registrable domain.
            return psl::domain_str(host)
                .is_some_and(|site| psl::domain_str(&self.host) == Some(site));
        }
        let site = if self.host == "localhost" {
            let Some(prefix) = host.strip_suffix(".localhost") else {
                return false;
            };
            let label = prefix.rsplit('.').next().unwrap_or("");
            format!("{label}.localhost")
        } else {
            let Some(domain) = psl::domain_str(host) else {
                return false;
            };
            domain.to_owned()
        };
        self.parse_authority(&authority(&site)).is_ok()
    }
    /// The origin before an app has an ID. Google's aliases need that ID,
    /// so their origin is unknown until the first deploy.
    pub fn address_origin(&self, address: &str) -> Result<Option<Origin>, Error> {
        if !valid_address(address) {
            return Err(Error);
        }
        if self.google {
            return Ok(None);
        }
        Origin::parse(&format!("https://{address}.{}", self.authority())).map(Some)
    }
    pub fn app_origin(&self, app: Uuid, address: &str) -> Result<Origin, Error> {
        if app.is_nil() {
            return Err(Error);
        }
        match self.address_origin(address)? {
            Some(origin) => Ok(origin),
            None => Origin::parse(&format!(
                "https://app-{}-dot-{}",
                app.simple(),
                self.authority()
            )),
        }
    }
    pub fn parse_authority<'a>(&self, authority: &'a str) -> Result<Target<'a>, Error> {
        // Checks canonical DNS and exact port spelling before interpreting aliases.
        Origin::parse(&format!("https://{authority}")).map_err(|_| Error)?;
        let prefix = authority.strip_suffix(self.authority()).ok_or(Error)?;
        if self.google {
            let id = prefix
                .strip_prefix("app-")
                .and_then(|v| v.strip_suffix("-dot-"))
                .ok_or(Error)?;
            if id.len() != 32
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(Error);
            }
            let id = Uuid::parse_str(id).map_err(|_| Error)?;
            if id.is_nil() {
                return Err(Error);
            }
            Ok(Target::GoogleApp(id))
        } else {
            let label = prefix.strip_suffix('.').ok_or(Error)?;
            if valid_address(label) {
                Ok(Target::Address(label))
            } else {
                Err(Error)
            }
        }
    }
}

// The existing <username>-<app> DNS contract, without adding a naming dependency.
fn valid_address(address: &str) -> bool {
    let Some((user, app)) = address.split_once('-') else {
        return false;
    };
    address.len() <= 63
        && !user.is_empty()
        && !app.is_empty()
        && user
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !app.starts_with('-')
        && !app.ends_with('-')
        && app
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests;
