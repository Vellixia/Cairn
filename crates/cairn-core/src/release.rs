//! Knowing whether a newer Cairn exists, and where to get it.
//!
//! Only the reasoning lives here — parsing a release, comparing versions,
//! naming an archive. Fetching is the caller's business, because the server and
//! the CLI reach the network on very different terms.

use serde::{Deserialize, Serialize};

/// Where releases are published.
///
/// The list, not `/releases/latest`: that endpoint excludes prereleases, so
/// while Cairn has only ever published alphas it answers 404 and every client
/// concludes there are no releases at all.
pub const RELEASES_API: &str = "https://api.github.com/repos/Vellixia/Cairn/releases?per_page=20";
/// Where a human should be sent to read about one.
pub const RELEASES_PAGE: &str = "https://github.com/Vellixia/Cairn/releases";

/// A published release, reduced to what an updater needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Release {
    /// The tag as published, e.g. `v0.1.0-alpha.2`.
    pub tag: String,
    /// The tag without its leading `v`, which is what versions compare as.
    pub version: String,
    /// The release page, for someone who would rather read than run.
    pub url: String,
}

fn one(v: &serde_json::Value) -> Option<Release> {
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    let url = v
        .get("html_url")
        .and_then(|u| u.as_str())
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    Some(Release { tag, version, url })
}

/// The newest release worth offering to someone on `current`.
///
/// Drafts are never offered — nobody published them. Prereleases are offered
/// only to someone already running one: an alpha user wants the next alpha,
/// while pulling a stable install onto an alpha would be a downgrade in
/// everything but version number.
///
/// The list arrives newest-first, but that ordering is GitHub's by creation
/// date, so versions are compared rather than trusted.
pub fn pick_release(body: &str, current: &str) -> Option<Release> {
    let list: Vec<serde_json::Value> = serde_json::from_str(body).ok()?;
    let on_prerelease = semver::Version::parse(current.trim_start_matches('v'))
        .map(|v| !v.pre.is_empty())
        .unwrap_or(false);

    list.iter()
        .filter(|r| r.get("draft").and_then(|d| d.as_bool()) != Some(true))
        .filter(|r| on_prerelease || r.get("prerelease").and_then(|p| p.as_bool()) != Some(true))
        .filter_map(one)
        .filter(|r| semver::Version::parse(&r.version).is_ok())
        .max_by(|a, b| {
            let a = semver::Version::parse(&a.version).expect("filtered");
            let b = semver::Version::parse(&b.version).expect("filtered");
            a.cmp(&b)
        })
}

/// Whether `latest` is a version worth moving to from `current`.
///
/// Prereleases order below the release they lead to, so `0.1.0-alpha.2` beats
/// `0.1.0-alpha.1` and `0.1.0` beats both. A version that will not parse is
/// never treated as newer: guessing would offer people upgrades that do not
/// exist.
pub fn update_available(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current.trim_start_matches('v')),
        semver::Version::parse(latest.trim_start_matches('v')),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

/// The build of Cairn this binary belongs to, as the release names it.
///
/// `None` on a platform Cairn publishes no archive for — Windows today — which
/// the updater reports rather than guessing at a download that is not there.
pub fn target_triple() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Some("x86_64-unknown-linux-gnu"),
        ("aarch64", "linux") => Some("aarch64-unknown-linux-gnu"),
        ("aarch64", "macos") => Some("aarch64-apple-darwin"),
        ("x86_64", "macos") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

/// The archive a release publishes for one platform.
pub fn archive_name(version: &str, target: &str) -> String {
    format!("cairn-v{version}-{target}.tar.gz")
}

/// A release asset's download URL.
pub fn asset_url(tag: &str, file: &str) -> String {
    format!("https://github.com/Vellixia/Cairn/releases/download/{tag}/{file}")
}

/// Find one archive's expected digest in a `SHA256SUMS` file.
///
/// The file lists `<digest>  <name>` per line. A missing entry is a refusal to
/// install, never a reason to skip the check.
pub fn expected_digest(sums: &str, archive: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let name = parts.next()?;
        (name.trim_start_matches('*') == archive).then(|| digest.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_prerelease_is_an_update() {
        assert!(update_available("0.1.0-alpha.1", "0.1.0-alpha.2"));
        assert!(update_available("0.1.0-alpha.2", "0.1.0"));
        assert!(!update_available("0.1.0", "0.1.0-alpha.3"));
        assert!(!update_available("0.1.0-alpha.1", "0.1.0-alpha.1"));
    }

    #[test]
    fn a_leading_v_is_tolerated_on_either_side() {
        assert!(update_available("v0.1.0", "v0.2.0"));
    }

    #[test]
    fn an_unparseable_version_never_offers_an_update() {
        assert!(!update_available("0.1.0", "not-a-version"));
        assert!(!update_available("nightly", "0.2.0"));
    }

    const LIST: &str = r#"[
        {"tag_name":"v0.2.0-alpha.1","html_url":"https://example.test/a","draft":false,"prerelease":true},
        {"tag_name":"v0.1.0","html_url":"https://example.test/s","draft":false,"prerelease":false},
        {"tag_name":"v9.9.9","html_url":"https://example.test/d","draft":true,"prerelease":false}
    ]"#;

    #[test]
    fn a_draft_is_never_offered() {
        let r = pick_release(LIST, "0.1.0-alpha.1").expect("picked");
        assert_ne!(r.version, "9.9.9", "a draft nobody published was offered");
    }

    #[test]
    fn a_prerelease_user_is_offered_the_next_prerelease() {
        let r = pick_release(LIST, "0.1.0-alpha.1").expect("picked");
        assert_eq!(r.version, "0.2.0-alpha.1");
    }

    #[test]
    fn a_stable_user_is_not_dragged_onto_an_alpha() {
        let r = pick_release(LIST, "0.1.0").expect("picked");
        assert_eq!(r.version, "0.1.0", "only the stable release was eligible");
        assert!(!update_available("0.1.0", &r.version));
    }

    #[test]
    fn the_newest_version_wins_regardless_of_list_order() {
        let body = r#"[
            {"tag_name":"v0.1.0","html_url":"u","draft":false,"prerelease":false},
            {"tag_name":"v0.3.0","html_url":"u","draft":false,"prerelease":false},
            {"tag_name":"v0.2.0","html_url":"u","draft":false,"prerelease":false}
        ]"#;
        assert_eq!(pick_release(body, "0.1.0").unwrap().version, "0.3.0");
    }

    #[test]
    fn a_digest_is_found_by_archive_name_only() {
        let sums = "abc123  cairn-v0.2.0-aarch64-apple-darwin.tar.gz\n\
                    def456  cairn-v0.2.0-x86_64-apple-darwin.tar.gz\n";
        assert_eq!(
            expected_digest(sums, "cairn-v0.2.0-x86_64-apple-darwin.tar.gz").as_deref(),
            Some("def456")
        );
        assert_eq!(
            expected_digest(sums, "cairn-v0.2.0-not-published.tar.gz"),
            None
        );
    }
}
