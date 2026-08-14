//! JSON and JSONC editing on the `jsonc-parser` CST (D37).
//!
//! The CST retains source spans, so mutating one node leaves every other byte
//! exactly as it was — including the comments a JSONC file carries, the
//! indentation style the developer chose, the key order, and any unicode
//! escapes. That is what SC-104 measures.
//!
//! `opencode.jsonc` is **parsed but never written** (D37): the same CST reads
//! it so a Cairn entry inside one is *detected*, while Cairn writes
//! `opencode.json`, which OpenCode merges alongside it.

use super::{Change, EditError};
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::Value;

/// Parse options that accept everything a supported agent may legitimately
/// write: comments and trailing commas are ordinary in these files.
fn options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: true,
        ..ParseOptions::default()
    }
}

/// Read a configuration file for inspection.
///
/// A file that does not exist reads as an empty object — that is a normal
/// state, not damage. A file that exists and cannot be parsed is `Malformed`,
/// and every caller then changes nothing.
pub fn read(path: &str, text: &str) -> Result<Value, EditError> {
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    match jsonc_parser::parse_to_serde_value(text, &options()) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Ok(Value::Object(serde_json::Map::new())),
        Err(e) => Err(EditError::Malformed {
            path: path.to_string(),
            detail: e.to_string(),
        }),
    }
}

/// Read the value at a nested object path, if present.
pub fn get(path: &str, text: &str, keys: &[&str]) -> Result<Option<Value>, EditError> {
    let mut cursor = read(path, text)?;
    for k in keys {
        match cursor.get(*k) {
            Some(v) => cursor = v.clone(),
            None => return Ok(None),
        }
    }
    Ok(Some(cursor))
}

/// The text an absent file starts from.
///
/// Multi-line so the CST's insertions are multi-line too: a configuration file
/// Cairn creates should look like one a person would write.
const EMPTY_DOCUMENT: &str = "{\n}\n";

/// Set a value at a nested object path, creating intermediate objects.
///
/// Returns `Unchanged` when the value is already exactly what is wanted, which
/// is what makes reconnecting write nothing (FR-135, SC-102).
pub fn upsert(path: &str, text: &str, keys: &[&str], value: &Value) -> Result<Change, EditError> {
    if keys.is_empty() {
        return Err(EditError::UnexpectedShape {
            path: path.to_string(),
            detail: "an empty key path cannot address a value".into(),
        });
    }
    if get(path, text, keys)?.as_ref() == Some(value) {
        return Ok(Change::Unchanged);
    }

    let source = if text.trim().is_empty() {
        EMPTY_DOCUMENT
    } else {
        text
    };
    let root = CstRootNode::parse(source, &options()).map_err(|e| EditError::Malformed {
        path: path.to_string(),
        detail: e.to_string(),
    })?;
    let mut object = root.object_value_or_set();
    for key in &keys[..keys.len() - 1] {
        object = object
            .object_value_or_create(key)
            .ok_or_else(|| EditError::UnexpectedShape {
                path: path.to_string(),
                detail: format!("`{key}` exists and is not an object"),
            })?;
    }
    let last = keys[keys.len() - 1];
    let input = to_input(value);
    match object.get(last) {
        Some(prop) => prop.set_value(input),
        None => {
            object.append(last, input);
        }
    }
    Ok(Change::Written(root.to_string()))
}

/// Remove the value at a nested object path.
///
/// Containers the removal itself empties are removed too. Cairn creates
/// `mcpServers` when it is absent, and leaving `"mcpServers": {}` behind after
/// a disconnect is litter Cairn put there — SC-104 measures exactly that. A
/// container that still holds anything the developer wrote is never touched.
///
/// Removing something that is not there is `Unchanged`, which keeps
/// disconnect idempotent (FR-157).
pub fn remove(path: &str, text: &str, keys: &[&str]) -> Result<Change, EditError> {
    if text.trim().is_empty() || get(path, text, keys)?.is_none() {
        return Ok(Change::Unchanged);
    }
    let root = CstRootNode::parse(text, &options()).map_err(|e| EditError::Malformed {
        path: path.to_string(),
        detail: e.to_string(),
    })?;
    let Some(root_object) = root.object_value() else {
        return Ok(Change::Unchanged);
    };

    // Walk down, remembering each container so one the removal empties can be
    // pruned on the way back up.
    let mut chain = vec![root_object.clone()];
    let mut object = root_object;
    for key in &keys[..keys.len() - 1] {
        match object.object_value(key) {
            Some(o) => {
                chain.push(o.clone());
                object = o;
            }
            None => return Ok(Change::Unchanged),
        }
    }
    if let Some(prop) = object.get(keys[keys.len() - 1]) {
        prop.remove();
    }
    // The root itself is never removed; an empty document is the caller's
    // signal that a file Cairn created holds nothing else.
    for depth in (1..chain.len()).rev() {
        if chain[depth].properties().is_empty() {
            if let Some(prop) = chain[depth - 1].get(keys[depth - 1]) {
                prop.remove();
            }
        }
    }
    Ok(Change::Written(root.to_string()))
}

