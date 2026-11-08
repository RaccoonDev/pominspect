//! Runs Maven to inspect a build (see ADR-0001) and turns its output into [`Module`]s.
//!
//! One Maven invocation runs both goals, by pinned plugin coordinates, writing
//! their results to files in a private temp directory. Maven's console output
//! is streamed back line by line for the log pane.

use crate::model::Module;
use crate::parse::{effective, tree};
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

const HELP_PLUGIN: &str = "org.apache.maven.plugins:maven-help-plugin:3.5.1";
const DEPENDENCY_PLUGIN: &str = "org.apache.maven.plugins:maven-dependency-plugin:3.8.1";

/// What to inspect and how to run Maven.
#[derive(Debug, Clone)]
pub struct Request {
    /// The `pom.xml` at the root of the build.
    pub pom: PathBuf,
    /// Overrides the Maven executable.
    pub mvn: Option<PathBuf>,
    pub profiles: Vec<String>,
    pub defines: Vec<String>,
}

impl Request {
    /// Accepts a directory containing `pom.xml` or a path to a POM file.
    pub fn new(
        path: &Path,
        mvn: Option<PathBuf>,
        profiles: Vec<String>,
        defines: Vec<String>,
    ) -> Result<Self> {
        let pom = if path.is_dir() {
            path.join("pom.xml")
        } else {
            path.to_path_buf()
        };
        if !pom.is_file() {
            bail!("no pom.xml found at {}", pom.display());
        }
        let pom = pom
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", pom.display()))?;
        Ok(Self {
            pom,
            mvn,
            profiles,
            defines,
        })
    }

    pub fn dir(&self) -> &Path {
        self.pom.parent().unwrap_or(Path::new("."))
    }

    /// `--mvn` if given, else the nearest `mvnw` in this or a parent directory, else `mvn`.
    pub fn executable(&self) -> PathBuf {
        if let Some(m) = &self.mvn {
            return m.clone();
        }
        self.dir()
            .ancestors()
            .map(|d| d.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }))
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "mvn.cmd" } else { "mvn" }))
    }

    fn args(&self, out: &Path) -> Vec<String> {
        let mut args = vec![
            "-B".into(),
            "-fae".into(),
            "-f".into(),
            self.pom.display().to_string(),
            format!("{HELP_PLUGIN}:effective-pom"),
            format!("-Doutput={}", out.join("effective-pom.xml").display()),
            format!("{DEPENDENCY_PLUGIN}:tree"),
            format!("-DoutputFile={}", out.join("deps.txt").display()),
            "-DappendOutput=true".into(),
            "-Dverbose".into(),
        ];
        if !self.profiles.is_empty() {
            args.push("-P".into());
            args.push(self.profiles.join(","));
        }
        args.extend(self.defines.iter().map(|d| format!("-D{d}")));
        args
    }

    /// The command line, for display.
    pub fn command_line(&self) -> String {
        let mut s = self.executable().display().to_string();
        for a in self.args(Path::new("<tmp>")) {
            s.push(' ');
            s.push_str(&a);
        }
        s
    }
}

/// Messages from a running inspection. `run` identifies the inspection so
/// stale messages from an earlier run can be ignored.
#[derive(Debug)]
pub enum Event {
    Log {
        run: u64,
        line: String,
    },
    Done {
        run: u64,
        result: Result<Vec<Module>, String>,
    },
}

/// Starts an inspection in the background and returns its run id.
pub fn spawn(req: Request, tx: Sender<Event>) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let run = NEXT.fetch_add(1, Ordering::Relaxed);
    thread::spawn(move || {
        let result = inspect(&req, run, &tx).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Event::Done { run, result });
    });
    run
}

fn inspect(req: &Request, run: u64, tx: &Sender<Event>) -> Result<Vec<Module>> {
    let out = std::env::temp_dir().join(format!("pominspect-{}-{run}", std::process::id()));
    std::fs::create_dir_all(&out).with_context(|| format!("cannot create {}", out.display()))?;
    let result = run_and_parse(req, run, tx, &out);
    let _ = std::fs::remove_dir_all(&out);
    result
}

fn run_and_parse(req: &Request, run: u64, tx: &Sender<Event>, out: &Path) -> Result<Vec<Module>> {
    let exe = req.executable();
    let mut child = Command::new(&exe)
        .args(req.args(out))
        .current_dir(req.dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start Maven (`{}`)", exe.display()))?;

    let readers = [
        forward(child.stdout.take().unwrap(), run, tx.clone()),
        forward(child.stderr.take().unwrap(), run, tx.clone()),
    ];
    let status = child.wait().context("waiting for Maven")?;
    let mut log = Vec::new();
    for r in readers {
        log.extend(r.join().unwrap_or_default());
    }

    let eff_path = out.join("effective-pom.xml");
    if !eff_path.is_file() {
        bail!(
            "Maven exited with {status} without producing an effective POM.\n\n{}",
            error_lines(&log, None).join("\n")
        );
    }
    let eff = std::fs::read_to_string(&eff_path)?;
    let trees = std::fs::read_to_string(out.join("deps.txt")).unwrap_or_default();
    assemble(&eff, &trees, &log)
}

fn forward(
    stream: impl Read + Send + 'static,
    run: u64,
    tx: Sender<Event>,
) -> thread::JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = tx.send(Event::Log {
                run,
                line: line.clone(),
            });
            lines.push(line);
        }
        lines
    })
}

