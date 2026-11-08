//! Rendering of the TUI.

use crate::app::{App, Entry, Mode, Row, Status};
use crate::model::{DepNode, Key, Module, Omission, Presence, VersionOrigin};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph, Wrap};

const SPINNER: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}
fn heading(s: &str) -> Line<'static> {
    Line::from(s.to_string()).bold().fg(Color::Cyan)
}
fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());

    if app.modules.is_empty() {
        draw_log(frame, app, main);
    } else if app.show_log {
        let [top, log] =
            Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(main);
        draw_browser(frame, app, top);
        draw_log(frame, app, log);
    } else {
        draw_browser(frame, app, main);
    }
    draw_status(frame, app, status);

    match &app.mode {
        Mode::Why(key) => draw_why(frame, app, key),
        Mode::Help => draw_help(frame),
        _ => {}
    }
}

fn draw_browser(frame: &mut Frame, app: &mut App, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let rows = app.rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| ListItem::new(row_line(app, r)))
        .collect();
    let list = List::new(items)
        .block(Block::bordered().title(" Modules "))
        .highlight_style(
            Style::new()
                .bg(Color::Rgb(50, 60, 80))
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, left, &mut app.list_state);

    let detail = detail_text(app);
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Detail ")),
        right,
    );
}

fn row_line(app: &App, row: &Row) -> Line<'static> {
    let arrow = match (row.expandable, row.expanded) {
        (false, _) => "  ",
        (true, true) => "▾ ",
        (true, false) => "▸ ",
    };
    let mut spans = vec![Span::raw("  ".repeat(row.depth)), Span::raw(arrow)];
    match app.entry(&row.id) {
        Some(Entry::Module(m)) => {
            spans.push(Span::raw(m.coords.artifact.clone()).bold());
            spans.push(Span::styled(format!(" ({})", m.coords.packaging), dim()));
            if m.error.is_some() {
                spans.push(Span::styled(" ✗ error", Style::new().fg(Color::Red)));
            }
            let conflicts = m.conflicts().len();
            if conflicts > 0 {
                spans.push(Span::styled(format!(" ⚠ {conflicts}"), warn()));
            }
        }
        Some(Entry::Dependencies(m)) => {
            spans.push(Span::raw("Dependencies"));
            spans.push(Span::styled(format!(" ({} direct)", m.tree.len()), dim()));
        }
        Some(Entry::Dep { module, node, .. }) => {
            let omitted = node.omitted.is_some();
            let name = Span::raw(format!("{} {}", node.artifact, node.version));
            spans.push(if omitted {
                name.style(dim().add_modifier(Modifier::CROSSED_OUT))
            } else {
                name
            });
            if node.scope != "compile" {
                spans.push(Span::styled(
                    format!(" [{}]", node.scope),
                    Style::new().fg(Color::Magenta),
                ));
            }
            match &node.omitted {
                Some(Omission::Conflict { winner }) => {
                    spans.push(Span::styled(format!(" ✗ conflict, {winner} wins"), warn()))
                }
                Some(Omission::Duplicate) => spans.push(Span::styled(" (duplicate)", dim())),
                Some(o) => spans.push(Span::styled(format!(" ({o})"), dim())),
                None if module.has_conflict(&node.key()) => spans.push(Span::styled(" ⚠", warn())),
                None => {}
            }
            spans.push(Span::styled(format!("  {}", node.group), dim()));
        }
        None => {}
    }
    Line::from(spans)
}

fn detail_text(app: &App) -> Text<'static> {
    match app.entry(&app.selected) {
        Some(Entry::Module(m)) => module_detail(m),
        Some(Entry::Dependencies(m)) => dependencies_detail(m),
        Some(Entry::Dep { module, path, node }) => dep_detail(module, path, node),
        None => Text::raw(""),
    }
}

fn module_detail(m: &Module) -> Text<'static> {
    let mut lines = vec![
        Line::from(m.coords.to_string()).bold(),
        Line::from(format!("packaging: {}", m.coords.packaging)),
        Line::raw(""),
    ];
    if let Some(e) = &m.error {
        lines.push(heading("Inspection Error").fg(Color::Red));
        lines.extend(e.lines().map(|l| Line::raw(l.to_string())));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(format!(
        "direct dependencies:   {}",
        m.tree.len()
    )));
    lines.push(Line::from(format!(
        "managed dependencies:  {}",
        m.managed.len()
    )));
    conflicts_section(m, &mut lines);
    Text::from(lines)
}

