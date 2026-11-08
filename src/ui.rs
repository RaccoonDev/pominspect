//! Rendering of the TUI.

use crate::app::{App, Entry, Group, Mode, Row, Status};
use crate::model::{DepNode, Key, Module, Omission, Presence, VersionOrigin};
use crate::overrides::{Cause, Change, VersionOverride};
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

/// Colour for a Version Override: red for downgrades, yellow for other
/// warnings, dim for informational ones.
fn override_style(o: &VersionOverride) -> Style {
    match (o.is_warning(), o.change) {
        (true, Change::Downgrade) => Style::new().fg(Color::LightRed),
        (true, _) => warn(),
        (false, _) => dim(),
    }
}

/// `▲ major (asked 2.4)` next to a node showing the resolved version,
/// `▼ downgrade (gets 3.12.0)` next to one showing the requested version,
/// `▲ major: asked 2.4, gets 3.0` where no version is shown.
fn override_badge(o: &VersionOverride, shown: Option<&str>) -> Span<'static> {
    let versions = match shown {
        Some(v) if v == o.resolved => format!(" (asked {})", o.requested),
        Some(_) => format!(" (gets {})", o.resolved),
        None => format!(": asked {}, gets {}", o.requested, o.resolved),
    };
    Span::styled(
        format!(" {} {}{versions}", o.change.symbol(), o.change.short()),
        override_style(o),
    )
}

fn cause_text(module: &Module, o: &VersionOverride) -> String {
    match &o.cause {
        Cause::Mediation => {
            let winner = module
                .resolved_path(&o.library)
                .map(|p| {
                    let mut s = module.coords.artifact.clone();
                    for n in module.chain(&p) {
                        s.push_str(&format!(" → {}:{}", n.artifact, n.version));
                    }
                    s
                })
                .unwrap_or_else(|| "another path".into());
            format!("Mediation: a nearer path wins ({winner})")
        }
        Cause::Management { at: Some(at) } => format!("Management: dependencyManagement, {at}"),
        Cause::Management { at: None } => "Management: dependencyManagement".into(),
    }
}

fn warning_count(m: &Module) -> usize {
    m.overrides.iter().filter(|o| o.is_warning()).count()
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
            let warnings = warning_count(m);
            if warnings > 0 {
                spans.push(Span::styled(format!(" ⚠ {warnings}"), warn()));
            }
        }
        Some(Entry::Dependencies(m)) => {
            spans.push(Span::raw("Dependencies"));
            spans.push(Span::styled(format!(" ({} direct)", m.tree.len()), dim()));
        }
        Some(Entry::Dep { module, node, path }) => {
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
            match (&node.omitted, module.override_at(path)) {
                (_, Some(o)) => spans.push(override_badge(o, Some(&node.version))),
                (Some(Omission::Duplicate), None) => {
                    spans.push(Span::styled(" (duplicate)", dim()))
                }
                (Some(o), None) => spans.push(Span::styled(format!(" ({o})"), dim())),
                (None, None) => {}
            }
            spans.push(Span::styled(format!("  {}", node.group), dim()));
        }
        Some(Entry::Overrides(m)) => {
            spans.push(Span::raw("Version Overrides"));
            spans.push(Span::styled(format!(" ({})", m.overrides.len()), dim()));
            let warnings = warning_count(m);
            if warnings > 0 {
                spans.push(Span::styled(format!(" ⚠ {warnings}"), warn()));
            }
        }
        Some(Entry::OverrideGroup { group, items, .. }) => {
            let style = match group {
                Group::Downgrade => Style::new().fg(Color::LightRed),
                Group::MajorUpgrade => warn(),
                Group::Other => dim(),
            };
            spans.push(Span::styled(group.label(), style));
            spans.push(Span::styled(format!(" ({})", items.len()), dim()));
        }
        Some(Entry::Override { item, .. }) => {
            spans.push(Span::raw(item.library.artifact.clone()));
            spans.push(override_badge(item, None));
            if item.is_reduced_scope() {
                spans.push(Span::styled(
                    format!(" [{}]", item.scope),
                    Style::new().fg(Color::Magenta),
                ));
            }
            spans.push(Span::styled(
                format!("  ← {}", short_gav(&item.requester)),
                dim(),
            ));
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
        Some(Entry::Overrides(m)) => overrides_detail(m),
        Some(Entry::OverrideGroup { group, items, .. }) => group_detail(group, &items),
        Some(Entry::Override { module, item }) => override_detail(module, item),
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
    overrides_summary(m, &mut lines);
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
    overrides_summary(m, &mut lines);
    Text::from(lines)
}

