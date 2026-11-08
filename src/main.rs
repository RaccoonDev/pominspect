mod app;
mod maven;
mod model;
mod parse;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Explore a Maven build's effective POM and dependency tree, and see the
/// Origin of every dependency: which POM declared it, which BOM or
/// dependencyManagement entry set its version, and which path pulled it in.
///
/// pominspect runs Maven itself (the project's ./mvnw if present, else `mvn`)
/// with pinned versions of maven-help-plugin and maven-dependency-plugin, so
/// what it shows always matches what Maven does. Maven may download artifacts
/// exactly as it would if you ran it yourself.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Maven project directory, or path to a pom.xml (default: current directory)
    path: Option<PathBuf>,

    /// Profiles to activate, as with `mvn -P` (comma-separated, repeatable)
    #[arg(short = 'P', long = "activate-profiles", value_delimiter = ',', value_name = "PROFILE")]
    profiles: Vec<String>,

    /// Define a property, as with `mvn -D` (repeatable)
    #[arg(short = 'D', value_name = "KEY=VALUE")]
    defines: Vec<String>,

    /// Maven executable to use instead of ./mvnw or `mvn` on PATH
    #[arg(long, value_name = "PATH")]
    mvn: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.path.unwrap_or_else(|| PathBuf::from("."));
    let request = maven::Request::new(&path, cli.mvn, cli.profiles, cli.defines)?;

    let (tx, rx) = mpsc::channel();
    let mut app = App::new(request, tx);
    app.reload();

    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            while let Ok(ev) = rx.try_recv() {
                app.on_maven(ev);
            }
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        app.on_key(key);
                    }
                }
            }
            app.tick = app.tick.wrapping_add(1);
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