fn dependencies_detail(m: &Module) -> Text<'static> {
    let mut resolved = std::collections::BTreeSet::new();
    fn walk(nodes: &[DepNode], out: &mut std::collections::BTreeSet<String>) {
        for n in nodes {
            if n.omitted.is_none() {
                out.insert(n.label());
            }
            walk(&n.children, out);
        }
    }
    walk(&m.tree, &mut resolved);
    let mut lines = vec![
        Line::from(format!("Dependencies of {}", m.coords.artifact)).bold(),
        Line::raw(""),
        Line::from(format!("direct:              {}", m.tree.len())),
        Line::from(format!("resolved artifacts:  {}", resolved.len())),
    ];
    conflicts_section(m, &mut lines);
    Text::from(lines)
}

fn conflicts_section(m: &Module, lines: &mut Vec<Line<'static>>) {
    let conflicts = m.conflicts();
    if conflicts.is_empty() {
        return;
    }
    lines.push(Line::raw(""));
    lines.push(heading(&format!("Version conflicts ({})", conflicts.len())));
    for key in conflicts {
        let occ = m.occurrences(&key);
        let winner = occ
            .iter()
            .find(|(_, n)| n.omitted.is_none())
            .map(|(_, n)| n.version.clone());
        let losers: Vec<String> = occ
            .iter()
            .filter(|(_, n)| n.is_conflict_loser())
            .map(|(_, n)| n.version.clone())
            .collect();
        lines.push(Line::from(vec![
            Span::styled("⚠ ", warn()),
            Span::raw(key.to_string()),
            Span::raw(format!(
                "  {} wins over {}",
                winner.unwrap_or("?".into()),
                losers.join(", ")
            )),
        ]));
    }
}

fn path_line(module: &Module, path: &[usize]) -> Line<'static> {
    let mut spans = vec![Span::styled(module.coords.artifact.clone(), dim())];
    for n in module.chain(path) {
        spans.push(Span::styled(" → ", dim()));
        spans.push(Span::raw(format!("{}:{}", n.artifact, n.version)));
    }
    Line::from(spans)
}

fn dep_detail(module: &Module, path: &[usize], node: &DepNode) -> Text<'static> {
    let mut lines = vec![Line::from(node.label()).bold()];
    let mut facts = vec![
        format!("type: {}", node.kind),
        format!("scope: {}", node.scope),
    ];
    facts.extend(node.notes.iter().cloned());
    lines.push(Line::from(facts.join("   ")));
    if let Some(o) = &node.omitted {
        lines.push(Line::styled(format!("not resolved here: {o}"), warn()));
    }

    lines.push(Line::raw(""));
    lines.push(heading("Origin"));
    if let Some(origin) = module.origin(path) {
        let present = match origin.presence {
            Presence::Direct { at: Some(at) } => format!("declared   ← {at}"),
            Presence::Direct { at: None } => "declared   ← (location not reported by Maven)".into(),
            Presence::Transitive { via } => format!("pulled in  ← by {via}"),
        };
        lines.push(Line::raw(format!("  {present}")));
        let version = match origin.version {
            VersionOrigin::Declared { at: Some(at) } => vec![format!("version    ← {at}")],
            VersionOrigin::Declared { at: None } => {
                vec!["version    ← (location not reported by Maven)".into()]
            }
            VersionOrigin::Managed { from, at } => vec![
                format!(
                    "version    ← dependencyManagement{}",
                    at.map(|a| format!(", {a}")).unwrap_or_default()
                ),
                format!(
                    "             (requested {from}, managed to {})",
                    node.version
                ),
            ],
            VersionOrigin::FromParent { via } => {
                vec![format!("version    ← as requested by {via}")]
            }
        };
        lines.extend(version.into_iter().map(|v| Line::raw(format!("  {v}"))));
    }

    lines.push(Line::raw(""));
    lines.push(heading("Path"));
    lines.push(path_line(module, path));

    let others: Vec<_> = module
        .occurrences(&node.key())
        .into_iter()
        .filter(|(p, _)| p != path)
        .collect();
    if !others.is_empty() {
        lines.push(Line::raw(""));
        lines.push(heading(&format!(
            "Other occurrences in {} ({})",
            module.coords.artifact,
            others.len()
        )));
        for (p, n) in others {
            lines.push(occurrence_line(n));
            lines.push(path_line(module, &p));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "w: why is this here, across all modules",
        dim(),
    ));
    Text::from(lines)
}

fn occurrence_line(n: &DepNode) -> Line<'static> {
    match &n.omitted {
        None => Line::from(vec![
            Span::styled("✓ ", Style::new().fg(Color::Green)),
            Span::raw(n.version.clone()),
        ]),
        Some(o) => Line::from(vec![
            Span::styled("✗ ", warn()),
            Span::raw(n.version.clone()),
            Span::styled(format!("  {o}"), dim()),
        ]),
    }
}

