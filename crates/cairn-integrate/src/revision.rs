//! The one canonical `skill_revision` algorithm (D29b).
//!
//! Everything that needs the number calls this module: the embedded metadata
//! validation, direct installation, `cairn doctor`, the `skillref` binary the
//! release workflow runs, and the release verification fetch. It is never
//! reimplemented in shell, YAML or a script — two implementations of one
//! number are guaranteed to drift.
//!
//! The interesting part is the circularity. `SKILL.md` carries
//! `metadata.cairn_skill_revision`, so hashing the file as it stands would
//! hash the value being computed. Before hashing, the *value* of that one
//! parsed frontmatter field is replaced with the literal `<REVISION>`. The
//! replacement is on the parsed field, not a text search, so a body line that
//! mentions the field name is untouched. `cairn_skill_schema` is hashed
//! normally: a schema change is a real change and must produce a new revision.

use include_dir::{include_dir, Dir};
use sha2::{Digest, Sha256};

/// The canonical Skill source, embedded so an installed Cairn always carries
/// the version it will write (`contracts/agent-contract.md`).
pub static SKILL_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills/cairn");

/// The placeholder the self-referential field is normalized to before hashing.
pub const REVISION_PLACEHOLDER: &str = "<REVISION>";

/// The frontmatter key holding the revision.
pub const REVISION_KEY: &str = "cairn_skill_revision";

/// The frontmatter key holding the schema.
pub const SCHEMA_KEY: &str = "cairn_skill_schema";

/// One file of the canonical tree: relative path plus normalized content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub content: String,
}

/// Normalize content: CRLF → LF, and exactly one trailing newline.
///
/// Cross-platform determinism is required because this number is computed on a
/// developer's machine, in CI, and on a user's machine, and all three must
/// agree.
pub fn normalize_content(raw: &str) -> String {
    let mut s = raw.replace("\r\n", "\n");
    while s.ends_with('\n') {
        s.pop();
    }
    s.push('\n');
    s
}

/// Replace the *value* of `metadata.cairn_skill_revision` inside the YAML
/// frontmatter with the placeholder.
///
/// Operates only within the frontmatter block and only on the key nested under
/// `metadata:`, so a body mention of the field name is left exactly as it is.
pub fn normalize_self_field(content: &str) -> String {
    let Some(fm) = Frontmatter::locate(content) else {
        return content.to_string();
    };
    let lines: Vec<&str> = content.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_metadata = false;
    let mut metadata_indent = 0usize;

    for (i, line) in lines.iter().enumerate() {
        if i <= fm.start || i >= fm.end {
            out.push((*line).to_string());
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        if in_metadata && !trimmed.is_empty() && indent <= metadata_indent {
            in_metadata = false;
        }
        if !in_metadata && trimmed.starts_with("metadata:") {
            in_metadata = true;
            metadata_indent = indent;
            out.push((*line).to_string());
            continue;
        }
        if in_metadata {
            if let Some(rest) = trimmed.strip_prefix(REVISION_KEY) {
                if let Some(_value) = rest.strip_prefix(':') {
                    out.push(format!(
                        "{}{REVISION_KEY}: {REVISION_PLACEHOLDER}",
                        " ".repeat(indent)
                    ));
                    continue;
                }
            }
        }
        out.push((*line).to_string());
    }
    out.join("\n")
}

/// The frontmatter delimiters, as line indices into a `\n`-split document.
struct Frontmatter {
    /// Index of the opening `---`.
    start: usize,
    /// Index of the closing `---`.
    end: usize,
}

impl Frontmatter {
    fn locate(content: &str) -> Option<Frontmatter> {
        let lines: Vec<&str> = content.split('\n').collect();
        if lines.first().map(|l| l.trim_end()) != Some("---") {
            return None;
        }
        let end = lines
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, l)| l.trim_end() == "---")
            .map(|(i, _)| i)?;
        Some(Frontmatter { start: 0, end })
    }
}