/// True where a document holds nothing at all.
///
/// After a disconnect this is how the caller knows a file Cairn created in its
/// entirety can go, rather than being left as an empty object the developer
/// never wrote.
pub fn is_empty_document(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    jsonc_parser::parse_to_serde_value::<Value>(t, &options())
        .ok()
        .and_then(|v| v.as_object().map(|o| o.is_empty()))
        .unwrap_or(false)
}

/// True where the container at `keys` occupies a single line.
///
/// The CST cannot insert into an object without expanding it onto several
/// lines, so a minified container Cairn writes into is reflowed. Cairn records
/// this one bit about its *own* edit — not a copy of the developer's file,
/// which FR-156 and FR-238 forbid — so removal can put the layout back exactly.
pub fn container_is_single_line(path: &str, text: &str, keys: &[&str]) -> bool {
    if keys.len() < 2 {
        return false;
    }
    let Ok(Some(_)) = get(path, text, &keys[..keys.len() - 1]) else {
        return false;
    };
    match container_span(text, &keys[..keys.len() - 1]) {
        Some((start, end)) => !text[start..end].contains('\n'),
        None => false,
    }
}

/// True where the array or object *at* `keys` occupies a single line.
///
/// The array counterpart of `container_is_single_line`: a lifecycle
/// registration list written on one line is reflowed by the insert, and the
/// same one bit puts it back.
pub fn value_is_single_line(path: &str, text: &str, keys: &[&str]) -> bool {
    let Ok(Some(_)) = get(path, text, keys) else {
        return false;
    };
    match container_span(text, keys) {
        Some((start, end)) => !text[start..end].contains('\n'),
        None => false,
    }
}

/// Remove every array element matching `is_ours`, then collapse the array back
/// onto one line.
pub fn remove_array_entries_collapsing(
    path: &str,
    text: &str,
    keys: &[&str],
    is_ours: &dyn Fn(&Value) -> bool,
    prune_empty: bool,
) -> Result<Change, EditError> {
    let removed = remove_array_entries(path, text, keys, is_ours, prune_empty)?;
    let Change::Written(out) = &removed else {
        return Ok(removed);
    };
    let Some((start, end)) = container_span(out, keys) else {
        // The array went with its last entry, so there is nothing left to put
        // back on one line. Pruning already removed the container.
        return Ok(removed);
    };
    let collapsed = collapse(&out[start..end]);
    let mut result = String::with_capacity(out.len());
    result.push_str(&out[..start]);
    result.push_str(&collapsed);
    result.push_str(&out[end..]);
    Ok(Change::Written(result))
}

/// Remove the value at `keys`, then collapse its container back onto one line.
///
/// Used when the container was single-line before Cairn wrote into it. Every
/// remaining property keeps the byte-exact text the CST preserved, so
/// collapsing reproduces the original bytes (SC-104).
pub fn remove_collapsing(path: &str, text: &str, keys: &[&str]) -> Result<Change, EditError> {
    let removed = remove(path, text, keys)?;
    let Change::Written(out) = &removed else {
        return Ok(removed);
    };
    let Some((start, end)) = container_span(out, &keys[..keys.len() - 1]) else {
        return Ok(removed);
    };
    let collapsed = collapse(&out[start..end]);
    let mut result = String::with_capacity(out.len());
    result.push_str(&out[..start]);
    result.push_str(&collapsed);
    result.push_str(&out[end..]);
    Ok(Change::Written(result))
}