/// `group:artifact:version` → `artifact:version`
fn short_gav(gav: &str) -> String {
    let parts: Vec<&str> = gav.split(':').collect();
    match parts.as_slice() {
        [_, a, v] => format!("{a}:{v}"),
        _ => gav.to_string(),
    }
}

fn overrides_summary(m: &Module, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::raw(""));
    if m.overrides.is_empty() {
        lines.push(Line::styled("no Version Overrides", dim()));
        return;
    }
    lines.push(heading(&format!(
        "Version Overrides ({}, {} warnings)",
        m.overrides.len(),
        warning_count(m)
    )));
    for change in [
        Change::Downgrade,
        Change::MajorUpgrade,
        Change::MinorUpgrade,
        Change::PatchUpgrade,
        Change::Unclassified,
    ] {
        let n = m.overrides.iter().filter(|o| o.change == change).count();
        if n > 0 {
            lines.push(Line::raw(format!(
                "  {} {:<14} {n}",
                change.symbol(),
                change.label()
            )));
        }
    }
}

fn override_row(o: &VersionOverride) -> Line<'static> {
    let mut spans = vec![
        Span::raw(format!("{} ", short_gav(&o.requester))),
        Span::styled("→ ", dim()),
        Span::raw(o.library.artifact.clone()),
        override_badge(o, None),
    ];
    if o.is_reduced_scope() {
        spans.push(Span::styled(
            format!(" [{}]", o.scope),
            Style::new().fg(Color::Magenta),
        ));
    }
    Line::from(spans)
}

/// The "Version Override" section shown for an override and for its tree node.
fn override_section(module: &Module, o: &VersionOverride, lines: &mut Vec<Line<'static>>) {
    let verdict = if o.is_warning() {
        "warning"
    } else {
        "informational"
    };
    lines.push(Line::from(vec![
        Span::styled("Version Override  ", Style::new().fg(Color::Cyan).bold()),
        Span::styled(
            format!("{} {} ({verdict})", o.change.symbol(), o.change.label()),
            override_style(o),
        ),
    ]));
    lines.push(Line::raw(format!("  requester  {}", o.requester)));
    lines.push(Line::raw(format!("  asked      {}", o.requested)));
    lines.push(Line::raw(format!("  gets       {}", o.resolved)));
    lines.push(Line::raw(format!("  because    {}", cause_text(module, o))));
    match o.scope.as_str() {
        "test" => lines.push(Line::styled(
            "  scope      test: only the test classpath is affected, so it ranks one level lower",
            dim(),
        )),
        "provided" => lines.push(Line::styled(
            "  scope      provided: the runtime container decides which version is present, so it ranks one level lower",
            dim(),
        )),
        _ => {}
    }
}

fn overrides_detail(m: &Module) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("Version Overrides in {}", m.coords.artifact)).bold(),
        Line::raw(""),
        Line::styled(
            "Dependencies that get a different version than the artifact requiring them asked for.",
            dim(),
        ),
        Line::styled(
            "Downgrades and major upgrades are warnings; test and provided scope rank one level lower.",
            dim(),
        ),
    ];
    overrides_summary(m, &mut lines);
    Text::from(lines)
}

fn group_detail(group: Group, items: &[&VersionOverride]) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("{} ({})", group.label(), items.len())).bold(),
        Line::raw(""),
    ];
    lines.extend(items.iter().map(|o| override_row(o)));
    Text::from(lines)
}

