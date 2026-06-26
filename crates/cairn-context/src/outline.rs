//! AST-based file outlines: render a code file as just its structure - the signatures of its
//! top-level items and their members - so a 1000-line file costs a handful of tokens instead of
//! a thousand lines. Lossless as always: the full original is retained in the blob store and one
//! `expand` away.
//!
//! Backed by [`tree_sitter`] (real parsing, not regex/heuristics). Supported today: Rust, Python,
//! JavaScript, TypeScript/TSX, Go, C, C++, Java, C#, Ruby, Bash. The [`LangSpec`] table makes
//! adding a language a matter of listing its node kinds - no new traversal logic.

use std::path::Path;
use tree_sitter::{Language, Node, Parser};

/// The structural view of a file: a signature outline.
pub struct Outline {
    /// Language name (for the human-facing note), e.g. `"rust"`.
    pub lang: &'static str,
    /// How many signatures we emitted (top-level + nested members).
    pub items: usize,
    /// The outline text - one signature per line, members indented under their container.
    pub text: String,
}

/// Per-language description of which AST nodes are signatures and where their bodies begin.
struct LangSpec {
    language: Language,
    name: &'static str,
    /// Node kinds whose header we emit as a one-line signature.
    sig_kinds: &'static [&'static str],
    /// Of the signatures, the kinds we descend into for nested members (impl/class/trait bodies).
    container_kinds: &'static [&'static str],
    /// Wrapper kinds we descend *through* without emitting (decorators, `export` statements).
    transparent_kinds: &'static [&'static str],
    /// Kinds that mark the start of a "body" - the signature is cut off right before the first one.
    body_kinds: &'static [&'static str],
}

/// Longest signature we keep before truncating (guards against pathological macros / unions).
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
        transparent_kinds: &[],
        // Note: tuple-struct fields (`ordered_field_declaration_list`) are intentionally *not*
        // here - they're part of the type's identity, so we keep `struct P(i32, i32);` whole.
        body_kinds: &[
            "block",
            "declaration_list",
            "field_declaration_list",
            "enum_variant_list",
        ],
    }
}

fn python_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_python::LANGUAGE.into(),
        name: "python",
        sig_kinds: &["function_definition", "class_definition"],
        container_kinds: &["class_definition"],
        transparent_kinds: &["decorated_definition"],
        body_kinds: &["block"],
    }
}

fn javascript_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_javascript::LANGUAGE.into(),
        name: "javascript",
        sig_kinds: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "method_definition",
            // top-level `const f = () => ...` / data constants (one line each).
            "lexical_declaration",
            "variable_declaration",
        ],
        container_kinds: &["class_declaration"],
        transparent_kinds: &["export_statement"],
        body_kinds: &["statement_block", "class_body"],
    }
}

fn typescript_spec(tsx: bool) -> LangSpec {
    LangSpec {
        language: if tsx {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        } else {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        },
        name: "typescript",
        sig_kinds: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            // interface members (reached only by descending an interface).
            "method_signature",
            "property_signature",
            "lexical_declaration",
            "variable_declaration",
        ],
        container_kinds: &[
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
        ],
        transparent_kinds: &["export_statement"],
        // `enum_body` is deliberately absent: small enums show their members inline (capped).
        body_kinds: &["statement_block", "class_body", "interface_body"],
    }
}

fn go_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_go::LANGUAGE.into(),
        name: "go",
        sig_kinds: &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
            "const_declaration",
            "var_declaration",
        ],
        container_kinds: &[],
        transparent_kinds: &[],
        // `method_elem` cuts an interface header (`type R interface`); `field_declaration_list`
        // cuts a struct header (`type P struct`); `block` cuts a func/method body.
        body_kinds: &["block", "field_declaration_list", "method_elem"],
    }
}

fn c_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_c::LANGUAGE.into(),
        name: "c",
        sig_kinds: &[
            "function_definition",
            "struct_specifier",
            "enum_specifier",
            "union_specifier",
        ],
        container_kinds: &[],
        transparent_kinds: &[],
        body_kinds: &[
            "compound_statement",
            "field_declaration_list",
            "enumerator_list",
        ],
    }
}

