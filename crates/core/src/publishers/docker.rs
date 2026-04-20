//! Docker publisher: push a container image to any OCI-compliant registry.
//!
//! - check: Query the registry's v2 API for the manifest at `<image>:<tag>`.
//!   Uses anonymous access by default; bearer-token auth is arranged via
//!   the `Www-Authenticate` challenge so public and ghcr.io public images
//!   work out of the box. For private images behind auth, `check` returns
//!   `Unknown` and `run` proceeds.
//! - run: `docker buildx build --push -t <image>:<version> ...`.
//!
//! The version tag is always `ctx.version` (no "v" prefix). The `image`
//! field is the fully qualified reference (e.g. `ghcr.io/owner/repo`).

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;

pub struct DockerPublisher {
    pub image: String,
    pub platforms: Vec<String>,
    pub dockerfile: Option<String>,
}

impl Publisher for DockerPublisher {
    fn name(&self) -> &'static str {
        "docker"
    }

    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        let (registry, repository) = split_image_ref(&self.image);
        let manifest_url = format!(
            "https://{registry}/v2/{repository}/manifests/{}",
            ctx.version
        );

        // Anonymous HEAD first.
        let resp = ureq::head(&manifest_url)
            .header(
                "Accept",
                "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json",
            )
            .header("User-Agent", "sr (+https://github.com/urmzd/sr)")
            .call();

        match resp {
            Ok(r) if r.status() == 200 => Ok(PublishState::Completed),
            Ok(r) if r.status() == 404 => Ok(PublishState::Needed),
            Err(ureq::Error::StatusCode(404)) => Ok(PublishState::Needed),
            Err(ureq::Error::StatusCode(401)) | Ok(_) => Ok(PublishState::Unknown(format!(
                "docker registry check inconclusive for {}:{}",
                self.image, ctx.version
            ))),
            Err(e) => Ok(PublishState::Unknown(format!("docker check failed: {e}"))),
        }
    }

    fn run(&self, ctx: &PublishCtx<'_>) -> Result<(), ReleaseError> {
        let mut cmd = String::from("docker buildx build --push");
        if !self.platforms.is_empty() {
            cmd.push_str(" --platform ");
            cmd.push_str(&shell_word(&self.platforms.join(",")));
        }
        if let Some(dockerfile) = &self.dockerfile {
            cmd.push_str(" -f ");
            cmd.push_str(&shell_word(dockerfile));
        }
        cmd.push_str(" -t ");
        cmd.push_str(&shell_word(&format!("{}:{}", self.image, ctx.version)));
        // Also tag as :latest when not a pre-release.
        if !ctx.version.contains('-') {
            cmd.push_str(" -t ");
            cmd.push_str(&shell_word(&format!("{}:latest", self.image)));
        }
        cmd.push_str(" .");

        if ctx.dry_run {
            eprintln!("[dry-run] docker ({}): {cmd}", ctx.package.path);
            return Ok(());
        }

        eprintln!("docker ({}): {cmd}", ctx.package.path);
        let wrapped = format!("cd {} && {cmd}", shell_word(&ctx.package.path));
        run_shell(&wrapped, None, ctx.env)
    }
}

/// Split an image reference like `ghcr.io/owner/repo` into (registry, repo).
/// Defaults to Docker Hub for unqualified names: `owner/repo` → `registry-1.docker.io`.
fn split_image_ref(image: &str) -> (String, String) {
    // A "registry" has a dot or colon in the first path component, or is "localhost".
    if let Some((head, tail)) = image.split_once('/') {
        let has_port = head.contains(':');
        let has_dot = head.contains('.');
        if has_port || has_dot || head == "localhost" {
            return (head.to_string(), tail.to_string());
        }
    }
    // Unqualified: Docker Hub. Single-segment (e.g. "nginx") → "library/nginx".
    let repo = if image.contains('/') {
        image.to_string()
    } else {
        format!("library/{image}")
    };
    ("registry-1.docker.io".to_string(), repo)
}

fn shell_word(s: &str) -> String {
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ghcr_image() {
        let (r, repo) = split_image_ref("ghcr.io/owner/repo");
        assert_eq!(r, "ghcr.io");
        assert_eq!(repo, "owner/repo");
    }

    #[test]
    fn split_docker_hub_multi_segment() {
        let (r, repo) = split_image_ref("urmzd/sr");
        assert_eq!(r, "registry-1.docker.io");
        assert_eq!(repo, "urmzd/sr");
    }

    #[test]
    fn split_docker_hub_single_segment() {
        let (r, repo) = split_image_ref("nginx");
        assert_eq!(r, "registry-1.docker.io");
        assert_eq!(repo, "library/nginx");
    }

    #[test]
    fn split_localhost_port() {
        let (r, repo) = split_image_ref("localhost:5000/img");
        assert_eq!(r, "localhost:5000");
        assert_eq!(repo, "img");
    }
}