/// Read one scalar field from the `metadata:` map of a document's frontmatter.
pub fn metadata_field(content: &str, key: &str) -> Option<String> {
    let fm = Frontmatter::locate(content)?;
    let lines: Vec<&str> = content.split('\n').collect();
    let mut in_metadata = false;
    let mut metadata_indent = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if i <= fm.start || i >= fm.end {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if in_metadata && !trimmed.is_empty() && indent <= metadata_indent {
            in_metadata = false;
        }
        if !in_metadata && trimmed.starts_with("metadata:") {
            in_metadata = true;
            metadata_indent = indent;
            continue;
        }
        if in_metadata {
            if let Some(rest) = trimmed.strip_prefix(key) {
                if let Some(value) = rest.strip_prefix(':') {
                    return Some(
                        value
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string(),
                    );
                }
            }
        }
    }
    None
}

/// Collect the canonical file list from the embedded tree.
///
/// "Canonical" means normalized for hashing: the self-referential revision
/// field is replaced with the placeholder. That is right for computing the
/// digest and wrong for installing, which needs the source exactly as it is —
/// see `embedded_files_verbatim`.
pub fn embedded_files() -> Vec<SkillFile> {
    let mut files = Vec::new();
    collect(&SKILL_DIR, &mut files);
    finish(files)
}

/// The embedded tree exactly as checked in.
///
/// What installation writes. The installed `SKILL.md` must carry its real
/// `metadata.cairn_skill_revision`, both because the agents read the
/// frontmatter and because doctor compares the installed declaration against
/// the files it actually finds.
pub fn embedded_files_verbatim() -> Vec<SkillFile> {
    let mut files = Vec::new();
    collect(&SKILL_DIR, &mut files);
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    for f in &mut files {
        f.content = normalize_content(&f.content);
    }
    files
}

fn collect(dir: &Dir<'_>, out: &mut Vec<SkillFile>) {
    for f in dir.files() {
        let content = String::from_utf8_lossy(f.contents()).to_string();
        out.push(SkillFile {
            path: f.path().to_string_lossy().replace('\\', "/"),
            content,
        });
    }
    for d in dir.dirs() {
        collect(d, out);
    }
}

/// Collect the canonical file list from a directory on disk.
///
/// Used by `skillref` against a checkout and by the release verification
/// against the archive it fetched back.
pub fn files_from_disk(root: &std::path::Path) -> std::io::Result<Vec<SkillFile>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(finish(out))
}

fn walk(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<SkillFile>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else {
            // Nothing is excluded — a new reference file changes the revision,
            // which is the point (D29b step 1).
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(SkillFile {
                path: rel,
                content: String::from_utf8_lossy(&std::fs::read(&path)?).to_string(),
            });
        }
    }
    Ok(())
}

/// Sort by relative path as raw bytes and normalize every file.
fn finish(mut files: Vec<SkillFile>) -> Vec<SkillFile> {
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    for f in &mut files {
        f.content = normalize_content(&f.content);
        if f.path == "SKILL.md" {
            f.content = normalize_content(&normalize_self_field(&f.content));
        }
    }
    files
}

/// The revision of a canonical file list: first 12 lowercase hex characters of
/// the SHA-256 of the length-prefixed stream.
///
/// Length-prefixing removes any possibility of two different trees hashing
/// alike by concatenation: `a/b` + `c` and `a` + `/bc` cannot collide.
pub fn revision_of(files: &[SkillFile]) -> String {
    let mut h = Sha256::new();
    for f in files {
        h.update(f.path.as_bytes());
        h.update([0u8]);
        h.update((f.content.len() as u64).to_be_bytes());
        h.update(f.content.as_bytes());
    }
    hex::encode(h.finalize())[..12].to_string()
}

