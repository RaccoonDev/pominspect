//! The inspection model: what pominspect knows about a Maven build after
//! Maven has produced the Effective POM and dependency tree for each module.
//!
//! This model is independent of any rendering so it can later be exposed as
//! JSON as well as through the TUI.

use std::fmt;

/// A place in a POM that Maven reports as the Origin of a value,
/// e.g. `com.fasterxml.jackson:jackson-bom:2.17.1, line 96`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The POM's coordinates as Maven prints them (`group:artifact:version`).
    pub pom: String,
    pub line: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, line {}", self.pom, self.line)
    }
}

/// `groupId:artifactId` — identifies an artifact independent of its version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub group: String,
    pub artifact: String,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.group, self.artifact)
    }
}

/// Coordinates of a module of the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gav {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub packaging: String,
}

impl fmt::Display for Gav {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.group, self.artifact, self.version)
    }
}

/// A dependency entry of the Effective POM, either from `<dependencies>`
/// or from `<dependencyManagement>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredDependency {
    pub group: String,
    pub artifact: String,
    pub version: Option<String>,
    pub scope: Option<String>,
    /// Where the dependency was declared (the location of its `artifactId`).
    pub declared_at: Option<Location>,
    /// Where its version value came from.
    pub version_at: Option<Location>,
}

impl DeclaredDependency {
    pub fn key(&self) -> Key {
        Key {
            group: self.group.clone(),
            artifact: self.artifact.clone(),
        }
    }
}

/// Why Maven left a node out of the resolved dependency set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Omission {
    Duplicate,
    Conflict { winner: String },
    Cycle,
    Other(String),
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Omission::Duplicate => write!(f, "omitted for duplicate"),
            Omission::Conflict { winner } => write!(f, "omitted for conflict with {winner}"),
            Omission::Cycle => write!(f, "omitted for cycle"),
            Omission::Other(s) => write!(f, "{s}"),
        }
    }
}

/// One node of a module's dependency tree, as reported by Maven.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepNode {
    pub group: String,
    pub artifact: String,
    pub kind: String,
    pub classifier: Option<String>,
    pub version: String,
    pub scope: String,
    /// The version this node had before `dependencyManagement` overrode it.
    pub managed_from: Option<String>,
    pub omitted: Option<Omission>,
    /// Remaining annotations Maven attached (e.g. `optional`, `scope updated from test`).
    pub notes: Vec<String>,
    pub children: Vec<DepNode>,
}

impl DepNode {
    pub fn key(&self) -> Key {
        Key {
            group: self.group.clone(),
            artifact: self.artifact.clone(),
        }
    }

    pub fn is_conflict_loser(&self) -> bool {
        matches!(self.omitted, Some(Omission::Conflict { .. }))
    }

    /// `group:artifact:version` (plus classifier if any).
    pub fn label(&self) -> String {
        match &self.classifier {
            Some(c) => format!("{}:{}:{}:{}", self.group, self.artifact, c, self.version),
            None => format!("{}:{}:{}", self.group, self.artifact, self.version),
        }
    }
}

/// A module of the build after inspection.
#[derive(Debug, Clone)]
pub struct Module {
    pub coords: Gav,
    /// Direct dependencies from the Effective POM.
    pub declared: Vec<DeclaredDependency>,
    /// `dependencyManagement` entries from the Effective POM.
    pub managed: Vec<DeclaredDependency>,
    /// The resolved dependency tree (children of the module itself).
    pub tree: Vec<DepNode>,
    /// Set when Maven could not inspect this module.
    pub error: Option<String>,
}

impl Module {
    /// Follows an index path from the module's tree roots.
    pub fn node(&self, path: &[usize]) -> Option<&DepNode> {
        let (first, rest) = path.split_first()?;
        let mut node = self.tree.get(*first)?;
        for i in rest {
            node = node.children.get(*i)?;
        }
        Some(node)
    }

    /// The nodes along an index path, from the tree root to the target.
    pub fn chain(&self, path: &[usize]) -> Vec<&DepNode> {
        (1..=path.len())
            .filter_map(|n| self.node(&path[..n]))
            .collect()
    }

