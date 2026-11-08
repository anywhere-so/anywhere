//! An image app's `image`: a reference to a public container image, such as
//! `ghcr.io/every-app/open-seo:latest` or `nginx:alpine`, which the app's
//! sandbox pulls at each deploy. The CLI and the
//! control plane read it with [`ImageRef::parse`], and the helper that pulls it
//! reads it again, so the grammar is strict: a subset of Docker's, with no
//! uppercase names, no `localhost` and no bare IP addresses, since a sandbox
//! reaches only the internet.

use std::fmt;

/// The longest reference read, digest included.
pub const MAX_REFERENCE_LEN: usize = 512;

/// The registry a reference with no registry names, as Docker reads it.
pub const DOCKER_HUB: &str = "docker.io";

/// Where Docker Hub's registry API answers, which isn't `docker.io` itself.
const DOCKER_HUB_API: &str = "registry-1.docker.io";

/// A checked reference to an image: a registry, a repository, and a tag, a
/// digest, or both, in which case the digest decides what's pulled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// The registry's host, lowercase, with its port if it has one, such as
    /// `ghcr.io` or `docker.io`.
    pub registry: String,
    /// The repository's path in the registry, such as `every-app/open-seo` or,
    /// on Docker Hub, `library/nginx`.
    pub repository: String,
    /// The tag, such as `latest`: `latest` when the reference gives neither a
    /// tag nor a digest.
    pub tag: Option<String>,
    /// The digest, `sha256:` and 64 lowercase hex characters.
    pub digest: Option<String>,
}

/// Why an image reference was refused. Each message says how to fix it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageRefError {
    #[error("image is empty; name the image to run, such as image = \"ghcr.io/owner/app:latest\"")]
    Empty,
    #[error(
        "image is {len} characters long written in full, with its registry and tag; keep it under {MAX_REFERENCE_LEN}"
    )]
    TooLong { len: usize },
    #[error(
        "image {reference:?} isn't an image reference: {reason}; write it as Docker does, such as image = \"ghcr.io/owner/app:1.2\" or image = \"nginx:alpine\""
    )]
    Invalid { reference: String, reason: String },
    #[error(
        "image {reference:?} names the registry {registry:?}, which a sandbox can't reach, since sandboxes reach only the internet; push the image to a public registry, such as ghcr.io or Docker Hub"
    )]
    UnreachableRegistry { reference: String, registry: String },
}

impl ImageRef {
    /// Reads a reference as Docker does: `[registry/]repository[:tag][@digest]`.
    /// A first part with a `.` or a `:` is a registry; with none, the registry
    /// is Docker Hub, where a one-part name is under `library/`.
    pub fn parse(reference: &str) -> Result<ImageRef, ImageRefError> {
        if reference.is_empty() {
            return Err(ImageRefError::Empty);
        }
        let len = reference.chars().count();
        if len > MAX_REFERENCE_LEN {
            return Err(ImageRefError::TooLong { len });
        }
        let invalid = |reason: &str| ImageRefError::Invalid {
            reference: reference.to_owned(),
            reason: reason.to_owned(),
        };

        let (rest, digest) = match reference.split_once('@') {
            Some((rest, digest)) => {
                let hex = digest.strip_prefix("sha256:").ok_or_else(|| {
                    invalid("its digest must be sha256: and 64 lowercase hex characters")
                })?;
                if !manifest::is_hash(hex) {
                    return Err(invalid(
                        "its digest must be sha256: and 64 lowercase hex characters",
                    ));
                }
                (rest, Some(digest.to_owned()))
            }
            None => (reference, None),
        };

        // A tag follows the last `:` after the last `/`; a `:` before that is
        // a registry's port.
        let last_slash = rest.rfind('/').map_or(0, |at| at + 1);
        let (name, tag) = match rest[last_slash..].find(':') {
            Some(at) => {
                let at = last_slash + at;
                let tag = &rest[at + 1..];
                if !is_tag(tag) {
                    return Err(invalid(
                        "its tag must be 1 to 128 letters, digits, _, . or -, not starting with . or -",
                    ));
                }
                (&rest[..at], Some(tag.to_owned()))
            }
            None => (rest, None),
        };

        let (registry, path) = match name.split_once('/') {
            Some((first, path)) if first.contains(['.', ':']) || first == "localhost" => {
                (registry(reference, first)?, path)
            }
            _ => (DOCKER_HUB.to_owned(), name),
        };
        if path.is_empty() || !path.split('/').all(is_path_component) {
            return Err(invalid(
                "its repository must be lowercase letters and digits, in parts split by /, each joined inside by ., _, __ or -",
            ));
        }
        let repository = if registry == DOCKER_HUB && !path.contains('/') {
            format!("library/{path}")
        } else {
            path.to_owned()
        };
        let tag = match (tag, &digest) {
            (None, None) => Some("latest".to_owned()),
            (tag, _) => tag,
        };
        let image = ImageRef {
            registry,
            repository,
            tag,
            digest,
        };
        // Written out in full, with Docker Hub's name and `latest` added, the
        // reference must still read back.
        let len = image.to_string().chars().count();
        if len > MAX_REFERENCE_LEN {
            return Err(ImageRefError::TooLong { len });
        }
        Ok(image)
    }

