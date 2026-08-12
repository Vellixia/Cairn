//! TOML editing on `toml_edit` (D37).
//!
//! `toml_edit` is the crate Cargo itself uses for this problem: it keeps a
//! document tree that retains comments, ordering and formatting, so editing
//! `[mcp_servers.cairn]` in `~/.codex/config.toml` leaves the rest of that
//! file exactly as the developer wrote it. The plain `toml` crate discards
//! comments, which fails SC-104 by construction.

use super::{Change, EditError};
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table};

/// Read a TOML file for inspection, as a plain value.
pub fn read(path: &str, text: &str) -> Result<Value, EditError> {
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let doc = parse(path, text)?;
    Ok(item_to_value(doc.as_item()))
}

/// Read the value at a nested table path.
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

/// Set a value at a nested table path, creating intermediate tables.
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
    let mut doc = parse(path, text)?;
    let mut table: &mut Table = doc.as_table_mut();
    for key in &keys[..keys.len() - 1] {
        let entry = table
            .entry(key)
            .or_insert_with(|| Item::Table(implicit_table()));
        table = entry
            .as_table_mut()
            .ok_or_else(|| EditError::UnexpectedShape {
                path: path.to_string(),
                detail: format!("`{key}` exists and is not a table"),
            })?;
    }
    table.insert(keys[keys.len() - 1], value_to_item(value));
    Ok(Change::Written(render(text, &doc)))
}

/// Remove the value at a nested table path.
pub fn remove(path: &str, text: &str, keys: &[&str]) -> Result<Change, EditError> {
    if text.trim().is_empty() || get(path, text, keys)?.is_none() {
        return Ok(Change::Unchanged);
    }
    let mut doc = parse(path, text)?;
    let mut table: &mut Table = doc.as_table_mut();
    for key in &keys[..keys.len() - 1] {
        match table.get_mut(key).and_then(|i| i.as_table_mut()) {
            Some(t) => table = t,
            None => return Ok(Change::Unchanged),
        }
    }
    table.remove(keys[keys.len() - 1]);
    Ok(Change::Written(render(text, &doc)))
}

fn parse(path: &str, text: &str) -> Result<DocumentMut, EditError> {
    to_lf(text)
        .parse::<DocumentMut>()
        .map_err(|e| EditError::Malformed {
            path: path.to_string(),
            detail: e.to_string(),
        })
}

/// `toml_edit` normalizes line endings to LF. A file that uses CRLF
/// throughout is edited as LF and written back as CRLF, so its line endings
/// survive exactly (FR-152, SC-104). A file that mixes them is left as the
/// parser produced it rather than having a choice imposed on it.
fn uniformly_crlf(text: &str) -> bool {
    text.contains("\r\n") && !text.replace("\r\n", "").contains('\n')
}

fn to_lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Render a document back in the source's own line-ending style.
fn render(source: &str, doc: &DocumentMut) -> String {
    let out = doc.to_string();
    if uniformly_crlf(source) {
        to_crlf(&out)
    } else {
        out
    }
}

/// A table that only materialises a header when something is written into it,
/// so `[mcp_servers]` does not appear on its own.
fn implicit_table() -> Table {
    let mut t = Table::new();
    t.set_implicit(true);
    t
}

fn value_to_item(value: &Value) -> Item {
    match value {
        Value::Object(map) => {
            let mut t = Table::new();
            for (k, v) in map {
                t.insert(k, value_to_item(v));
            }
            Item::Table(t)
        }
        other => Item::Value(value_to_toml(other)),
    }
}

fn value_to_toml(value: &Value) -> toml_edit::Value {
    match value {
        Value::Null => toml_edit::Value::from(""),
        Value::Bool(b) => toml_edit::Value::from(*b),
        Value::Number(n) => n
            .as_i64()
            .map(toml_edit::Value::from)
            .or_else(|| n.as_f64().map(toml_edit::Value::from))
            .unwrap_or_else(|| toml_edit::Value::from(n.to_string())),
        Value::String(s) => toml_edit::Value::from(s.clone()),
        Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for v in a {
                arr.push(value_to_toml(v));
            }
            toml_edit::Value::Array(arr)
        }
        Value::Object(map) => {
            let mut inline = toml_edit::InlineTable::new();
            for (k, v) in map {
                inline.insert(k, value_to_toml(v));
            }
            toml_edit::Value::InlineTable(inline)
        }
    }
}