    /// Every node in the tree with the given key, with its index path.
    pub fn occurrences(&self, key: &Key) -> Vec<(Vec<usize>, &DepNode)> {
        let mut out = Vec::new();
        let mut path = Vec::new();
        fn walk<'a>(
            nodes: &'a [DepNode],
            key: &Key,
            path: &mut Vec<usize>,
            out: &mut Vec<(Vec<usize>, &'a DepNode)>,
        ) {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if n.group == key.group && n.artifact == key.artifact {
                    out.push((path.clone(), n));
                }
                walk(&n.children, key, path, out);
                path.pop();
            }
        }
        walk(&self.tree, key, &mut path, &mut out);
        out
    }

    /// True if some occurrence of `key` lost a version conflict.
    pub fn has_conflict(&self, key: &Key) -> bool {
        self.occurrences(key)
            .iter()
            .any(|(_, n)| n.is_conflict_loser())
    }

    /// Every key that has at least one conflict-losing occurrence.
    pub fn conflicts(&self) -> Vec<Key> {
        let mut keys = Vec::new();
        fn walk(nodes: &[DepNode], keys: &mut Vec<Key>) {
            for n in nodes {
                if n.is_conflict_loser() && !keys.contains(&n.key()) {
                    keys.push(n.key());
                }
                walk(&n.children, keys);
            }
        }
        walk(&self.tree, &mut keys);
        keys
    }

    /// Why the node at `path` is present and has its version.
    pub fn origin(&self, path: &[usize]) -> Option<Origin> {
        let chain = self.chain(path);
        let node = *chain.last()?;
        let key = node.key();
        if chain.len() == 1 {
            let declared = self.declared.iter().find(|d| d.key() == key);
            return Some(Origin {
                presence: Presence::Direct {
                    at: declared.and_then(|d| d.declared_at.clone()),
                },
                version: match (&node.managed_from, self.managed_entry(&key)) {
                    (Some(from), Some(m)) => VersionOrigin::Managed {
                        from: from.clone(),
                        at: m.version_at.clone(),
                    },
                    _ => VersionOrigin::Declared {
                        at: declared.and_then(|d| d.version_at.clone()),
                    },
                },
            });
        }
        let parent = chain[chain.len() - 2];
        Some(Origin {
            presence: Presence::Transitive {
                via: parent.label(),
            },
            version: match &node.managed_from {
                Some(from) => VersionOrigin::Managed {
                    from: from.clone(),
                    at: self.managed_entry(&key).and_then(|m| m.version_at.clone()),
                },
                None => VersionOrigin::FromParent {
                    via: parent.label(),
                },
            },
        })
    }

    fn managed_entry(&self, key: &Key) -> Option<&DeclaredDependency> {
        self.managed.iter().find(|m| &m.key() == key)
    }
}

/// The Origin of a dependency node: why it is there and why it has its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub presence: Presence,
    pub version: VersionOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Declared directly by the module (possibly inherited from a parent POM).
    Direct { at: Option<Location> },
    /// Pulled in by another dependency.
    Transitive { via: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionOrigin {
    /// The version written in the Effective POM; `at` is where the value came from
    /// (which may be a parent POM, a property, or an imported BOM).
    Declared { at: Option<Location> },
    /// `dependencyManagement` replaced the version `from` with the managed one.
    Managed { from: String, at: Option<Location> },
    /// The version the parent dependency's own POM asked for.
    FromParent { via: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::assemble;

    fn web() -> Module {
        let eff = include_str!("../tests/fixtures/maven-output/effective-pom.xml");
        let deps = include_str!("../tests/fixtures/maven-output/deps.txt");
        assemble(eff, deps, &[]).unwrap().remove(2)
    }

    fn loc(pom: &str, line: u32) -> Option<Location> {
        Some(Location {
            pom: pom.into(),
            line,
        })
    }

    #[test]
    fn direct_dependency_origin() {
        let m = web();
        // web → commons-lang3 (index 2)
        assert_eq!(
            m.origin(&[2]).unwrap(),
            Origin {
                presence: Presence::Direct {
                    at: loc("com.example:web:1.0.0", 23)
                },
                version: VersionOrigin::Declared {
                    at: loc("com.example:web:1.0.0", 24)
                },
            }
        );
    }

    #[test]
    fn managed_transitive_origin_points_at_dependency_management() {
        let m = web();
        // web → logback-classic → slf4j-api (managed from 2.0.7)
        let o = m.origin(&[1, 1]).unwrap();
        assert_eq!(
            o.presence,
            Presence::Transitive {
                via: "ch.qos.logback:logback-classic:1.4.14".into()
            }
        );
        assert_eq!(
            o.version,
            VersionOrigin::Managed {
                from: "2.0.7".into(),
                at: loc("com.example:multi:1.0.0", 27)
            }
        );
    }

    #[test]
    fn unmanaged_transitive_version_comes_from_parent() {
        let m = web();
        let o = m.origin(&[1, 0]).unwrap();
        assert_eq!(
            o.version,
            VersionOrigin::FromParent {
                via: "ch.qos.logback:logback-classic:1.4.14".into()
            }
        );
    }

    #[test]
    fn finds_conflicts_and_occurrences() {
        let m = web();
        let lang = Key {
            group: "org.apache.commons".into(),
            artifact: "commons-lang3".into(),
        };
        assert_eq!(m.conflicts(), vec![lang.clone()]);
        let paths: Vec<_> = m.occurrences(&lang).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec![vec![2], vec![3, 0]]);
    }
}