    /// The host the registry's API answers on: Docker Hub's is
    /// `registry-1.docker.io`, and every other registry's is its own.
    pub fn api_host(&self) -> &str {
        if self.registry == DOCKER_HUB {
            DOCKER_HUB_API
        } else {
            &self.registry
        }
    }

    /// What to ask the registry for: the digest if there is one, else the tag.
    pub fn reference(&self) -> &str {
        match (&self.digest, &self.tag) {
            (Some(digest), _) => digest,
            (None, Some(tag)) => tag,
            // `parse` gives every reference a tag or a digest.
            (None, None) => "latest",
        }
    }
}

/// The reference written out in full, such as
/// `docker.io/library/nginx:alpine`, which reads back as the same.
impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.registry, self.repository)?;
        if let Some(tag) = &self.tag {
            write!(f, ":{tag}")?;
        }
        if let Some(digest) = &self.digest {
            write!(f, "@{digest}")?;
        }
        Ok(())
    }
}

/// A registry: a host of DNS labels, at least two, and maybe a port. Hosts a
/// sandbox can never reach, `localhost` and IP addresses, are refused.
fn registry(reference: &str, registry: &str) -> Result<String, ImageRefError> {
    let unreachable = || ImageRefError::UnreachableRegistry {
        reference: reference.to_owned(),
        registry: registry.to_owned(),
    };
    let invalid = || ImageRefError::Invalid {
        reference: reference.to_owned(),
        reason: format!("its registry {registry:?} isn't a host name, with a port or without"),
    };
    let (host, port) = match registry.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (registry, None),
    };
    if let Some(port) = port
        && (port.starts_with('0') || !matches!(port.parse::<u16>(), Ok(port) if port != 0))
    {
        return Err(invalid());
    }
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(unreachable());
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || !labels.iter().all(|label| is_label(label)) {
        return Err(invalid());
    }
    // A name of digits and dots alone is an IPv4 address.
    if labels
        .iter()
        .all(|label| label.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(unreachable());
    }
    Ok(match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// A DNS label: letters, digits and `-`, 1 to 63 of them, with no `-` at
/// either end.
fn is_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/// A tag: a letter, digit or `_`, then up to 127 of those, `.` or `-`.
fn is_tag(tag: &str) -> bool {
    let mut bytes = tag.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphanumeric() || b == b'_')
        && tag.len() <= 128
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

/// One part of a repository's path: lowercase letters and digits, joined
/// inside by one `.`, one `_`, two `_`, or any number of `-`.
fn is_path_component(part: &str) -> bool {
    let bytes = part.as_bytes();
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if bytes.is_empty() || !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) {
        return false;
    }
    let mut at = 0;
    while at < bytes.len() {
        if alnum(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && !alnum(bytes[at]) {
            at += 1;
        }
        let separator = &part[start..at];
        let ok = matches!(separator, "." | "_" | "__") || separator.bytes().all(|b| b == b'-');
        if !ok {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:5b300ddde698a02895810754b986e5854db4aaef37d4163b8f2f4aedcc18be84";

    fn parsed(reference: &str) -> ImageRef {
        ImageRef::parse(reference).unwrap()
    }

    #[test]
    fn reads_a_registry_repository_and_tag() {
        let image = parsed("ghcr.io/every-app/open-seo:latest");
        assert_eq!(image.registry, "ghcr.io");
        assert_eq!(image.repository, "every-app/open-seo");
        assert_eq!(image.tag.as_deref(), Some("latest"));
        assert_eq!(image.digest, None);
        assert_eq!(image.api_host(), "ghcr.io");
        assert_eq!(image.reference(), "latest");
    }

    #[test]
    fn a_name_alone_is_docker_hubs_official_image_at_latest() {
        let image = parsed("nginx");
        assert_eq!(image.to_string(), "docker.io/library/nginx:latest");
        assert_eq!(image.api_host(), "registry-1.docker.io");
        assert_eq!(
            parsed("nginx:alpine").to_string(),
            "docker.io/library/nginx:alpine"
        );
        assert_eq!(
            parsed("grafana/grafana").to_string(),
            "docker.io/grafana/grafana:latest"
        );
    }

    #[test]
    fn a_digest_decides_what_is_pulled() {
        let image = parsed(&format!("ghcr.io/a/b:1.0@{DIGEST}"));
        assert_eq!(image.reference(), DIGEST);
        assert_eq!(image.tag.as_deref(), Some("1.0"));
        let image = parsed(&format!("ghcr.io/a/b@{DIGEST}"));
        assert_eq!(image.tag, None);
        assert_eq!(image.to_string(), format!("ghcr.io/a/b@{DIGEST}"));
    }

    #[test]
    fn a_registry_may_have_a_port_and_is_lowercased() {
        let image = parsed("Registry.Example.com:5000/team/app:v2");
        assert_eq!(image.registry, "registry.example.com:5000");
        assert_eq!(image.to_string(), "registry.example.com:5000/team/app:v2");
    }

    #[test]
    fn what_it_writes_reads_back_the_same() {
        for reference in [
            "nginx",
            "quay.io/libpod/alpine_nginx:latest",
            "ghcr.io/a/b__c/d-e.f:1_2.3-4",
            &format!("docker.io/library/redis:7@{DIGEST}"),
        ] {
            let image = parsed(reference);
            assert_eq!(parsed(&image.to_string()), image);
        }
    }

    #[test]
    fn refuses_registries_a_sandbox_cant_reach() {
        for reference in [
            "localhost:5000/app",
            "localhost/app",
            "10.0.0.1/app",
            "127.0.0.1:5000/a",
        ] {
            assert!(
                matches!(
                    ImageRef::parse(reference),
                    Err(ImageRefError::UnreachableRegistry { .. })
                ),
                "{reference}"
            );
        }
    }

    #[test]
    fn refuses_what_docker_would_refuse() {
        for reference in [
            "",
            "Nginx",
            "nginx:",
            "nginx:-tag",
            "nginx@sha256:abc",
            "nginx@md5:00",
            "ghcr.io/",
            "ghcr.io//app",
            "ghcr.io/app/",
            "ghcr.io/a..b",
            "ghcr.io/a___b",
            "ghcr.io/-a",
            "ghcr.io:0/a",
            "ghcr.io:99999/a",
            "-bad.io/a",
            "ghcr.io/a b",
            "ghcr.io/a:b:c",
        ] {
            assert!(ImageRef::parse(reference).is_err(), "{reference:?}");
        }
        // Under the limit as written, but over it in full, with
        // `docker.io/library/` and `:latest` added.
        let short = "a".repeat(MAX_REFERENCE_LEN - 4);
        assert!(matches!(
            ImageRef::parse(&short),
            Err(ImageRefError::TooLong { .. })
        ));
        let long = format!("ghcr.io/{}", "a".repeat(MAX_REFERENCE_LEN));
        assert!(matches!(
            ImageRef::parse(&long),
            Err(ImageRefError::TooLong { .. })
        ));
    }
}
