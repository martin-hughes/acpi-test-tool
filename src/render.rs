//! Backend-neutral presentation and searching of namespace entries.

use crate::backend::{Entry, Value};
use std::collections::BTreeMap;

fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('\\')
        .split('.')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Strip trailing underscores from a single name segment (`_SB_` -> `_SB`), as most projects do
/// when displaying ACPI names. A segment made only of underscores is left alone.
pub fn display_segment(seg: &str) -> &str {
    let trimmed = seg.trim_end_matches('_');
    if trimmed.is_empty() { seg } else { trimmed }
}

/// Apply [`display_segment`] to every segment of a path such as `\_SB_.PCI0`. Anything that is
/// not a segment (separators, `\`, `^` prefixes) is preserved.
pub fn display_path(path: &str) -> String {
    path.split('.')
        .map(display_segment)
        .collect::<Vec<_>>()
        .join(".")
}

fn format_entry(name: &str, e: &Entry) -> String {
    if e.summary.is_empty() {
        format!("{name} [{}]", e.type_name)
    } else {
        format!("{name} [{}] {}", e.type_name, e.summary)
    }
}

#[derive(Default)]
struct Node {
    entry: Option<Entry>,
    children: BTreeMap<String, Node>,
}

/// Render entries as an ASCII tree rooted at `\`.
pub fn render_tree(entries: &[Entry]) -> String {
    let mut root = Node::default();
    for e in entries {
        let mut node = &mut root;
        for seg in segments(&e.path) {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node.entry = Some(e.clone());
    }
    let mut out = String::from("\\\n");
    render_children(&root, "", &mut out);
    out
}

fn render_children(node: &Node, prefix: &str, out: &mut String) {
    let count = node.children.len();
    for (i, (name, child)) in node.children.iter().enumerate() {
        let last = i + 1 == count;
        let line = match &child.entry {
            Some(e) => format_entry(display_segment(name), e),
            None => display_segment(name).to_string(),
        };
        out.push_str(&format!(
            "{prefix}{}{line}\n",
            if last { "└── " } else { "├── " }
        ));
        let next = format!("{prefix}{}", if last { "    " } else { "│   " });
        render_children(child, &next, out);
    }
}

/// Pad a segment to four characters with `_`, as AML names are.
fn pad_segment(seg: &str) -> String {
    let mut s = seg.to_string();
    while s.len() < 4 {
        s.push('_');
    }
    s
}

/// Find entries matching `query`. A query starting with `\` must match a path exactly;
/// otherwise it matches any entry whose path ends with the given segments.
pub fn find<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let wanted: Vec<String> = segments(query).into_iter().map(pad_segment).collect();
    let absolute = query.starts_with('\\');
    let mut found: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let have = segments(&e.path);
            if absolute {
                have == wanted
            } else {
                !wanted.is_empty()
                    && have.len() >= wanted.len()
                    && have[have.len() - wanted.len()..] == wanted[..]
            }
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

pub fn render_found(found: &[&Entry]) -> String {
    found
        .iter()
        .map(|e| format_entry(&display_path(&e.path), e) + "\n")
        .collect()
}

/// Describe the value returned by executing the method at `path`.
pub fn render_result(path: &str, value: &Value) -> String {
    let path = display_path(path);
    if value.summary.is_empty() {
        format!("{path} returned [{}]\n", value.type_name)
    } else {
        format!("{path} returned [{}] {}\n", value.type_name, value.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, ty: &str, summary: &str) -> Entry {
        Entry {
            path: path.into(),
            type_name: ty.into(),
            summary: summary.into(),
        }
    }

    fn sample() -> Vec<Entry> {
        vec![
            e("\\_SB_", "Scope", ""),
            e("\\_SB_.PCI0", "Device", ""),
            e("\\_SB_.PCI0.FOO_", "Integer", "0x1 (1)"),
            e("\\_SB_.DEV0", "Device", ""),
            e("\\_SB_.DEV0.FOO_", "Integer", "0x2 (2)"),
        ]
    }

    #[test]
    fn tree_shape() {
        let expected = [
            "\\",
            "└── _SB [Scope]",
            "    ├── DEV0 [Device]",
            "    │   └── FOO [Integer] 0x2 (2)",
            "    └── PCI0 [Device]",
            "        └── FOO [Integer] 0x1 (1)",
            "",
        ]
        .join("\n");
        assert_eq!(render_tree(&sample()), expected);
    }

    #[test]
    fn strips_trailing_underscores_for_display() {
        assert_eq!(display_segment("_SB_"), "_SB");
        assert_eq!(display_segment("FOO_"), "FOO");
        assert_eq!(display_segment("PCI0"), "PCI0");
        assert_eq!(display_segment("____"), "____");
        assert_eq!(display_path("\\_SB_.PCI0.FOO_"), "\\_SB.PCI0.FOO");
        assert_eq!(display_path("\\"), "\\");
    }

    #[test]
    fn found_entries_are_displayed_without_trailing_underscores() {
        let entries = sample();
        let found = find(&entries, "DEV0.FOO_");
        assert_eq!(render_found(&found), "\\_SB.DEV0.FOO [Integer] 0x2 (2)\n");
    }

    #[test]
    fn displayed_names_can_be_used_as_queries() {
        let entries = sample();
        assert_eq!(find(&entries, "\\_SB.PCI0.FOO").len(), 1);
        assert_eq!(find(&entries, "FOO").len(), 2);
    }

    #[test]
    fn find_absolute() {
        let entries = sample();
        let found = find(&entries, "\\_SB_.PCI0");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "\\_SB_.PCI0");
    }

    #[test]
    fn find_bare_segment_returns_all_matches() {
        let entries = sample();
        assert_eq!(find(&entries, "FOO_").len(), 2);
    }

    #[test]
    fn find_pads_short_names_and_matches_relative_paths() {
        let entries = sample();
        assert_eq!(find(&entries, "_SB").len(), 1);
        assert_eq!(find(&entries, "DEV0.FOO_").len(), 1);
        assert!(find(&entries, "NOPE").is_empty());
        assert!(find(&entries, "").is_empty());
    }
}
