//! Parser for `maven-help-plugin:effective-pom -Dverbose` output.
//!
//! In verbose mode Maven follows each value with a comment naming the POM and
//! line it came from: `<version>2.17.1</version>  <!-- g:a:v, line 96 -->`.
//! A multi-module build yields `<projects>` holding one `<project>` per module;
//! a single module yields a bare `<project>`.

use crate::model::{DeclaredDependency, Gav, Location};
use anyhow::{Context, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug)]
pub struct EffectiveProject {
    pub coords: Gav,
    pub declared: Vec<DeclaredDependency>,
    pub managed: Vec<DeclaredDependency>,
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Declared,
    Managed,
}

pub fn parse(xml: &str) -> Result<Vec<EffectiveProject>> {
    let mut reader = Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut projects = Vec::new();
    let mut project: Option<EffectiveProject> = None;
    let mut dep: Option<(Section, DeclaredDependency)> = None;
    let mut text = String::new();
    // The leaf element that just closed, awaiting its Origin comment.
    let mut last_leaf: Option<String> = None;

    loop {
        let event = reader.read_event().with_context(|| {
            format!(
                "effective POM is not valid XML near byte {}",
                reader.buffer_position()
            )
        })?;
        match event {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_string();
                last_leaf = None;
                text.clear();
                if name == "project" {
                    project = Some(EffectiveProject {
                        coords: Gav {
                            group: String::new(),
                            artifact: String::new(),
                            version: String::new(),
                            packaging: "jar".into(),
                        },
                        declared: Vec::new(),
                        managed: Vec::new(),
                    });
                } else if name == "dependency" {
                    dep = section_of(&path).map(|s| (s, DeclaredDependency::default()));
                }
                path.push(name);
            }
            Event::Text(e) => text.push_str(&e.into_inner()),
            Event::CData(e) => text.push_str(&e.into_inner()),
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let value = text.trim().to_string();
                text.clear();
                let parent = path.last().map(String::as_str);
                match (name.as_str(), parent) {
                    ("project", _) => projects.extend(project.take()),
                    ("dependency", _) => {
                        if let (Some((section, d)), Some(p)) = (dep.take(), project.as_mut()) {
                            match section {
                                Section::Declared => p.declared.push(d),
                                Section::Managed => p.managed.push(d),
                            }
                        }
                    }
                    (field, Some("project")) => {
                        if let Some(p) = project.as_mut() {
                            match field {
                                "groupId" => p.coords.group = value,
                                "artifactId" => p.coords.artifact = value,
                                "version" => p.coords.version = value,
                                "packaging" => p.coords.packaging = value,
                                _ => {}
                            }
                        }
                    }
                    (field, Some("dependency")) => {
                        if let Some((_, d)) = dep.as_mut() {
                            match field {
                                "groupId" => d.group = value,
                                "artifactId" => d.artifact = value,
                                "version" => d.version = Some(value),
                                "scope" => d.scope = Some(value),
                                _ => {}
                            }
                            last_leaf = Some(field.to_string());
                        }
                    }
                    _ => {}
                }
            }
            Event::Comment(e) => {
                if let (Some(field), Some((_, d))) = (last_leaf.take(), dep.as_mut())
                    && let Some(loc) = parse_location(&e.into_inner())
                {
                    match field.as_str() {
                        "artifactId" => d.declared_at = Some(loc),
                        "version" => d.version_at = Some(loc),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(projects)
}

/// Which list a `<dependency>` starting under `path` belongs to, if any.
/// Dependencies of plugins and profiles are ignored.
fn section_of(path: &[String]) -> Option<Section> {
    let tail: Vec<&str> = path.iter().rev().take(3).map(String::as_str).collect();
    match tail.as_slice() {
        ["dependencies", "project", ..] => Some(Section::Declared),
        ["dependencies", "dependencyManagement", "project"] => Some(Section::Managed),
        _ => None,
    }
}

/// Parses ` com.example:web:1.0.0, line 22 ` into a [`Location`].
fn parse_location(comment: &str) -> Option<Location> {
    let (pom, line) = comment.trim().rsplit_once(", line ")?;
    Some(Location {
        pom: pom.trim().to_string(),
        line: line.trim().parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/maven-output/effective-pom.xml");

    fn loc(pom: &str, line: u32) -> Option<Location> {
        Some(Location {
            pom: pom.into(),
            line,
        })
    }

    #[test]
    fn reads_every_project() {
        let projects = parse(FIXTURE).unwrap();
        let coords: Vec<_> = projects.iter().map(|p| p.coords.to_string()).collect();
        assert_eq!(
            coords,
            [
                "com.example:multi:1.0.0",
                "com.example:core:1.0.0",
                "com.example:web:1.0.0"
            ]
        );
        assert_eq!(projects[0].coords.packaging, "pom");
        assert_eq!(projects[1].coords.packaging, "jar");
    }

    #[test]
    fn version_origin_points_at_the_bom() {
        let core = &parse(FIXTURE).unwrap()[1];
        let databind = core
            .declared
            .iter()
            .find(|d| d.artifact == "jackson-databind")
            .unwrap();
        assert_eq!(databind.version.as_deref(), Some("2.17.1"));
        assert_eq!(databind.declared_at, loc("com.example:core:1.0.0", 13));
        assert_eq!(
            databind.version_at,
            loc("com.fasterxml.jackson:jackson-bom:2.17.1", 96)
        );
    }

    #[test]
    fn reads_dependency_management_but_not_profile_dependencies() {
        let projects = parse(FIXTURE).unwrap();
        let root = &projects[0];
        assert!(
            root.declared.is_empty(),
            "profile dependencies must not count as declared"
        );
        let slf4j = root
            .managed
            .iter()
            .find(|d| d.artifact == "slf4j-api")
            .unwrap();
        assert_eq!(slf4j.version_at, loc("com.example:multi:1.0.0", 27));
        assert_eq!(projects[2].declared.len(), 5);
    }
}