/// Combines the Effective POM and dependency trees into modules. A module
/// without a dependency tree gets an Inspection Error built from the log.
pub fn assemble(effective_xml: &str, tree_text: &str, log: &[String]) -> Result<Vec<Module>> {
    let projects = effective::parse(effective_xml)?;
    let mut trees = tree::parse(tree_text)?;
    Ok(projects
        .into_iter()
        .map(|p| {
            let found = trees.iter().position(|t| {
                t.coords.group == p.coords.group
                    && t.coords.artifact == p.coords.artifact
                    && t.coords.version == p.coords.version
            });
            let (tree, error) = match found {
                Some(i) => (trees.swap_remove(i).children, None),
                None => {
                    let lines = error_lines(log, Some(&p.coords.artifact));
                    let msg = if lines.is_empty() {
                        "Maven produced no dependency tree for this module. Open the Maven log (m) for details."
                            .to_string()
                    } else {
                        lines.join("\n")
                    };
                    (Vec::new(), Some(msg))
                }
            };
            Module { coords: p.coords, declared: p.declared, managed: p.managed, tree, error }
        })
        .collect())
}

/// `[ERROR]` lines from the log, optionally only those naming a project.
fn error_lines(log: &[String], artifact: Option<&str>) -> Vec<String> {
    let errors = log.iter().filter(|l| l.starts_with("[ERROR]"));
    match artifact {
        None => errors.cloned().collect(),
        Some(a) => {
            let needle = format!("on project {a}:");
            errors.filter(|l| l.contains(&needle)).cloned().collect()
        }
    }
}

/// Extracts `web 1.0.0 [3/3]` from `[INFO] Building web 1.0.0   [3/3]`.
pub fn progress(line: &str) -> Option<String> {
    let rest = line.strip_prefix("[INFO] Building ")?;
    if rest.starts_with("jar:") || rest.starts_with("war:") {
        return None;
    }
    let collapsed: Vec<&str> = rest.split_whitespace().collect();
    Some(collapsed.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EFF: &str = include_str!("../tests/fixtures/maven-output/effective-pom.xml");
    const DEPS: &str = include_str!("../tests/fixtures/maven-output/deps.txt");

    #[test]
    fn assembles_modules_with_trees() {
        let modules = assemble(EFF, DEPS, &[]).unwrap();
        assert_eq!(modules.len(), 3);
        assert!(modules.iter().all(|m| m.error.is_none()));
        assert_eq!(modules[2].tree.len(), 5);
    }

    #[test]
    fn module_without_tree_gets_inspection_error() {
        let without_web: String = DEPS
            .split("com.example:web:jar")
            .next()
            .unwrap()
            .to_string();
        let log = vec![
            "[ERROR] Failed to execute goal x on project web: Could not resolve dependencies"
                .to_string(),
            "[ERROR] Failed to execute goal x on project other: nope".to_string(),
        ];
        let modules = assemble(EFF, &without_web, &log).unwrap();
        let err = modules[2].error.as_deref().unwrap();
        assert!(err.contains("on project web:"));
        assert!(!err.contains("other"));
        assert!(modules[1].error.is_none());
    }

    #[test]
    fn reads_reactor_progress() {
        assert_eq!(
            progress("[INFO] Building web 1.0.0                                     [3/3]")
                .as_deref(),
            Some("web 1.0.0 [3/3]")
        );
        assert_eq!(progress("[INFO] Downloading from central"), None);
    }

    /// Runs real Maven against the fixture build: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs Maven on PATH"]
    fn inspects_fixture_with_real_maven() {
        let req = Request::new(Path::new("tests/fixtures/multi"), None, vec![], vec![]).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let run = spawn(req, tx);
        let mut saw_progress = false;
        let modules = loop {
            match rx.recv().unwrap() {
                Event::Log { line, .. } => saw_progress |= progress(&line).is_some(),
                Event::Done { run: r, result } => {
                    assert_eq!(r, run);
                    break result.unwrap();
                }
            }
        };
        assert!(saw_progress);
        let names: Vec<_> = modules.iter().map(|m| m.coords.artifact.as_str()).collect();
        assert_eq!(names, ["multi", "core", "web"]);
        assert!(modules.iter().all(|m| m.error.is_none()));
        assert_eq!(modules[2].conflicts().len(), 1);
    }
}