fn draw_why(frame: &mut Frame, app: &App, key: &Key) {
    let mut lines = Vec::new();
    for m in &app.modules {
        let occ = m.occurrences(key);
        if occ.is_empty() {
            continue;
        }
        lines.push(heading(&m.coords.artifact));
        for (path, n) in occ {
            lines.push(occurrence_line(n));
            lines.push(path_line(m, &path));
        }
        lines.push(Line::raw(""));
    }
    let area = popup(frame.area(), 80, 80);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(format!(" Why {key}? "))
                .title_bottom(" Esc close "),
        ),
        area,
    );
}

fn draw_help(frame: &mut Frame) {
    let keys = [
        ("↑↓ / j k", "move"),
        ("→ / l", "expand, or enter"),
        ("← / h", "collapse, or go to parent"),
        ("Enter / Space", "toggle"),
        ("E / C", "expand / collapse everything below"),
        ("PgUp PgDn g G", "page, top, bottom"),
        ("/", "search (group:artifact:version)"),
        ("n / N", "next / previous match"),
        (
            "w",
            "why: every path to the selected dependency, in all modules",
        ),
        ("m", "toggle the Maven log"),
        ("r", "re-run Maven"),
        ("q", "quit"),
    ];
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:>15}  "), warn()),
                Span::raw(*d),
            ])
        })
        .collect();
    let area = popup(frame.area(), 60, 60);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Keys ")),
        area,
    );
}

fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(height);
    let lines: Vec<Line> = app.log[start..]
        .iter()
        .map(|l| {
            let style = if l.starts_with("[ERROR]") {
                Style::new().fg(Color::Red)
            } else if l.starts_with("[WARNING]") {
                warn()
            } else {
                Style::new()
            };
            Line::styled(l.clone(), style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Maven log (m) ")),
        area,
    );
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let left = match (&app.mode, &app.message) {
        (Mode::Search(buf), _) => Line::from(vec![
            Span::styled("/", warn()),
            Span::raw(format!("{buf}▏")),
        ]),
        (_, Some(msg)) => Line::styled(msg.clone(), warn()),
        _ => Line::styled(
            " / search   w why   m maven log   r reload   ? help   q quit",
            dim(),
        ),
    };
    let right = match &app.status {
        Status::Running { progress } => Line::styled(
            format!(
                "{} running mvn…{} ",
                SPINNER[app.tick % SPINNER.len()],
                progress
                    .as_ref()
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default()
            ),
            Style::new().fg(Color::Cyan),
        ),
        Status::Ready => Line::styled(
            format!("✓ {} modules ", app.modules.len()),
            Style::new().fg(Color::Green),
        ),
        Status::Failed => Line::styled("✗ Maven failed, see log ", Style::new().fg(Color::Red)),
    };
    let width = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).areas(area);
    frame.render_widget(Paragraph::new(left), l);
    frame.render_widget(Paragraph::new(right), r);
}

fn popup(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let [_, v, _] = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .areas(area);
    let [_, h, _] = Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .areas(v);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::{self, Request};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }

    fn loaded_app() -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let req = Request::new(
            std::path::Path::new("tests/fixtures/multi"),
            None,
            vec![],
            vec![],
        )
        .unwrap();
        let mut app = App::new(req, tx);
        let modules = maven::assemble(
            include_str!("../tests/fixtures/maven-output/effective-pom.xml"),
            include_str!("../tests/fixtures/maven-output/deps.txt"),
            &[],
        )
        .unwrap();
        app.on_maven(maven::Event::Done {
            run: 0,
            result: Ok(modules),
        });
        app
    }

    #[test]
    fn search_reveals_conflicting_dependency_and_shows_origin() {
        let mut app = loaded_app();
        press(&mut app, "/commons-lang3:3.14\n");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("commons-lang3 3.14.0 ✗ conflict, 3.12.0 wins"));
        assert!(s.contains("pulled in  ← by org.apache.commons:commons-text:1.12.0"));
        assert!(s.contains("web → commons-text:1.12.0 → commons-lang3:3.14.0"));
    }

    #[test]
    fn why_lists_paths_across_modules() {
        let mut app = loaded_app();
        press(&mut app, "/slf4j-api\nw");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("Why org.slf4j:slf4j-api?"));
        assert!(s.contains("web → logback-classic:1.4.14 → slf4j-api:2.0.13"));
        assert!(s.contains("core → slf4j-api:2.0.13"));
    }
}