fn cpp_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_cpp::LANGUAGE.into(),
        name: "cpp",
        sig_kinds: &[
            "function_definition",
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "union_specifier",
            "namespace_definition",
        ],
        container_kinds: &["namespace_definition"],
        transparent_kinds: &[],
        body_kinds: &[
            "compound_statement",
            "field_declaration_list",
            "enumerator_list",
            "declaration_list",
        ],
    }
}

fn java_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_java::LANGUAGE.into(),
        name: "java",
        sig_kinds: &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
            "method_declaration",
            "constructor_declaration",
        ],
        container_kinds: &["class_declaration", "interface_declaration"],
        transparent_kinds: &[],
        body_kinds: &["block", "class_body", "interface_body", "constructor_body"],
    }
}

fn csharp_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_c_sharp::LANGUAGE.into(),
        name: "csharp",
        sig_kinds: &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "enum_declaration",
            "record_declaration",
            "namespace_declaration",
            "method_declaration",
            "property_declaration",
            "constructor_declaration",
        ],
        container_kinds: &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "namespace_declaration",
        ],
        transparent_kinds: &[],
        body_kinds: &["block", "declaration_list", "accessor_list"],
    }
}

fn ruby_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_ruby::LANGUAGE.into(),
        name: "ruby",
        sig_kinds: &["method", "singleton_method", "class", "module"],
        container_kinds: &["class", "module"],
        transparent_kinds: &[],
        body_kinds: &["body_statement"],
    }
}

fn bash_spec() -> LangSpec {
    LangSpec {
        language: tree_sitter_bash::LANGUAGE.into(),
        name: "bash",
        sig_kinds: &["function_definition"],
        container_kinds: &[],
        transparent_kinds: &[],
        body_kinds: &["compound_statement"],
    }
}

fn spec_for_path(path: &Path) -> Option<LangSpec> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(rust_spec()),
        Some("py" | "pyi") => Some(python_spec()),
        Some("js" | "mjs" | "cjs" | "jsx") => Some(javascript_spec()),
        Some("ts" | "mts" | "cts") => Some(typescript_spec(false)),
        Some("tsx") => Some(typescript_spec(true)),
        Some("go") => Some(go_spec()),
        Some("c") => Some(c_spec()),
        // `.h` goes to the C++ grammar: it parses C headers fine and also handles C++ classes.
        Some("cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h") => Some(cpp_spec()),
        Some("java") => Some(java_spec()),
        Some("cs") => Some(csharp_spec()),
        Some("rb") => Some(ruby_spec()),
        Some("sh" | "bash") => Some(bash_spec()),
        _ => None,
    }
}

/// Whether Cairn can produce a signature outline for this path's language.
pub fn supported(path: &Path) -> bool {
    spec_for_path(path).is_some()
}

/// Outline `source` (the contents of `path`) as a signature map. `with_lines` prefixes each
/// signature with its 1-based start line (`map` mode); without it you get bare signatures.
/// Returns `None` for unsupported languages or unparseable input - callers fall back to a full read.
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
        // Nothing structural to show (e.g. a file of only statements) - let the caller fall back.
        return None;
    }
    Some(Outline {
        lang: spec.name,
        items,
        text,
    })
}

/// Emit signatures for every item directly under `node`, descending into container bodies and
/// through transparent wrappers (decorators, `export`).
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

        if spec.transparent_kinds.contains(&kind) {
            // A wrapper (e.g. `export ...`, `@decorator ...`) - emit its inner item at the same depth.
            walk(child, bytes, spec, depth, with_lines, out, count);
            continue;
        }
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
    // A dangling opening brace can be left when we cut just before a `{ ... }` body.
    let mut sig = collapsed
        .trim_end_matches(|c: char| c == '{' || c.is_whitespace())
        .to_string();
    if sig.chars().count() > MAX_SIG {
        sig = sig.chars().take(MAX_SIG).collect::<String>();
        sig.push_str(" ...");
    }
    sig
}

