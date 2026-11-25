use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use voltshot_core::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndexFormat {
    #[default]
    PlainText,
    Markdown,
    Html,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexOptions {
    pub max_depth: usize,
    pub include_hidden: bool,
    pub format: IndexFormat,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            max_depth: usize::MAX,
            include_hidden: false,
            format: IndexFormat::PlainText,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryNode {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub children: Vec<DirectoryNode>,
}

pub fn scan_directory(root: &Path, opts: &IndexOptions) -> Result<DirectoryNode, CoreError> {
    let meta = fs::metadata(root)?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    if !meta.is_dir() {
        return Ok(DirectoryNode {
            name,
            is_dir: false,
            size_bytes: meta.len(),
            children: Vec::new(),
        });
    }

    scan_dir_recursive(root, name, 0, opts)
}

fn scan_dir_recursive(
    path: &Path,
    name: String,
    depth: usize,
    opts: &IndexOptions,
) -> Result<DirectoryNode, CoreError> {
    let mut children = Vec::new();
    let mut total_size = 0u64;

    if depth < opts.max_depth {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child_name = entry.file_name().to_string_lossy().into_owned();
            if !opts.include_hidden && child_name.starts_with('.') {
                continue;
            }
            let file_type = entry.file_type()?;
            let child = if file_type.is_dir() && !file_type.is_symlink() {
                scan_dir_recursive(&entry.path(), child_name, depth + 1, opts)?
            } else {
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                DirectoryNode {
                    name: child_name,
                    is_dir: false,
                    size_bytes,
                    children: Vec::new(),
                }
            };
            total_size = total_size.saturating_add(child.size_bytes);
            children.push(child);
        }

        children.sort_by(|a, b| {
            (!a.is_dir, a.name.to_lowercase(), &a.name).cmp(&(
                !b.is_dir,
                b.name.to_lowercase(),
                &b.name,
            ))
        });
    }

    Ok(DirectoryNode {
        name,
        is_dir: true,
        size_bytes: total_size,
        children,
    })
}

pub fn render_index(node: &DirectoryNode, format: IndexFormat) -> Result<String, CoreError> {
    match format {
        IndexFormat::PlainText => {
            let mut out = String::new();
            render_plain_text(node, 0, &mut out);
            Ok(out)
        }
        IndexFormat::Markdown => {
            let mut out = String::new();
            render_markdown(node, 0, &mut out);
            Ok(out)
        }
        IndexFormat::Html => {
            let escaped_title = escape_html(&node.name);
            let mut out = String::new();
            let _ = writeln!(
                &mut out,
                "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{escaped_title}</title></head>\n<body>\n<ul>"
            );
            render_html_node(node, 1, &mut out);
            out.push_str("</ul>\n</body>\n</html>\n");
            Ok(out)
        }
        IndexFormat::Json => serde_json::to_string_pretty(node)
            .map_err(|err| CoreError::ConfigError(err.to_string())),
    }
}

fn render_plain_text(node: &DirectoryNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    if node.is_dir {
        let _ = writeln!(out, "{indent}{}/ ({} B)", node.name, node.size_bytes);
        for child in &node.children {
            render_plain_text(child, depth + 1, out);
        }
    } else {
        let _ = writeln!(out, "{indent}{} ({} B)", node.name, node.size_bytes);
    }
}

fn render_markdown(node: &DirectoryNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    if node.is_dir {
        let _ = writeln!(out, "{indent}- **{}/** ({} B)", node.name, node.size_bytes);
        for child in &node.children {
            render_markdown(child, depth + 1, out);
        }
    } else {
        let _ = writeln!(out, "{indent}- {} ({} B)", node.name, node.size_bytes);
    }
}

fn render_html_node(node: &DirectoryNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let escaped = escape_html(&node.name);
    if node.is_dir {
        if node.children.is_empty() {
            let _ = writeln!(
                out,
                "{indent}<li><strong>{escaped}/</strong> ({} B)</li>",
                node.size_bytes
            );
        } else {
            let _ = writeln!(
                out,
                "{indent}<li><strong>{escaped}/</strong> ({} B)",
                node.size_bytes
            );
            let _ = writeln!(out, "{indent}  <ul>");
            for child in &node.children {
                render_html_node(child, depth + 2, out);
            }
            let _ = writeln!(out, "{indent}  </ul>");
            let _ = writeln!(out, "{indent}</li>");
        }
    } else {
        let _ = writeln!(out, "{indent}<li>{escaped} ({} B)</li>", node.size_bytes);
    }
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scans_multilevel_tree_deterministically_respecting_hidden_and_depth() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        fs::create_dir_all(root.join("alpha/nested")).expect("create alpha/nested");
        fs::create_dir_all(root.join("beta")).expect("create beta");
        fs::write(root.join("zeta.txt"), b"12345").expect("write zeta");
        fs::write(root.join("apple.txt"), b"12").expect("write apple");
        fs::write(root.join(".secret"), b"hidden").expect("write hidden");
        fs::write(root.join("alpha/mid.txt"), b"abc").expect("write mid");
        fs::write(root.join("alpha/nested/deep.txt"), b"abcdef").expect("write deep");

        let opts = IndexOptions {
            max_depth: 10,
            include_hidden: false,
            format: IndexFormat::PlainText,
        };
        let tree = scan_directory(root, &opts).expect("scan full tree");
        assert!(tree.is_dir);
        let child_names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(child_names, vec!["alpha", "beta", "apple.txt", "zeta.txt"]);
        assert_eq!(tree.size_bytes, 5 + 2 + 3 + 6);

        let alpha = &tree.children[0];
        assert_eq!(alpha.size_bytes, 3 + 6);
        let alpha_children: Vec<&str> = alpha.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(alpha_children, vec!["nested", "mid.txt"]);

        let with_hidden = scan_directory(
            root,
            &IndexOptions {
                include_hidden: true,
                ..opts
            },
        )
        .expect("scan with hidden");
        assert!(with_hidden.children.iter().any(|c| c.name == ".secret"));

        let shallow = scan_directory(
            root,
            &IndexOptions {
                max_depth: 1,
                include_hidden: false,
                format: IndexFormat::PlainText,
            },
        )
        .expect("scan depth 1");
        assert!(shallow.children[0].children.is_empty());
    }

    #[test]
    fn renders_plain_text_markdown_json_and_escaped_html() {
        let node = DirectoryNode {
            name: "root & <dir>".to_owned(),
            is_dir: true,
            size_bytes: 10,
            children: vec![DirectoryNode {
                name: "a\"b'c<script>.txt".to_owned(),
                is_dir: false,
                size_bytes: 10,
                children: Vec::new(),
            }],
        };

        let txt = render_index(&node, IndexFormat::PlainText).expect("render txt");
        assert!(txt.contains("root & <dir>/ (10 B)"));
        assert!(txt.contains("  a\"b'c<script>.txt (10 B)"));

        let md = render_index(&node, IndexFormat::Markdown).expect("render md");
        assert!(md.contains("- **root & <dir>/** (10 B)"));
        assert!(md.contains("  - a\"b'c<script>.txt (10 B)"));

        let html = render_index(&node, IndexFormat::Html).expect("render html");
        assert!(html.contains("root &amp; &lt;dir&gt;"));
        assert!(html.contains("a&quot;b&#39;c&lt;script&gt;.txt"));
        assert!(!html.contains("<script>"));

        let json = render_index(&node, IndexFormat::Json).expect("render json");
        let parsed: DirectoryNode = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed, node);
    }
}
