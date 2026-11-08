//! Parser for `maven-dependency-plugin:tree -Dverbose` text output.
//!
//! With `-DappendOutput=true` the file holds one tree per module; each tree
//! starts with an unindented line holding the module's coordinates.

use crate::model::{DepNode, Gav, Omission};
use anyhow::{Context, Result, bail};

/// A module's coordinates and the children of its tree root.
#[derive(Debug)]
pub struct ModuleTree {
    pub coords: Gav,
    pub children: Vec<DepNode>,
}

pub fn parse(text: &str) -> Result<Vec<ModuleTree>> {
    let mut trees: Vec<ModuleTree> = Vec::new();
    // Stack of (depth, node) for the tree under construction.
    let mut stack: Vec<(usize, DepNode)> = Vec::new();

    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ctx = || format!("dependency tree line {}: {line}", lineno + 1);
        match marker(line) {
            None => {
                if let Some(t) = trees.last_mut() {
                    collapse(&mut stack, 0, &mut t.children);
                }
                trees.push(ModuleTree { coords: parse_root(line).with_context(ctx)?, children: Vec::new() });
            }
            Some((depth, content)) => {
                let Some(tree) = trees.last_mut() else { bail!("{}: no module root", ctx()) };
                collapse(&mut stack, depth, &mut tree.children);
                if depth > stack.len() {
                    bail!("{}: skipped a nesting level", ctx());
                }
                stack.push((depth, parse_node(content).with_context(ctx)?));
            }
        }
    }
    if let Some(t) = trees.last_mut() {
        collapse(&mut stack, 0, &mut t.children);
    }
    Ok(trees)
}

/// Pops every node at `depth` or deeper, attaching each to its parent.
fn collapse(stack: &mut Vec<(usize, DepNode)>, depth: usize, roots: &mut Vec<DepNode>) {
    while stack.last().is_some_and(|(d, _)| *d >= depth) {
        let (_, node) = stack.pop().unwrap();
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(node),
            None => roots.push(node),
        }
    }
}

/// Finds the `+- ` / `\- ` marker; returns the depth (0 = direct child of the root)
/// and the text after it.
fn marker(line: &str) -> Option<(usize, &str)> {
    let idx = [line.find("+- "), line.find("\\- ")].into_iter().flatten().min()?;
    let prefix = &line[..idx];
    if !prefix.chars().all(|c| c == ' ' || c == '|') {
        return None;
    }
    Some((idx / 3, &line[idx + 3..]))
}

fn parse_root(line: &str) -> Result<Gav> {
    let parts: Vec<&str> = line.trim().split(':').collect();
    match parts.as_slice() {
        [g, a, p, v] => Ok(Gav {
            group: g.to_string(),
            artifact: a.to_string(),
            packaging: p.to_string(),
            version: v.to_string(),
        }),
        _ => bail!("expected group:artifact:packaging:version"),
    }
}

fn parse_node(content: &str) -> Result<DepNode> {
    let content = content.trim();
    let (coords, notes) = if let Some(inner) = content.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        // Omitted node: `(coords - note; note)`
        match inner.split_once(" - ") {
            Some((c, n)) => (c, n),
            None => (inner, ""),
        }
    } else if let Some((c, rest)) = content.split_once(" (") {
        (c, rest.strip_suffix(')').unwrap_or(rest))
    } else {
        (content, "")
    };

    let parts: Vec<&str> = coords.split(':').collect();
    let (group, artifact, kind, classifier, version, scope) = match parts.as_slice() {
        [g, a, t, v, s] => (g, a, t, None, v, s),
        [g, a, t, c, v, s] => (g, a, t, Some(c.to_string()), v, s),
        _ => bail!("unexpected coordinates `{coords}`"),
    };

    let mut node = DepNode {
        group: group.to_string(),
        artifact: artifact.to_string(),
        kind: kind.to_string(),
        classifier,
        version: version.to_string(),
        scope: scope.to_string(),
        managed_from: None,
        omitted: None,
        notes: Vec::new(),
        children: Vec::new(),
    };
    for note in notes.split(';').map(str::trim).filter(|n| !n.is_empty()) {
        if let Some(v) = note.strip_prefix("version managed from ") {
            node.managed_from = Some(v.to_string());
        } else if note == "omitted for duplicate" {
            node.omitted = Some(Omission::Duplicate);
        } else if let Some(w) = note.strip_prefix("omitted for conflict with ") {
            node.omitted = Some(Omission::Conflict { winner: w.to_string() });
        } else if note == "omitted for cycle" {
            node.omitted = Some(Omission::Cycle);
        } else if note.starts_with("omitted") {
            node.omitted = Some(Omission::Other(note.to_string()));
        } else {
            node.notes.push(note.to_string());
        }
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/maven-output/deps.txt");

    #[test]
    fn splits_appended_output_into_modules() {
        let trees = parse(FIXTURE).unwrap();
        let names: Vec<_> = trees.iter().map(|t| t.coords.artifact.as_str()).collect();
        assert_eq!(names, ["multi", "core", "web"]);
        assert!(trees[0].children.is_empty());
        assert_eq!(trees[2].coords.packaging, "jar");
    }

    #[test]
    fn builds_nested_structure() {
        let web = &parse(FIXTURE).unwrap()[2];
        let top: Vec<_> = web.children.iter().map(|n| n.artifact.as_str()).collect();
        assert_eq!(top, ["core", "logback-classic", "commons-lang3", "commons-text", "junit"]);
        let databind = &web.children[0].children[0];
        assert_eq!(databind.artifact, "jackson-databind");
        assert_eq!(databind.children.len(), 2);
        assert_eq!(web.children[4].children[0].artifact, "hamcrest-core");
        assert_eq!(web.children[4].scope, "test");
    }

    #[test]
    fn reads_conflicts_and_management() {
        let web = &parse(FIXTURE).unwrap()[2];
        let lang_via_text = &web.children[3].children[0];
        assert_eq!(lang_via_text.version, "3.14.0");
        assert_eq!(lang_via_text.omitted, Some(Omission::Conflict { winner: "3.12.0".into() }));

        let slf4j_via_logback = &web.children[1].children[1];
        assert_eq!(slf4j_via_logback.omitted, Some(Omission::Duplicate));
        assert_eq!(slf4j_via_logback.managed_from.as_deref(), Some("2.0.7"));
    }

    #[test]
    fn reads_classifier_and_plain_notes() {
        let n = parse_node("io.netty:netty-tcnative:jar:linux-x86_64:2.0.0:runtime (optional)").unwrap();
        assert_eq!(n.classifier.as_deref(), Some("linux-x86_64"));
        assert_eq!(n.version, "2.0.0");
        assert_eq!(n.notes, ["optional"]);
    }
}