/// The first descendant of `node` (pre-order) whose kind marks the start of a body. We do not
/// descend into nested signatures - their bodies aren't this node's. This depth-tolerant search is
/// what lets one rule handle both direct bodies (Rust `fn ... { block }`) and wrapped ones
/// (Go `type T struct { ... }`, where the field list sits two levels down).
fn body_node<'a>(node: Node<'a>, spec: &LangSpec) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if spec.body_kinds.contains(&child.kind()) {
            return Some(child);
        }
        if spec.sig_kinds.contains(&child.kind()) {
            continue; // a nested item - skip its subtree
        }
        if let Some(found) = body_node(child, spec) {
            return Some(found);
        }
    }
    None
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

    fn text(path: &str, src: &str) -> String {
        outline(&PathBuf::from(path), src, false)
            .unwrap_or_else(|| panic!("expected an outline for {path}"))
            .text
    }

    #[test]
    fn rust_outline_is_signatures_only_and_indents_members() {
        let src = r#"
//! A module.
use std::fmt;

/// A point.
pub struct Point { pub x: f64, pub y: f64 }

pub struct Pair(i32, i32);

pub enum Shape { Circle(f64), Square { side: f64 } }

const MAX: usize = 10;

impl Point {
    pub fn new(x: f64, y: f64) -> Self { Self { x, y } }
    fn norm(&self) -> f64 { (self.x * self.x).sqrt() }
}

pub trait Draw { fn draw(&self) -> String; }

pub fn area(s: &Shape) -> f64 { 0.0 }
"#;
        let o = text("sample.rs", src);
        assert!(o.contains("pub struct Point"));
        assert!(o.contains("pub struct Pair(i32, i32);")); // tuple fields kept
        assert!(o.contains("pub enum Shape"));
        assert!(o.contains("const MAX: usize = 10;"));
        assert!(o.contains("impl Point"));
        assert!(o.contains("pub fn new(x: f64, y: f64) -> Self"));
        assert!(o.contains("fn norm(&self) -> f64"));
        assert!(o.contains("pub trait Draw"));
        assert!(o.contains("fn draw(&self) -> String;"));
        assert!(o.contains("pub fn area(s: &Shape) -> f64"));
        assert!(o.contains("\n    pub fn new")); // member indented
        assert!(!o.contains("self.x * self.x"));
        assert!(!o.contains("Self { x, y }"));
    }

    #[test]
    fn python_outline_handles_decorators_and_methods() {
        let src = "\
@dec
def foo(a, b=1):
    return a + b

class Animal:
    def __init__(self, name):
        self.name = name

    @property
    def speak(self):
        return \"...\"
";
        let o = text("a.py", src);
        assert!(o.contains("def foo(a, b=1)"));
        assert!(o.contains("class Animal"));
        assert!(o.contains("def __init__(self, name)"));
        assert!(o.contains("def speak(self)"));
        assert!(o.contains("\n    def __init__")); // member indented under the class
        assert!(!o.contains("return a + b"));
        assert!(!o.contains("self.name = name"));
    }

    #[test]
    fn javascript_outline_covers_functions_classes_and_arrows() {
        let src = "\
export function add(a, b) { return a + b; }

export class Stack {
  push(x) { this.items.push(x); }
  pop() { return this.items.pop(); }
}

export const double = (n) => n * 2;
const SECRET = 42;
";
        let o = text("a.js", src);
        assert!(o.contains("function add(a, b)"));
        assert!(o.contains("class Stack"));
        assert!(o.contains("push(x)"));
        assert!(o.contains("pop()"));
        assert!(o.contains("const double = (n) => n * 2"));
        assert!(o.contains("const SECRET = 42"));
        assert!(o.contains("\n    push(x)")); // method indented under the class
        assert!(!o.contains("this.items.push"));
    }

    #[test]
    fn typescript_outline_covers_interfaces_types_enums_classes() {
        let src = "\
export interface Shape { area(): number; name: string; }
export type ID = string | number;
export enum Color { Red, Green, Blue }
export class Circle implements Shape {
  private secret = 1;
  constructor(public r: number) {}
  area(): number { return 3.14 * this.r * this.r; }
}
export function clamp(x: number, lo: number, hi: number): number { return x; }
";
        let o = text("a.ts", src);
        assert!(o.contains("interface Shape"));
        assert!(o.contains("area(): number")); // interface method + class method
        assert!(o.contains("name: string")); // interface property
        assert!(o.contains("type ID = string | number"));
        assert!(o.contains("enum Color")); // small enum kept inline
        assert!(o.contains("class Circle implements Shape"));
        assert!(o.contains("constructor(public r: number)"));
        assert!(o.contains("function clamp(x: number, lo: number, hi: number): number"));
        assert!(!o.contains("3.14")); // bodies elided
        assert!(!o.contains("private secret")); // class fields elided
    }

    #[test]
    fn go_outline_covers_types_funcs_and_methods() {
        let src = "\
package main

type Point struct {
\tX int
\tY int
}

type Stringer interface {
\tString() string
}

const Pi = 3.14

func Dist(a, b Point) float64 { return 0 }

func (p Point) Norm() float64 { return 0 }
";
        let o = text("a.go", src);
        assert!(o.contains("type Point struct"));
        assert!(o.contains("type Stringer interface"));
        assert!(o.contains("const Pi = 3.14"));
        assert!(o.contains("func Dist(a, b Point) float64"));
        assert!(o.contains("func (p Point) Norm() float64"));
        assert!(!o.contains("X int")); // struct fields elided
        assert!(!o.contains("return 0")); // bodies elided
    }

    #[test]
    fn c_outline_covers_functions_structs_enums() {
        let src = "\
int add(int a, int b) {
    return a + b;
}

