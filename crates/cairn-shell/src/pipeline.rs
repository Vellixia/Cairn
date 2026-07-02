//! The compression pipeline primitives. Pure functions - no I/O, no state, easy to test.
//!
//! Three composable operations:
//! 1. [`filter_keep`] / [`filter_drop`] - keep or drop lines by substring/prefix
//! 2. [`dedup_consecutive`] - collapse runs of identical adjacent lines
//! 3. [`truncate_head_tail`] - head + tail truncate with an `expand` hint

/// Keep only lines containing one of `needles` (case-insensitive).
pub fn filter_keep(lines: &[&str], needles: &[&str]) -> Vec<String> {
    let lowered: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
    lines
        .iter()
        .filter(|l| {
            let low = l.to_lowercase();
            lowered.iter().any(|n| low.contains(n))
        })
        .map(|s| s.to_string())
        .collect()
}

/// Drop lines that start with any of `prefixes`.
pub fn filter_drop(lines: &[&str], prefixes: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !prefixes.iter().any(|p| l.starts_with(p)))
        .map(|s| s.to_string())
        .collect()
}

/// Collapse runs of identical consecutive lines into `line  (xN)`.
pub fn dedup_consecutive(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j] == lines[i] {
            j += 1;
        }
        let count = j - i;
        if count > 1 {
            out.push(format!("{}  (x{count})", lines[i]));
        } else {
            out.push(lines[i].clone());
        }
        i = j;
    }
    out
}

/// If `lines` exceeds `limit`, keep `head` + `tail` with an omission marker between.
pub fn truncate_head_tail(lines: &[String], limit: usize, head: usize, tail: usize) -> Vec<String> {
    if lines.len() <= limit || head + tail >= lines.len() {
        return lines.to_vec();
    }
    let omitted = lines.len() - head - tail;
    let mut out = Vec::with_capacity(head + tail + 1);
    out.extend(lines[..head].iter().cloned());
    out.push(format!(
        "... {omitted} lines omitted (recover the full output with `expand`) ..."
    ));
    out.extend(lines[lines.len() - tail..].iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_keep_case_insensitive() {
        let lines = vec!["test PASSED", "hello", "TEST FAILED", "world"];
        let out = filter_keep(&lines, &["test"]);
        assert_eq!(out, vec!["test PASSED", "TEST FAILED"]);
    }

    #[test]
    fn filter_keep_lowercases_needle_too() {
        let lines = vec!["< HTTP/1.1 200 OK", "<html></html>"];
        let out = filter_keep(&lines, &["HTTP/"]);
        assert_eq!(out, vec!["< HTTP/1.1 200 OK"]);
    }

    #[test]
    fn filter_drop_drops_by_prefix() {
        let lines = vec!["ok 1", "  (use git add)", "ok 2", "  (use git commit)"];
        let out = filter_drop(&lines, &["  (use "]);
        assert_eq!(out, vec!["ok 1", "ok 2"]);
    }

    #[test]
    fn dedup_consecutive_collapses_runs() {
        let lines = vec![
            "a".into(),
            "a".into(),
            "a".into(),
            "b".into(),
            "c".into(),
            "c".into(),
        ];
        let out = dedup_consecutive(&lines);
        assert_eq!(out, vec!["a  (x3)", "b", "c  (x2)"]);
    }

    #[test]
    fn truncate_head_tail_omits_middle() {
        let lines: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(out.len(), 60 + 1 + 40);
        assert!(out[60].contains("omitted"));
    }

    #[test]
    fn truncate_head_tail_no_op_for_short_input() {
        let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let out = truncate_head_tail(&lines, 200, 60, 40);
        assert_eq!(out.len(), 5);
    }
}