/// The schema of a canonical file list, read from `SKILL.md`'s frontmatter.
pub fn schema_of(files: &[SkillFile]) -> u32 {
    files
        .iter()
        .find(|f| f.path == "SKILL.md")
        .and_then(|f| metadata_field(&f.content, SCHEMA_KEY))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// The revision the running binary embeds.
pub fn embedded_revision() -> String {
    revision_of(&embedded_files())
}

/// The schema the running binary embeds.
pub fn embedded_schema() -> u32 {
    schema_of(&embedded_files())
}

/// The value written into the checked-in `SKILL.md`.
pub fn declared_revision() -> Option<String> {
    SKILL_DIR
        .get_file("SKILL.md")
        .map(|f| String::from_utf8_lossy(f.contents()).to_string())
        .and_then(|c| metadata_field(&c, REVISION_KEY))
}

/// The branch name that identifies this Skill *content* (D29a).
///
/// It names content, not a release: two Cairn releases that ship the same
/// Skill share one branch, and the branch keeps pointing at whichever commit
/// first introduced that content.
pub fn branch_name(schema: u32, revision: &str) -> String {
    format!("skill-release/{schema}-{revision}")
}

/// The embedded tree's branch name.
pub fn embedded_branch() -> String {
    branch_name(embedded_schema(), &embedded_revision())
}

/// Everything `skillref` prints, and everything the release job reads.
pub fn skillref_json(files: &[SkillFile]) -> serde_json::Value {
    let schema = schema_of(files);
    let revision = revision_of(files);
    serde_json::json!({
        "skill_schema": schema,
        "skill_revision": revision,
        "skill_branch": branch_name(schema, &revision),
    })
}

/// Read the installed Skill's own files back from disk and recompute its
/// revision.
///
/// Doctor uses this rather than trusting the installed metadata: a Skill whose
/// frontmatter claims one revision and whose files say another is exactly the
/// drift the comparison exists to catch (T045).
pub fn installed_revision(dir: &std::path::Path) -> std::io::Result<InstalledSkill> {
    let files = files_from_disk(dir)?;
    // The on-disk file still has its real value; the canonicalized copy does
    // not, so it is read again rather than recovered from the hashed form.
    let declared = std::fs::read_to_string(dir.join("SKILL.md"))
        .ok()
        .and_then(|raw| metadata_field(&raw, REVISION_KEY));
    Ok(InstalledSkill {
        schema: schema_of(&files),
        declared_revision: declared,
        computed_revision: revision_of(&files),
    })
}

/// What doctor compares against the embedded values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkill {
    pub schema: u32,
    /// What the installed `SKILL.md` claims.
    pub declared_revision: Option<String>,
    /// What its files actually hash to.
    pub computed_revision: String,
}