fn item_to_value(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(v) => toml_value_to_value(v),
        Item::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t.iter() {
                map.insert(k.to_string(), item_to_value(v));
            }
            Value::Object(map)
        }
        Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| item_to_value(&Item::Table(t.clone())))
                .collect(),
        ),
    }
}

fn toml_value_to_value(v: &toml_edit::Value) -> Value {
    match v {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::Number((*i.value()).into()),
        toml_edit::Value::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(toml_value_to_value).collect()),
        toml_edit::Value::InlineTable(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t.iter() {
                map.insert(k.to_string(), toml_value_to_value(v));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CONFIG: &str = r#"# Codex configuration
model = "gpt-5"          # the developer's choice

[mcp_servers.other]
command = "other-server"
args = ["serve"]

# a trailing note
"#;

    #[test]
    fn comments_order_and_formatting_survive() {
        // FR-152, SC-104.
        let out = upsert(
            "config.toml",
            CONFIG,
            &["mcp_servers", "cairn"],
            &json!({"command": "cairn", "args": ["mcp"]}),
        )
        .unwrap();
        let text = out.text().unwrap();
        assert!(text.contains("# Codex configuration"));
        assert!(text.contains(r#"model = "gpt-5"          # the developer's choice"#));
        assert!(text.contains("[mcp_servers.other]"));
        assert!(text.contains("# a trailing note"));
        assert!(text.contains("[mcp_servers.cairn]"));
    }

    #[test]
    fn removing_returns_the_file_to_its_original_bytes() {
        let added = upsert(
            "config.toml",
            CONFIG,
            &["mcp_servers", "cairn"],
            &json!({"command": "cairn", "args": ["mcp"]}),
        )
        .unwrap();
        let back = remove(
            "config.toml",
            added.text().unwrap(),
            &["mcp_servers", "cairn"],
        )
        .unwrap();
        assert_eq!(back.text().unwrap(), CONFIG);
    }

    #[test]
    fn a_second_identical_write_is_unchanged() {
        let entry = json!({"command": "cairn", "args": ["mcp"]});
        let once = upsert("config.toml", CONFIG, &["mcp_servers", "cairn"], &entry).unwrap();
        let twice = upsert(
            "config.toml",
            once.text().unwrap(),
            &["mcp_servers", "cairn"],
            &entry,
        )
        .unwrap();
        assert_eq!(twice, Change::Unchanged);
    }

    #[test]
    fn a_crlf_file_keeps_crlf() {
        // toml_edit normalizes line endings; a file that uses CRLF throughout
        // must not silently change to LF (FR-152, SC-104).
        let src = "model = \"gpt-5\"\r\n\r\n[mcp_servers.one]\r\ncommand = \"one\"\r\n";
        let added = upsert(
            "config.toml",
            src,
            &["mcp_servers", "cairn"],
            &json!({"command": "cairn"}),
        )
        .unwrap();
        assert!(added.text().unwrap().contains("\r\n"));
        let back = remove(
            "config.toml",
            added.text().unwrap(),
            &["mcp_servers", "cairn"],
        )
        .unwrap();
        assert_eq!(back.text().unwrap(), src);
    }

    #[test]
    fn malformed_toml_fails_closed() {
        // FR-137: nothing is written and nothing is guessed.
        let bad = "model = \n[unclosed\n";
        let e = upsert("config.toml", bad, &["a"], &json!(1)).unwrap_err();
        assert!(matches!(e, EditError::Malformed { .. }));
    }

    #[test]
    fn a_truncated_table_header_is_malformed_rather_than_repaired() {
        let bad = "[mcp_servers.cairn\ncommand = \"cairn\"\n";
        assert!(read("config.toml", bad).is_err());
    }

    #[test]
    fn an_empty_file_gains_only_what_cairn_owns() {
        let out = upsert(
            "config.toml",
            "",
            &["mcp_servers", "cairn"],
            &json!({"command": "cairn"}),
        )
        .unwrap();
        let text = out.text().unwrap();
        assert!(text.contains("[mcp_servers.cairn]"));
        // The implicit parent table does not get a header of its own.
        assert!(!text.contains("[mcp_servers]\n"));
    }

    #[test]
    fn reading_a_nested_path_sees_what_was_written() {
        let out = upsert(
            "config.toml",
            CONFIG,
            &["mcp_servers", "cairn"],
            &json!({"command": "cairn", "args": ["mcp"]}),
        )
        .unwrap();
        let found = get(
            "config.toml",
            out.text().unwrap(),
            &["mcp_servers", "cairn", "command"],
        )
        .unwrap();
        assert_eq!(found, Some(json!("cairn")));
    }
}