struct Point {
    int x;
    int y;
};

enum Color { RED, GREEN };
";
        let o = text("m.c", src);
        assert!(o.contains("int add(int a, int b)"));
        assert!(o.contains("struct Point"));
        assert!(o.contains("enum Color"));
        assert!(!o.contains("return a + b"));
        assert!(!o.contains("int x;"));
    }

    #[test]
    fn cpp_outline_covers_classes_namespaces_functions() {
        let src = "\
class Widget {
public:
    int id;
    void render();
};

int compute(int n) { return n * 2; }

namespace ns {
    void helper() {}
}
";
        let o = text("m.cpp", src);
        assert!(o.contains("class Widget"));
        assert!(o.contains("int compute(int n)"));
        assert!(o.contains("namespace ns"));
        assert!(o.contains("void helper()")); // namespace member (descended)
        assert!(!o.contains("return n * 2"));
    }

    #[test]
    fn java_outline_covers_classes_interfaces_methods() {
        let src = "\
public class Foo {
    private int x;
    public int get() {
        return x;
    }
}

interface Bar {
    void run();
}
";
        let o = text("M.java", src);
        assert!(o.contains("public class Foo"));
        assert!(o.contains("public int get()"));
        assert!(o.contains("interface Bar"));
        assert!(o.contains("void run();"));
        assert!(!o.contains("return x"));
        assert!(!o.contains("private int x")); // field elided
    }

    #[test]
    fn csharp_outline_covers_classes_properties_methods() {
        let src = "\
public class Svc {
    public int Id { get; set; }
    public void Run() {}
}

interface IThing {
    void M();
}
";
        let o = text("S.cs", src);
        assert!(o.contains("public class Svc"));
        assert!(o.contains("public int Id")); // property
        assert!(o.contains("public void Run()"));
        assert!(o.contains("interface IThing"));
        assert!(o.contains("void M()"));
    }

    #[test]
    fn ruby_outline_covers_classes_methods_modules() {
        let src = "\
class Animal
  def speak
    \"...\"
  end
  def legs(n)
    n
  end
end

module Helpers
  def self.run
  end
end
";
        let o = text("a.rb", src);
        assert!(o.contains("class Animal"));
        assert!(o.contains("def speak"));
        assert!(o.contains("def legs(n)"));
        assert!(o.contains("module Helpers"));
        assert!(!o.contains("\"...\""));
    }

    #[test]
    fn bash_outline_covers_functions() {
        let src = "\
function greet() {
  echo hello
}

run() {
  echo running
}
";
        let o = text("s.sh", src);
        assert!(o.contains("greet"));
        assert!(o.contains("run"));
        assert!(!o.contains("echo hello"));
    }

    #[test]
    fn map_mode_prefixes_line_numbers() {
        let src = "pub fn a() {}\npub fn b() {}\n";
        let o = outline(&PathBuf::from("x.rs"), src, true).unwrap();
        assert!(o.text.lines().any(|l| l.starts_with("1: pub fn a()")));
        assert!(o.text.lines().any(|l| l.starts_with("2: pub fn b()")));
    }

    #[test]
    fn unsupported_language_returns_none() {
        let path = PathBuf::from("notes.txt");
        assert!(outline(&path, "just some prose\n", false).is_none());
        assert!(!supported(&path));
        assert!(supported(&PathBuf::from("lib.rs")));
        assert!(supported(&PathBuf::from("app.py")));
        assert!(supported(&PathBuf::from("ui.tsx")));
        assert!(supported(&PathBuf::from("main.go")));
    }
}