impl InstalledSkill {
    /// True where the installed content matches this build's embedded Skill.
    pub fn matches(&self, schema: u32, revision: &str) -> bool {
        self.schema == schema && self.computed_revision == revision
    }
    /// True where the installed metadata disagrees with the installed files —
    /// a tampered or partially updated installation.
    pub fn self_consistent(&self) -> bool {
        match &self.declared_revision {
            Some(d) => d == &self.computed_revision,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_in_revision_equals_the_computed_value() {
        // D29b validation: an ordinary `cargo test` catches a Skill edit that
        // forgot to update the frontmatter field.
        let computed = embedded_revision();
        let declared = declared_revision().expect("SKILL.md declares a revision");
        assert_eq!(
            declared, computed,
            "skills/cairn/SKILL.md metadata.{REVISION_KEY} is stale; set it to {computed}"
        );
    }

    #[test]
    fn the_self_field_is_normalized_and_a_body_mention_is_not() {
        let doc = "---\nname: cairn\nmetadata:\n  cairn_skill_schema: 1\n  cairn_skill_revision: abc123abc123\n---\n\nThe field cairn_skill_revision: abc123abc123 is documented here.\n";
        let out = normalize_self_field(doc);
        assert!(out.contains(&format!("  {REVISION_KEY}: {REVISION_PLACEHOLDER}")));
        assert!(
            out.contains("The field cairn_skill_revision: abc123abc123 is documented here."),
            "a body mention must be untouched"
        );
        // The schema is hashed normally: a schema change is a real change.
        assert!(out.contains("cairn_skill_schema: 1"));
    }

    #[test]
    fn changing_only_the_revision_value_does_not_change_the_digest() {
        // The circularity fix, stated as a property.
        let a = vec![SkillFile {
            path: "SKILL.md".into(),
            content: "---\nmetadata:\n  cairn_skill_revision: aaaaaaaaaaaa\n---\nbody\n".into(),
        }];
        let b = vec![SkillFile {
            path: "SKILL.md".into(),
            content: "---\nmetadata:\n  cairn_skill_revision: bbbbbbbbbbbb\n---\nbody\n".into(),
        }];
        assert_eq!(revision_of(&finish(a)), revision_of(&finish(b)));
    }

    #[test]
    fn changing_the_schema_does_change_the_digest() {
        let a = vec![SkillFile {
            path: "SKILL.md".into(),
            content: "---\nmetadata:\n  cairn_skill_schema: 1\n---\nbody\n".into(),
        }];
        let b = vec![SkillFile {
            path: "SKILL.md".into(),
            content: "---\nmetadata:\n  cairn_skill_schema: 2\n---\nbody\n".into(),
        }];
        assert_ne!(revision_of(&finish(a)), revision_of(&finish(b)));
    }

    #[test]
    fn length_prefixing_prevents_a_concatenation_collision() {
        // `a/b` + `c` must not hash the same as `a` + `/bc` (D29b step 5).
        let one = vec![SkillFile {
            path: "a/b".into(),
            content: "c".into(),
        }];
        let two = vec![SkillFile {
            path: "a".into(),
            content: "/bc".into(),
        }];
        assert_ne!(revision_of(&finish(one)), revision_of(&finish(two)));
    }

    #[test]
    fn crlf_and_trailing_newlines_do_not_change_the_digest() {
        let unix = vec![SkillFile {
            path: "r.md".into(),
            content: "one\ntwo\n".into(),
        }];
        let windows = vec![SkillFile {
            path: "r.md".into(),
            content: "one\r\ntwo\r\n\r\n\r\n".into(),
        }];
        assert_eq!(revision_of(&finish(unix)), revision_of(&finish(windows)));
    }

    #[test]
    fn a_new_reference_file_changes_the_revision() {
        let before = embedded_files();
        let mut after = before.clone();
        after.push(SkillFile {
            path: "references/new.md".into(),
            content: "x\n".into(),
        });
        assert_ne!(revision_of(&before), revision_of(&finish(after)));
    }

    #[test]
    fn the_branch_names_content_not_a_release() {
        // D29a: the name is derived from schema and revision only. Nothing
        // about a Cairn version can appear in it.
        let b = branch_name(1, "c07d4419b2ae");
        assert_eq!(b, "skill-release/1-c07d4419b2ae");
        assert!(!b.contains("alpha"));
    }

    #[test]
    fn the_digest_is_twelve_lowercase_hex_characters() {
        let r = embedded_revision();
        assert_eq!(r.len(), 12);
        assert!(r
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn skillref_output_is_the_same_function_the_binary_uses() {
        let v = skillref_json(&embedded_files());
        assert_eq!(v["skill_revision"], embedded_revision());
        assert_eq!(v["skill_schema"], embedded_schema());
        assert_eq!(v["skill_branch"], embedded_branch());
    }

    #[test]
    fn ordering_is_by_raw_path_bytes_not_locale() {
        let files = finish(vec![
            SkillFile {
                path: "references/z.md".into(),
                content: "z\n".into(),
            },
            SkillFile {
                path: "SKILL.md".into(),
                content: "s\n".into(),
            },
            SkillFile {
                path: "references/a.md".into(),
                content: "a\n".into(),
            },
        ]);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["SKILL.md", "references/a.md", "references/z.md"]
        );
    }
}