/// Byte range of the object value at `keys`, braces included.
///
/// A small scanner rather than a parse: it only has to find a brace-delimited
/// span in a document the CST has already validated.
fn container_span(text: &str, keys: &[&str]) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut target_depth = 1usize;
    let mut key_index = 0usize;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                // A key at the depth we are looking for?
                if depth == target_depth && key_index < keys.len() {
                    let want = format!("\"{}\"", keys[key_index]);
                    if text[i..].starts_with(&want) {
                        let after = i + want.len();
                        let rest = &text[after..];
                        let trimmed = rest.trim_start();
                        if let Some(colon) = trimmed.strip_prefix(':') {
                            let value = colon.trim_start();
                            if value.starts_with('{') || value.starts_with('[') {
                                let brace = after + (rest.len() - value.len());
                                key_index += 1;
                                if key_index == keys.len() {
                                    return matching_brace(text, brace).map(|e| (brace, e));
                                }
                                // Only an object can hold the next key.
                                if !value.starts_with('{') {
                                    return None;
                                }
                                target_depth += 1;
                                i = brace + 1;
                                depth += 1;
                                continue;
                            }
                        }
                    }
                }
                in_string = true;
                i += 1;
            }
            '{' | '[' => {
                depth += 1;
                i += 1;
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// End offset (exclusive) of the object opening at `start`.
fn matching_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, b) in bytes[start..].iter().enumerate() {
        let c = *b as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Remove line breaks and their indentation from a comment-free span, then
/// restore the object's own padding style.
///
/// Expanding is lossy in exactly one way: `{"a":1}` and `{ "a": 1 }` both
/// expand to the same multi-line form, so collapsing has to choose. The file's
/// own formatting decides it — a document that writes `"key": value` with a
/// space after the colon also writes `{ … }` with padding, and one that writes
/// `"key":value` does not. That inference is taken from the surviving property
/// text, which the CST preserved byte-exactly.
fn collapse(span: &str) -> String {
    let collapsed = collapse_whitespace(span);
    if !span.contains('\n') || !collapsed.starts_with('{') || !collapsed.ends_with('}') {
        return collapsed;
    }
    let inner = &collapsed[1..collapsed.len() - 1];
    if inner.is_empty() {
        return collapsed;
    }
    if uses_spaced_colons(inner) {
        format!("{{ {inner} }}")
    } else {
        collapsed
    }
}

/// Whether a property list writes `"key": value` rather than `"key":value`.
fn uses_spaced_colons(inner: &str) -> bool {
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, b) in bytes.iter().enumerate() {
        let c = *b as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                return bytes.get(i + 1) == Some(&b' ');
            }
            _ => {}
        }
    }
    false
}

