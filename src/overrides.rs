//! Version Overrides: Requester → library edges where the build resolves a
//! different version than the Requester asked for (see GLOSSARY.md).

use crate::model::{DepNode, Key, Location, Module, Omission};

/// How the resolved version differs from the requested one, from most to
/// least risky. Graded by [`compare`], a heuristic (see ADR-0001's exception).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Downgrade,
    MajorUpgrade,
    MinorUpgrade,
    PatchUpgrade,
    Unclassified,
}

impl Change {
    pub fn label(self) -> &'static str {
        match self {
            Change::Downgrade => "Downgrade",
            Change::MajorUpgrade => "Major Upgrade",
            Change::MinorUpgrade => "Minor Upgrade",
            Change::PatchUpgrade => "Patch Upgrade",
            Change::Unclassified => "Unclassified",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Change::Downgrade => "downgrade",
            Change::MajorUpgrade => "major",
            Change::MinorUpgrade => "minor",
            Change::PatchUpgrade => "patch",
            Change::Unclassified => "unclassified",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Change::Downgrade => "▼",
            Change::Unclassified => "?",
            _ => "▲",
        }
    }
}

/// Why the Requester didn't get the version it asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// Maven's Mediation picked another path's version ("omitted for conflict").
    Mediation,
    /// A `dependencyManagement` entry or imported BOM replaced the version;
    /// `at` is where the managed version is written.
    Management { at: Option<Location> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionOverride {
    /// The Requester's `group:artifact:version` (the module's own, for direct dependencies).
    pub requester: String,
    pub library: Key,
    pub requested: String,
    pub resolved: String,
    pub change: Change,
    pub cause: Cause,
    /// The dependency scope of the edge.
    pub scope: String,
    /// Index paths into the module's tree where this edge appears.
    pub paths: Vec<Vec<usize>>,
}

impl VersionOverride {
    /// True when the edge only affects the test classpath or a container-provided jar.
    pub fn is_reduced_scope(&self) -> bool {
        self.scope == "test" || self.scope == "provided"
    }

    /// Risk rank used for warnings and ordering (0 = highest): the [`Change`],
    /// one level lower for test or provided scope.
    pub fn rank(&self) -> u8 {
        let base = self.change as u8;
        if self.is_reduced_scope() && self.change != Change::Unclassified {
            (base + 1).min(Change::PatchUpgrade as u8)
        } else {
            base
        }
    }

    /// Downgrades and Major Upgrades (after the scope adjustment) are warnings.
    pub fn is_warning(&self) -> bool {
        self.rank() <= Change::MajorUpgrade as u8
    }
}

/// Finds every Version Override in a module's dependency tree, most risky first.
pub fn find(module: &Module) -> Vec<VersionOverride> {
    let mut out: Vec<VersionOverride> = Vec::new();
    let mut path = Vec::new();
    walk(
        module,
        &module.tree,
        &module.coords.to_string(),
        &mut path,
        &mut out,
    );
    out.sort_by(|a, b| {
        (a.change, a.rank(), &a.library, &a.requester).cmp(&(
            b.change,
            b.rank(),
            &b.library,
            &b.requester,
        ))
    });
    out
}

fn walk(
    module: &Module,
    nodes: &[DepNode],
    requester: &str,
    path: &mut Vec<usize>,
    out: &mut Vec<VersionOverride>,
) {
    for (i, node) in nodes.iter().enumerate() {
        path.push(i);
        if let Some(found) = edge_override(module, node, requester, path) {
            match out
                .iter_mut()
                .find(|o| o.requester == found.requester && o.library == found.library)
            {
                Some(existing) => {
                    existing.paths.push(path.clone());
                    if existing.is_reduced_scope() && !found.is_reduced_scope() {
                        existing.scope = found.scope;
                    }
                }
                None => out.push(found),
            }
        }
        // Omitted nodes have no children in Maven's output, so a Requester that
        // lost Mediation never contributes overrides.
        walk(module, &node.children, &node.label(), path, out);
        path.pop();
    }
}

fn edge_override(
    module: &Module,
    node: &DepNode,
    requester: &str,
    path: &[usize],
) -> Option<VersionOverride> {
    let requested = node
        .managed_from
        .clone()
        .unwrap_or_else(|| node.version.clone());
    let (resolved, cause) = match &node.omitted {
        Some(Omission::Conflict { winner }) => (winner.clone(), Cause::Mediation),
        Some(Omission::Cycle | Omission::Other(_)) => return None,
        _ => (
            node.version.clone(),
            Cause::Management {
                at: module
                    .managed_entry(&node.key())
                    .and_then(|m| m.version_at.clone()),
            },
        ),
    };
    if requested == resolved {
        return None;
    }
    Some(VersionOverride {
        requester: requester.to_string(),
        library: node.key(),
        change: compare(&requested, &resolved),
        requested,
        resolved,
        cause,
        scope: node.scope.clone(),
        paths: vec![path.to_vec()],
    })
}

/// Grades a version change by the leading numeric segments of both versions
/// (`32.1.2-jre` → `[32, 1, 2]`). Below 1.0 a second-segment change counts as
/// major. Versions whose numbers are equal but whose text differs (qualifiers),
/// or that don't start with a number, are Unclassified.
pub fn compare(requested: &str, resolved: &str) -> Change {
    let (Some(a), Some(b)) = (numeric_prefix(requested), numeric_prefix(resolved)) else {
        return Change::Unclassified;
    };
    let len = a.len().max(b.len());
    let seg = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    let Some(i) = (0..len).find(|&i| seg(&a, i) != seg(&b, i)) else {
        return Change::Unclassified;
    };
    if seg(&b, i) < seg(&a, i) {
        return Change::Downgrade;
    }
    match i {
        0 => Change::MajorUpgrade,
        1 if seg(&a, 0) == 0 => Change::MajorUpgrade,
        1 => Change::MinorUpgrade,
        _ => Change::PatchUpgrade,
    }
}

fn numeric_prefix(version: &str) -> Option<Vec<u64>> {
    let nums: Vec<u64> = version
        .split(['.', '-'])
        .map_while(|s| s.parse().ok())
        .collect();
    (!nums.is_empty()).then_some(nums)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::assemble;
    use crate::model::Gav;
    use crate::parse::tree;

    fn modules() -> Vec<Module> {
        assemble(
            include_str!("../tests/fixtures/maven-output/effective-pom.xml"),
            include_str!("../tests/fixtures/maven-output/deps.txt"),
            &[],
        )
        .unwrap()
    }

    fn summary(m: &Module) -> Vec<(String, String, String, String, Change, bool)> {
        m.overrides
            .iter()
            .map(|o| {
                (
                    o.requester.clone(),
                    o.library.artifact.clone(),
                    o.requested.clone(),
                    o.resolved.clone(),
                    o.change,
                    o.is_warning(),
                )
            })
            .collect()
    }

    fn row(
        req: &str,
        lib: &str,
        asked: &str,
        gets: &str,
        c: Change,
        w: bool,
    ) -> (String, String, String, String, Change, bool) {
        (req.into(), lib.into(), asked.into(), gets.into(), c, w)
    }

    #[test]
    fn grades_version_changes() {
        use Change::*;
        let cases = [
            ("2.4.1", "3.0.0", MajorUpgrade),
            ("20.0", "33.2.1-jre", MajorUpgrade),
            ("3.14.0", "3.12.0", Downgrade),
            ("1.3", "1.1", Downgrade),
            ("2.0.7", "2.0.13", PatchUpgrade),
            ("1.2", "1.3.0", MinorUpgrade),
            ("2.0", "2.0.0.1", PatchUpgrade),
            ("0.3.1", "0.4.0", MajorUpgrade),
            ("0.0.3", "0.0.4", PatchUpgrade),
            ("1.0-RC1", "1.0", Unclassified),
            ("33.0-jre", "33.0-android", Unclassified),
            ("2.0", "2.0.0", Unclassified),
            ("latest", "1.0", Unclassified),
        ];
        for (a, b, want) in cases {
            assert_eq!(compare(a, b), want, "{a} → {b}");
        }
    }

    #[test]
    fn finds_management_override_in_core() {
        let core = &modules()[1];
        assert_eq!(
            summary(core),
            [row(
                "org.reflections:reflections:0.9.11",
                "guava",
                "20.0",
                "33.2.1-jre",
                Change::MajorUpgrade,
                true
            )]
        );
        assert!(
            matches!(&core.overrides[0].cause, Cause::Management { at: Some(l) } if l.pom == "com.example:multi:1.0.0")
        );
    }

    #[test]
    fn finds_every_override_in_web_most_risky_first() {
        let web = &modules()[2];
        assert_eq!(
            summary(web),
            [
                row(
                    "org.apache.commons:commons-text:1.12.0",
                    "commons-lang3",
                    "3.14.0",
                    "3.12.0",
                    Change::Downgrade,
                    true
                ),
                row(
                    "junit:junit:4.13.2",
                    "hamcrest-core",
                    "1.3",
                    "1.1",
                    Change::Downgrade,
                    true
                ),
                row(
                    "org.reflections:reflections:0.9.11",
                    "guava",
                    "20.0",
                    "33.2.1-jre",
                    Change::MajorUpgrade,
                    true
                ),
                row(
                    "ch.qos.logback:logback-classic:1.4.14",
                    "slf4j-api",
                    "2.0.7",
                    "2.0.13",
                    Change::PatchUpgrade,
                    false
                ),
            ]
        );
        assert_eq!(web.overrides[0].cause, Cause::Mediation);
        assert_eq!(web.overrides[0].paths, vec![vec![3, 0]]);
        // The test-scope downgrade ranks one level lower but is still a warning.
        assert_eq!(web.overrides[1].rank(), 1);
    }

    #[test]
    fn module_without_overrides() {
        assert!(modules()[0].overrides.is_empty());
    }

    #[test]
    fn same_requester_on_several_paths_is_one_override() {
        let text = "\
com.example:app:jar:1.0
+- com.example:a:jar:1.0:test
|  \\- corp:dep:jar:1.4:test
|     \\- aws:sdk:jar:3.0:test (version managed from 2.4)
+- com.example:b:jar:1.0:compile
|  \\- corp:dep:jar:1.4:compile
|     \\- aws:sdk:jar:3.0:compile (version managed from 2.4)
+- com.example:d:jar:1.0:compile
|  \\- (corp:dep:jar:1.4:compile - omitted for duplicate)
\\- com.example:c:jar:1.0:compile
   \\- (corp:dep:jar:1.3:compile - omitted for conflict with 1.4)
";
        let t = tree::parse(text).unwrap().remove(0);
        let mut m = Module {
            coords: Gav {
                group: "com.example".into(),
                artifact: "app".into(),
                version: "1.0".into(),
                packaging: "jar".into(),
            },
            declared: vec![],
            managed: vec![],
            tree: t.children,
            error: None,
            overrides: vec![],
        };
        m.overrides = find(&m);
        assert_eq!(
            summary(&m),
            [
                row(
                    "corp:dep:1.4",
                    "sdk",
                    "2.4",
                    "3.0",
                    Change::MajorUpgrade,
                    true
                ),
                // c's own request lost Mediation; c is the Requester here, not corp:dep.
                row(
                    "com.example:c:1.0",
                    "dep",
                    "1.3",
                    "1.4",
                    Change::MinorUpgrade,
                    false
                ),
            ]
        );
        let sdk = &m.overrides[0];
        assert_eq!(sdk.paths, vec![vec![0, 0, 0], vec![1, 0, 0]]);
        assert_eq!(
            sdk.scope, "compile",
            "the compile path outweighs the test path"
        );
    }

    #[test]
    fn reduced_scope_is_ranked_lower() {
        let mut o = modules()[1].overrides[0].clone();
        assert!(o.is_warning());
        o.scope = "provided".into();
        assert!(
            !o.is_warning(),
            "a provided-scope major upgrade is informational"
        );
        assert_eq!(o.change, Change::MajorUpgrade, "the label stays honest");
    }
}