fn override_detail(module: &Module, o: &VersionOverride) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("{}:{}", o.library, o.resolved)).bold(),
        Line::raw(""),
    ];
    override_section(module, o, &mut lines);
    lines.push(Line::raw(""));
    lines.push(heading(&format!("Paths ({})", o.paths.len())));
    for p in &o.paths {
        lines.push(path_line(module, p));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Enter: show in the dependency tree   w: why, across all modules",
        dim(),
    ));
    Text::from(lines)
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
        lines.push(Line::styled(format!("not resolved here: {o}"), dim()));
    }
    if let Some(o) = module.override_at(path) {
        lines.push(Line::raw(""));
        override_section(module, o, &mut lines);
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
            lines.push(occurrence_line(module, &p, n));
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

/// One occurrence of a library: what its Requester asked for and got if it is
/// a Version Override, otherwise whether Maven resolved it.
fn occurrence_line(module: &Module, path: &[usize], n: &DepNode) -> Line<'static> {
    if let Some(o) = module.override_at(path) {
        return Line::from(vec![
            Span::styled(format!("{} ", o.change.symbol()), override_style(o)),
            Span::raw(format!(
                "{} asked {} → gets {}",
                short_gav(&o.requester),
                o.requested,
                o.resolved
            )),
            Span::styled(format!("  {}", o.change.short()), override_style(o)),
            Span::styled(format!("  ({})", cause_text(module, o)), dim()),
        ]);
    }
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
            lines.push(occurrence_line(m, &path, n));
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
        (
            "Enter / Space",
            "toggle; on a Version Override, show it in the tree",
        ),
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
    fn tree_badges_the_overridden_edge_and_shows_why() {
        let mut app = loaded_app();
        press(&mut app, "/commons-lang3:3.14\n");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("web (jar) ⚠ 3"));
        assert!(s.contains("commons-lang3 3.14.0 ▼ downgrade (gets 3.12.0)"));
        assert!(s.contains("Version Override  ▼ Downgrade (warning)"));
        assert!(s.contains("requester  org.apache.commons:commons-text:1.12.0"));
        assert!(s.contains("Mediation: a nearer path wins (web → commons-lang3:3.12.0)"));
        assert!(!s.contains("conflict,"), "old conflict markers are gone");
    }

    #[test]
    fn overrides_facet_groups_by_severity_and_jumps_to_tree() {
        let mut app = loaded_app();
        // web is the third module; open its Version Overrides facet and the Major group.
        press(&mut app, "/version overrides\nnn");
        press(&mut app, "l");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("Version Overrides (4) ⚠ 3"));
        assert!(s.contains("▼ Downgrade (2)"));
        assert!(s.contains("▲ Major Upgrade (1)"));
        assert!(s.contains("· Minor / Patch / Unclassified (1)"));

        press(&mut app, "/guava\n");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("guava ▲ major: asked 20.0, gets 33.2.1-jre  ← reflect"));
        assert!(s.contains("requester  org.reflections:reflections:0.9.11"));
        assert!(s.contains("Management: dependencyManagement, com.example:multi:1.0.0,"));

        press(&mut app, "\n");
        let s = screen(&mut app);
        println!("{s}");
        assert!(matches!(app.entry(&app.selected), Some(Entry::Dep { .. })));
        assert!(s.contains("pulled in  ← by org.reflections:reflections:0.9.11"));
    }

    #[test]
    fn test_scope_downgrade_explains_its_ranking() {
        let mut app = loaded_app();
        press(&mut app, "/hamcrest-core\n");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("hamcrest-core 1.1 [test] ▼ downgrade (asked 1.3)"));
        assert!(s.contains("test: only the test classpath is affected"));
    }

    #[test]
    fn why_lists_paths_across_modules() {
        let mut app = loaded_app();
        press(&mut app, "/slf4j-api\nw");
        let s = screen(&mut app);
        println!("{s}");
        assert!(s.contains("Why org.slf4j:slf4j-api?"));
        assert!(s.contains("web → logback-classic:1.4.14 → slf4j-api:2.0.13"));
        assert!(s.contains("▲ logback-classic:1.4.14 asked 2.0.7 → gets 2.0.13  patch"));
        assert!(s.contains("core → slf4j-api:2.0.13"));
    }
}
