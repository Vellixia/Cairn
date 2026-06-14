//! AST-based file outlines: render a code file as just its structure — the signatures of its
//! top-level items and their members — so a 1000-line file costs a handful of tokens instead of
//! a thousand lines. Lossless as always: the full original is retained in the blob store and one
//! `expand` away.
//!
//! Backed by [`tree_sitter`] (real parsing, not regex/heuristics). Today: Rust. The [`LangSpec`]
//! table makes adding a language a matter of listing its node kinds — no new logic.

use std::path::Path;
use tree_sitter::{Language, Node, Parser};

/// The structural view of a file: a signature outline.
pub struct Outline {
    /// Language name (for the human-facing note), e.g. `"rust"`.
    pub lang: &'static str,
    /// How many signatures we emitted (top-level + nested members).
    pub items: usize,
    /// The outline text — one signature per line, members indented under their container.
    pub text: String,
}

/// Per-language description of which AST nodes are signatures and where their bodies begin.
struct LangSpec {
    language: Language,
    name: &'static str,
    /// Node kinds whose header we emit as a one-line signature.
    sig_kinds: &'static [&'static str],
    /// Of those, the kinds we descend into for nested signatures (impl/trait/mod bodies).
    container_kinds: &'static [&'static str],
    /// Child kinds that mark the start of a "body" — the signature is cut off right before it.
    body_kinds: &'static [&'static str],
}

/// Longest signature we keep before truncating (guards against pathological macros / where-clauses).
const MAX_SIG: usize = 200;

fn rust_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_rust::LANGUAGE.into(),
        name: "rust",
        sig_kinds: &[
            "function_item",
            "function_signature_item",
            "struct_item",
            "enum_item",
            "union_item",
            "trait_item",
            "impl_item",
            "mod_item",
            "type_item",
            "const_item",
            "static_item",
            "macro_definition",
            "associated_type",
        ],
        container_kinds: &["impl_item", "trait_item", "mod_item"],
        // Note: tuple-struct fields (`ordered_field_declaration_list`) are intentionally *not*
        // here — they're part of the type's identity, so we keep `struct P(i32, i32);` whole.
        body_kinds: &[
            "block",
            "declaration_list",
            "field_declaration_list",
            "enum_variant_list",
        ],
    }
}

fn spec_for_path(path: &Path) -> Option<LangSpec> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(rust_spec()),
        _ => None,
    }
}

/// Whether Cairn can produce a signature outline for this path's language.
pub fn supported(path: &Path) -> bool {
    spec_for_path(path).is_some()
}

/// Outline `source` (the contents of `path`) as a signature map. `with_lines` prefixes each
/// signature with its 1-based start line (`map` mode); without it you get bare signatures.
/// Returns `None` for unsupported languages or unparseable input — callers fall back to a full read.
pub fn outline(path: &Path, source: &str, with_lines: bool) -> Option<Outline> {
    let spec = spec_for_path(path)?;
    let mut parser = Parser::new();
    parser.set_language(&spec.language).ok()?;
    let tree = parser.parse(source, None)?;
    let bytes = source.as_bytes();

    let mut text = String::new();
    let mut items = 0usize;
    walk(
        tree.root_node(),
        bytes,
        &spec,
        0,
        with_lines,
        &mut text,
        &mut items,
    );

    if items == 0 {
        // Nothing structural to show (e.g. a file of only statements) — let the caller fall back.
        return None;
    }
    Some(Outline {
        lang: spec.name,
        items,
        text,
    })
}

/// Emit signatures for every item directly under `node`, descending into container bodies.
fn walk(
    node: Node,
    bytes: &[u8],
    spec: &LangSpec,
    depth: usize,
    with_lines: bool,
    out: &mut String,
    count: &mut usize,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if !spec.sig_kinds.contains(&kind) {
            continue;
        }
        let sig = signature_text(child, bytes, spec);
        for _ in 0..depth {
            out.push_str("    ");
        }
        if with_lines {
            out.push_str(&(child.start_position().row + 1).to_string());
            out.push_str(": ");
        }
        out.push_str(&sig);
        out.push('\n');
        *count += 1;

        if spec.container_kinds.contains(&kind) {
            if let Some(body) = body_node(child, spec) {
                walk(body, bytes, spec, depth + 1, with_lines, out, count);
            }
        }
    }
}