/// Remove line breaks and their indentation from a comment-free span.
fn collapse_whitespace(span: &str) -> String {
    let mut out = String::with_capacity(span.len());
    let mut chars = span.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '\n' | '\r' => {
                while matches!(
                    chars.peek(),
                    Some(' ') | Some('\t') | Some('\n') | Some('\r')
                ) {
                    chars.next();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Replace the first array element matching `is_ours`, or append `entry`.
///
/// This is how a lifecycle registration is installed without disturbing a
/// developer's own handlers on the same event: their entries are never
/// examined for content, only skipped (FR-180).
pub fn upsert_array_entry(
    path: &str,
    text: &str,
    keys: &[&str],
    is_ours: &dyn Fn(&Value) -> bool,
    entry: &Value,
) -> Result<Change, EditError> {
    let existing = get(path, text, keys)?;
    if let Some(Value::Array(items)) = &existing {
        let ours: Vec<&Value> = items.iter().filter(|v| is_ours(v)).collect();
        if ours.len() == 1 && ours[0] == entry {
            return Ok(Change::Unchanged);
        }
    }
    if existing.is_some() && !matches!(existing, Some(Value::Array(_))) {
        return Err(EditError::UnexpectedShape {
            path: path.to_string(),
            detail: format!("`{}` exists and is not an array", keys.join(".")),
        });
    }

    let source = if text.trim().is_empty() {
        EMPTY_DOCUMENT
    } else {
        text
    };
    let root = CstRootNode::parse(source, &options()).map_err(|e| EditError::Malformed {
        path: path.to_string(),
        detail: e.to_string(),
    })?;
    let mut object = root.object_value_or_set();
    for key in &keys[..keys.len() - 1] {
        object = object
            .object_value_or_create(key)
            .ok_or_else(|| EditError::UnexpectedShape {
                path: path.to_string(),
                detail: format!("`{key}` exists and is not an object"),
            })?;
    }
    let array = object.array_value_or_set(keys[keys.len() - 1]);

    // Deduplication is deterministic: given the same input, the same entries
    // survive and the same are removed (FR-158). Ours are removed in document
    // order and exactly one is appended.
    let mut removed_any = false;
    for element in array.elements() {
        let as_value = node_to_value(&element);
        if as_value.as_ref().map(is_ours).unwrap_or(false) {
            element.remove();
            removed_any = true;
        }
    }
    let _ = removed_any;
    array.append(to_input(entry));
    Ok(Change::Written(root.to_string()))
}

/// Remove every array element matching `is_ours`.
///
/// `prune_empty` says whether Cairn created this array — one bit about Cairn's
/// own edit, supplied by the local record. When it did, an array left empty
/// goes with it, along with any container the removal empties. When it did
/// not, an empty array the developer wrote is left exactly where it is: Cairn
/// removes what it owns and nothing else (FR-178, FR-180).
pub fn remove_array_entries(
    path: &str,
    text: &str,
    keys: &[&str],
    is_ours: &dyn Fn(&Value) -> bool,
    prune_empty: bool,
) -> Result<Change, EditError> {
    let Some(Value::Array(items)) = get(path, text, keys)? else {
        return Ok(Change::Unchanged);
    };
    if !items.iter().any(is_ours) {
        return Ok(Change::Unchanged);
    }
    let remaining = items.iter().filter(|v| !is_ours(v)).count();

    let root = CstRootNode::parse(text, &options()).map_err(|e| EditError::Malformed {
        path: path.to_string(),
        detail: e.to_string(),
    })?;
    let Some(root_object) = root.object_value() else {
        return Ok(Change::Unchanged);
    };
    let mut chain = vec![root_object.clone()];
    let mut object = root_object;
    for key in &keys[..keys.len() - 1] {
        match object.object_value(key) {
            Some(o) => {
                chain.push(o.clone());
                object = o;
            }
            None => return Ok(Change::Unchanged),
        }
    }
    let last = keys[keys.len() - 1];
    if remaining == 0 && prune_empty {
        // An event key that only ever held Cairn's registration goes with it,
        // rather than being left as an empty array Cairn created.
        if let Some(prop) = object.get(last) {
            prop.remove();
        }
    } else if let Some(array) = object.array_value(last) {
        for element in array.elements() {
            if node_to_value(&element)
                .as_ref()
                .map(is_ours)
                .unwrap_or(false)
            {
                element.remove();
            }
        }
    }
    // Prune a container the removal emptied, exactly as `remove` does: an
    // empty `hooks` object is litter Cairn created.
    if !prune_empty {
        return Ok(Change::Written(root.to_string()));
    }
    for depth in (1..chain.len()).rev() {
        if chain[depth].properties().is_empty() {
            if let Some(prop) = chain[depth - 1].get(keys[depth - 1]) {
                prop.remove();
            }
        }
    }
    Ok(Change::Written(root.to_string()))
}

/// Collapse the container at `keys` back onto one line.
///
/// Applied after a removal to containers that were on one line before Cairn
/// wrote into them. A container that no longer exists is a no-op.
pub fn collapse_path(text: &str, keys: &[&str]) -> String {
    let Some((start, end)) = container_span(text, keys) else {
        return text.to_string();
    };
    format!(
        "{}{}{}",
        &text[..start],
        collapse(&text[start..end]),
        &text[end..]
    )
}

/// Collapse the document's own top-level object onto one line.
///
/// The counterpart of `container_is_single_line` for a file that was entirely
/// on one line — `{}` most often — which the CST expands the moment anything
/// is inserted.
pub fn collapse_root(text: &str) -> String {
    let Some(start) = text.find(['{', '[']) else {
        return text.to_string();
    };
    let Some(end) = matching_brace(text, start) else {
        return text.to_string();
    };
    format!(
        "{}{}{}",
        &text[..start],
        collapse(&text[start..end]),
        &text[end..]
    )
}

/// True where the document's top-level value is on a single line.
pub fn root_is_single_line(text: &str) -> bool {
    let Some(start) = text.find(['{', '[']) else {
        return false;
    };
    match matching_brace(text, start) {
        Some(end) => !text[start..end].contains('\n'),
        None => false,
    }
}

/// Read one CST node back as a plain value, for matching.
fn node_to_value(node: &jsonc_parser::cst::CstNode) -> Option<Value> {
    jsonc_parser::parse_to_serde_value(&node.to_string(), &options())
        .ok()
        .flatten()
}

/// Convert a plain value into the CST's input form.
fn to_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(a) => CstInputValue::Array(a.iter().map(to_input).collect()),
        Value::Object(o) => {
            CstInputValue::Object(o.iter().map(|(k, v)| (k.clone(), to_input(v))).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const COMMENTED: &str = r#"{
  // the developer's own note
  "mcpServers": {
    "other":	{ "command": "other-server" }
  },
  "unrelated": "été"
}
"#;

    #[test]
    fn a_comment_and_an_escape_survive_an_edit() {
        // SC-104: 100% of non-Cairn content byte-identical.
        let out = upsert(
            "cfg.json",
            COMMENTED,
            &["mcpServers", "cairn"],
            &json!({"command": "cairn", "args": ["mcp"]}),
        )
        .unwrap();
        let text = out.text().unwrap();
        assert!(text.contains("// the developer's own note"));
        assert!(text.contains(r#""unrelated": "été""#));
        assert!(text.contains("\"other\":\t{ \"command\": \"other-server\" }"));
    }

    #[test]
    fn removing_returns_the_file_to_its_original_bytes() {
        // The connect → disconnect round trip SC-104 measures.
        let added = upsert(
            "cfg.json",
            COMMENTED,
            &["mcpServers", "cairn"],
            &json!({"command": "cairn", "args": ["mcp"]}),
        )
        .unwrap();
        let back = remove("cfg.json", added.text().unwrap(), &["mcpServers", "cairn"]).unwrap();
        assert_eq!(back.text().unwrap(), COMMENTED);
    }

    #[test]
    fn a_second_identical_write_is_unchanged() {
        // FR-135, SC-102.
        let entry = json!({"command": "cairn", "args": ["mcp"]});
        let once = upsert("cfg.json", COMMENTED, &["mcpServers", "cairn"], &entry).unwrap();
        let twice = upsert(
            "cfg.json",
            once.text().unwrap(),
            &["mcpServers", "cairn"],
            &entry,
        )
        .unwrap();
        assert_eq!(twice, Change::Unchanged);
    }

    #[test]
    fn malformed_input_fails_closed() {
        // FR-137: report the condition, change nothing.
        let bad = "{ \"a\": ";
        let e = upsert("cfg.json", bad, &["a"], &json!(1)).unwrap_err();
        assert!(matches!(e, EditError::Malformed { .. }));
        assert_eq!(
            e.condition(),
            crate::model::HealthCondition::MalformedConfig
        );
        assert!(read("cfg.json", bad).is_err());
    }

    #[test]
    fn an_unexpected_shape_is_refused_rather_than_overwritten() {
        let text = r#"{ "mcpServers": [] }"#;
        let e = upsert(
            "cfg.json",
            text,
            &["mcpServers", "cairn"],
            &json!({"command": "cairn"}),
        )
        .unwrap_err();
        assert!(matches!(e, EditError::UnexpectedShape { .. }));
    }

    #[test]
    fn an_array_entry_is_replaced_not_appended_twice() {
        // The Feature 001 duplication bug, as a test (SC-103).
        let is_ours = |v: &Value| v.get("id").and_then(|i| i.as_str()) == Some("cairn");
        let entry = json!({"id": "cairn", "command": "cairn hook Stop"});
        let start = r#"{
  "hooks": {
    "Stop": [
      { "command": "make lint" }
    ]
  }
}
"#;
        let once =
            upsert_array_entry("s.json", start, &["hooks", "Stop"], &is_ours, &entry).unwrap();
        let twice = upsert_array_entry(
            "s.json",
            once.text().unwrap(),
            &["hooks", "Stop"],
            &is_ours,
            &entry,
        )
        .unwrap();
        assert_eq!(twice, Change::Unchanged, "connecting twice duplicated");
        let v = read("s.json", once.text().unwrap()).unwrap();
        let arr = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr.iter().filter(|e| is_ours(e)).count(), 1);
        assert!(once.text().unwrap().contains("make lint"));
    }

    #[test]
    fn removing_our_array_entry_leaves_the_developers_alone() {
        // FR-180, SC-116.
        let is_ours = |v: &Value| v.get("id").and_then(|i| i.as_str()) == Some("cairn");
        let start = r#"{
  "hooks": {
    "Stop": [
      { "command": "make lint" },
      { "id": "cairn", "command": "cairn hook Stop" }
    ]
  }
}
"#;
        let out =
            remove_array_entries("s.json", start, &["hooks", "Stop"], &is_ours, true).unwrap();
        let text = out.text().unwrap();
        assert!(text.contains("make lint"));
        assert!(!text.contains("cairn hook Stop"));
    }

    #[test]
    fn an_event_key_holding_only_our_entry_goes_with_it() {
        let is_ours = |v: &Value| v.get("id").and_then(|i| i.as_str()) == Some("cairn");
        let start = r#"{ "hooks": { "Stop": [ { "id": "cairn" } ] } }"#;
        let out =
            remove_array_entries("s.json", start, &["hooks", "Stop"], &is_ours, true).unwrap();
        let v = read("s.json", out.text().unwrap()).unwrap();
        assert!(v["hooks"].get("Stop").is_none());
    }

    #[test]
    fn removing_the_last_entry_takes_the_container_cairn_created_with_it() {
        // The defect the corpus caught: `{"mcpServers": {}}` is litter Cairn
        // put there, and SC-104 measures byte identity.
        let out = upsert("c.json", "", &["mcpServers", "cairn"], &json!({"a": 1})).unwrap();
        let back = remove("c.json", out.text().unwrap(), &["mcpServers", "cairn"]).unwrap();
        assert!(is_empty_document(back.text().unwrap()), "{:?}", back.text());
    }

    #[test]
    fn a_minified_container_survives_the_round_trip_exactly() {
        // The CST expands an object to insert into it; collapsing on removal
        // is what returns a minified file to its original bytes (SC-104).
        let src = r#"{"mcpServers":{"one":{"command":"one"}}}"#;
        assert!(container_is_single_line(
            "c.json",
            src,
            &["mcpServers", "cairn"]
        ));
        let added = upsert("c.json", src, &["mcpServers", "cairn"], &crate::mcp_entry()).unwrap();
        let back =
            remove_collapsing("c.json", added.text().unwrap(), &["mcpServers", "cairn"]).unwrap();
        assert_eq!(back.text().unwrap(), src);
    }

    #[test]
    fn a_multiline_container_is_not_reported_as_single_line() {
        let src = "{\n  \"mcpServers\": {\n    \"one\": { \"command\": \"one\" }\n  }\n}\n";
        assert!(!container_is_single_line(
            "c.json",
            src,
            &["mcpServers", "cairn"]
        ));
    }

    #[test]
    fn a_container_the_developer_still_uses_is_never_pruned() {
        let src = "{\n  \"mcpServers\": {\n    \"theirs\": { \"command\": \"t\" }\n  }\n}\n";
        let out = upsert("c.json", src, &["mcpServers", "cairn"], &json!({"a": 1})).unwrap();
        let back = remove("c.json", out.text().unwrap(), &["mcpServers", "cairn"]).unwrap();
        assert_eq!(back.text().unwrap(), src);
    }

    #[test]
    fn an_empty_array_the_developer_wrote_is_never_pruned() {
        // FR-180: Cairn removes what it owns and nothing else.
        let is_ours = |v: &Value| v.get("id").and_then(|i| i.as_str()) == Some("cairn");
        let start = "{\n  \"hooks\": [\n    { \"id\": \"cairn\" }\n  ]\n}\n";
        let out = remove_array_entries("s.json", start, &["hooks"], &is_ours, false).unwrap();
        let v = read("s.json", out.text().unwrap()).unwrap();
        assert_eq!(v["hooks"].as_array().map(|a| a.len()), Some(0));
    }

    #[test]
    fn an_absent_file_is_created_rather_than_reported_as_damage() {
        let out = upsert("new.json", "", &["mcpServers", "cairn"], &json!({"a": 1})).unwrap();
        let v = read("new.json", out.text().unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["cairn"]["a"], 1);
    }

    #[test]
    fn jsonc_is_read_so_a_shadowing_entry_is_detected() {
        // D37: parsed but not written. Detection is the whole reason.
        let jsonc =
            "{\n  // project overrides\n  \"mcp\": { \"cairn\": { \"command\": \"x\" } },\n}\n";
        let found = get("opencode.jsonc", jsonc, &["mcp", "cairn"]).unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn crlf_and_tabs_survive() {
        let src = "{\r\n\t\"a\": 1\r\n}\r\n";
        let out = upsert("c.json", src, &["b"], &json!(2)).unwrap();
        let text = out.text().unwrap();
        assert!(text.contains("\r\n"), "CRLF was normalised away");
        assert!(text.contains('\t'), "tab indentation was lost");
    }
}