/// The header text of `node`, cut off right before its body and collapsed to a single line.
fn signature_text(node: Node, bytes: &[u8], spec: &LangSpec) -> String {
    let end = body_node(node, spec)
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let raw = &bytes[node.start_byte()..end.max(node.start_byte())];
    let collapsed = collapse_ws(&String::from_utf8_lossy(raw));
    // A dangling opening brace can be left when we cut just before a `{ … }` body.
    let mut sig = collapsed
        .trim_end_matches(|c: char| c == '{' || c.is_whitespace())
        .to_string();
    if sig.chars().count() > MAX_SIG {
        sig = sig.chars().take(MAX_SIG).collect::<String>();
        sig.push_str(" …");
    }
    sig
}

/// The first child of `node` whose kind marks the start of a body.
fn body_node<'a>(node: Node<'a>, spec: &LangSpec) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| spec.body_kinds.contains(&c.kind()));
    found
}

/// Collapse every run of whitespace (incl. newlines) to a single space, trimmed.
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
//! A module.
use std::fmt;

/// A point.
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub struct Pair(i32, i32);

pub enum Shape {
    Circle(f64),
    Square { side: f64 },
}

const MAX: usize = 10;

impl Point {
    /// Build a new point.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn norm(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

pub trait Draw {
    fn draw(&self) -> String;
}

pub fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Square { side } => side * side,
    }
}
"#;

    #[test]
    fn rust_outline_is_signatures_only_and_indents_members() {
        let path = PathBuf::from("sample.rs");
        let o = outline(&path, SAMPLE, false).expect("rust is supported");
        assert_eq!(o.lang, "rust");

        // Top-level items and impl members are present...
        assert!(o.text.contains("pub struct Point"));
        assert!(o.text.contains("pub struct Pair(i32, i32);")); // tuple fields kept
        assert!(o.text.contains("pub enum Shape"));
        assert!(o.text.contains("const MAX: usize = 10;"));
        assert!(o.text.contains("impl Point"));
        assert!(o.text.contains("pub fn new(x: f64, y: f64) -> Self"));
        assert!(o.text.contains("fn norm(&self) -> f64"));
        assert!(o.text.contains("pub trait Draw"));
        assert!(o.text.contains("fn draw(&self) -> String;"));
        assert!(o.text.contains("pub fn area(s: &Shape) -> f64"));

        // ...but bodies are gone.
        assert!(!o.text.contains("self.x * self.x"));
        assert!(!o.text.contains("3.14"));
        assert!(!o.text.contains("Self { x, y }"));

        // Members are indented under their container.
        assert!(o.text.contains("\n    pub fn new"));

        // The outline is dramatically smaller than the source.
        assert!(o.text.len() * 2 < SAMPLE.len());
    }

    #[test]
    fn map_mode_prefixes_line_numbers() {
        let path = PathBuf::from("sample.rs");
        let o = outline(&path, SAMPLE, true).unwrap();
        // `pub struct Point` starts on line 6 of SAMPLE (1-based, leading newline included).
        assert!(o.text.lines().any(|l| l.contains(": pub struct Point")));
        assert!(o.text.lines().all(|l| {
            let t = l.trim_start();
            t.is_empty() || t.chars().next().unwrap().is_ascii_digit()
        }));
    }

    #[test]
    fn unsupported_language_returns_none() {
        let path = PathBuf::from("notes.txt");
        assert!(outline(&path, "just some prose\n", false).is_none());
        assert!(!supported(&path));
        assert!(supported(&PathBuf::from("lib.rs")));
    }
}
