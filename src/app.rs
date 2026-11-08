//! TUI state and input handling (rendering lives in `ui`).

use crate::maven::{self, Request};
use crate::model::{DepNode, Key, Module};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::sync::mpsc::Sender;

/// Identifies an entry of the navigation tree:
/// `[m]` a module, `[m, 0]` its Dependencies facet, `[m, 0, i, j, …]` a dependency node.
pub type NodeId = Vec<usize>;

pub const FACET_DEPENDENCIES: usize = 0;

pub enum Entry<'a> {
    Module(&'a Module),
    Dependencies(&'a Module),
    Dep {
        module: &'a Module,
        path: &'a [usize],
        node: &'a DepNode,
    },
}

pub struct Row {
    pub id: NodeId,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
}

pub enum Status {
    Running { progress: Option<String> },
    Ready,
    Failed,
}

pub enum Mode {
    Normal,
    Search(String),
    Why(Key),
    Help,
}

pub struct App {
    pub request: Request,
    tx: Sender<maven::Event>,
    run: u64,
    pub modules: Vec<Module>,
    pub status: Status,
    pub log: Vec<String>,
    pub show_log: bool,
    pub expanded: HashSet<NodeId>,
    pub selected: NodeId,
    pub list_state: ListState,
    pub mode: Mode,
    pub query: String,
    pub message: Option<String>,
    pub tick: usize,
    pub quit: bool,
}

impl App {
    pub fn new(request: Request, tx: Sender<maven::Event>) -> Self {
        Self {
            request,
            tx,
            run: 0,
            modules: Vec::new(),
            status: Status::Ready,
            log: Vec::new(),
            show_log: false,
            expanded: HashSet::new(),
            selected: vec![0],
            list_state: ListState::default(),
            mode: Mode::Normal,
            query: String::new(),
            message: None,
            tick: 0,
            quit: false,
        }
    }

    /// Starts a fresh Maven inspection; current results stay visible until it finishes.
    pub fn reload(&mut self) {
        self.log.clear();
        self.log.push(format!("$ {}", self.request.command_line()));
        self.status = Status::Running { progress: None };
        self.run = maven::spawn(self.request.clone(), self.tx.clone());
    }

    pub fn on_maven(&mut self, event: maven::Event) {
        match event {
            maven::Event::Log { run, line } if run == self.run => {
                if let Some(p) = maven::progress(&line) {
                    self.status = Status::Running { progress: Some(p) };
                }
                self.log.push(line);
            }
            maven::Event::Done { run, result } if run == self.run => match result {
                Ok(modules) => {
                    let first_load = self.modules.is_empty();
                    self.modules = modules;
                    self.status = Status::Ready;
                    if first_load {
                        self.expanded = (0..self.modules.len()).map(|m| vec![m]).collect();
                        if self.modules.len() == 1 {
                            self.expanded.insert(vec![0, FACET_DEPENDENCIES]);
                        }
                        self.selected = vec![0];
                    }
                    self.fix_selection();
                }
                Err(e) => {
                    self.log.push(String::new());
                    self.log
                        .extend(e.lines().map(|l| format!("[pominspect] {l}")));
                    self.status = Status::Failed;
                    self.show_log = true;
                }
            },
            _ => {}
        }
    }

    pub fn entry<'a>(&'a self, id: &'a [usize]) -> Option<Entry<'a>> {
        let module = self.modules.get(*id.first()?)?;
        match id.len() {
            1 => Some(Entry::Module(module)),
            2 => Some(Entry::Dependencies(module)),
            _ => {
                let path = &id[2..];
                Some(Entry::Dep {
                    module,
                    path,
                    node: module.node(path)?,
                })
            }
        }
    }

    fn children_count(&self, id: &[usize]) -> usize {
        match self.entry(id) {
            Some(Entry::Module(_)) => 1,
            Some(Entry::Dependencies(m)) => m.tree.len(),
            Some(Entry::Dep { node, .. }) => node.children.len(),
            None => 0,
        }
    }

    /// The visible rows of the navigation tree.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for m in 0..self.modules.len() {
            self.push_rows(vec![m], 0, &mut rows);
        }
        rows
    }

    fn push_rows(&self, id: NodeId, depth: usize, rows: &mut Vec<Row>) {
        let n = self.children_count(&id);
        let expanded = n > 0 && self.expanded.contains(&id);
        rows.push(Row {
            id: id.clone(),
            depth,
            expandable: n > 0,
            expanded,
        });
        if expanded {
            for i in 0..n {
                let mut child = id.clone();
                child.push(i);
                self.push_rows(child, depth + 1, rows);
            }
        }
    }

    /// Every entry, expanded or not, in display order.
    fn all_ids(&self) -> Vec<NodeId> {
        fn walk(app: &App, id: NodeId, out: &mut Vec<NodeId>) {
            let n = app.children_count(&id);
            out.push(id.clone());
            for i in 0..n {
                let mut child = id.clone();
                child.push(i);
                walk(app, child, out);
            }
        }
        let mut out = Vec::new();
        for m in 0..self.modules.len() {
            walk(self, vec![m], &mut out);
        }
        out
    }

    /// Text that search matches against.
    fn search_text(&self, id: &[usize]) -> String {
        match self.entry(id) {
            Some(Entry::Module(m)) => m.coords.to_string(),
            Some(Entry::Dependencies(_)) => "dependencies".into(),
            Some(Entry::Dep { node, .. }) => node.label(),
            None => String::new(),
        }
    }

    fn parent(id: &[usize]) -> Option<NodeId> {
        (id.len() > 1).then(|| id[..id.len() - 1].to_vec())
    }

    /// Ensures the selection is a visible row, falling back to its nearest visible ancestor.
    fn fix_selection(&mut self) {
        let rows = self.rows();
        if rows.is_empty() {
            self.list_state.select(None);
            return;
        }
        let mut id = self.selected.clone();
        loop {
            if let Some(i) = rows.iter().position(|r| r.id == id) {
                self.selected = id;
                self.list_state.select(Some(i));
                return;
            }
            match Self::parent(&id) {
                Some(p) => id = p,
                None => break,
            }
        }
        self.selected = rows[0].id.clone();
        self.list_state.select(Some(0));
    }

    fn reveal(&mut self, id: &[usize]) {
        let mut p = Self::parent(id);
        while let Some(a) = p {
            p = Self::parent(&a);
            self.expanded.insert(a);
        }
        self.selected = id.to_vec();
        self.fix_selection();
    }

    fn move_by(&mut self, delta: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let cur = rows.iter().position(|r| r.id == self.selected).unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.selected = rows[next].id.clone();
        self.list_state.select(Some(next));
    }

    fn set_expanded_recursive(&mut self, id: &[usize], expand: bool) {
        let n = self.children_count(id);
        if n == 0 {
            return;
        }
        if expand {
            self.expanded.insert(id.to_vec());
        } else {
            self.expanded.remove(id);
        }
        for i in 0..n {
            let mut child = id.to_vec();
            child.push(i);
            self.set_expanded_recursive(&child, expand);
        }
    }

    fn search(&mut self, forward: bool) {
        if self.query.is_empty() {
            return;
        }
        let needle = self.query.to_lowercase();
        let ids = self.all_ids();
        let start = ids.iter().position(|i| *i == self.selected).unwrap_or(0);
        let n = ids.len();
        let found = (1..=n)
            .map(|k| {
                if forward {
                    (start + k) % n
                } else {
                    (start + n - k) % n
                }
            })
            .find(|&i| self.search_text(&ids[i]).to_lowercase().contains(&needle));
        match found {
            Some(i) => {
                let id = ids[i].clone();
                self.reveal(&id);
                self.message = None;
            }
            None => self.message = Some(format!("no match for “{}”", self.query)),
        }
    }

    /// The dependency key of the selected entry, if it is a dependency.
    pub fn selected_key(&self) -> Option<Key> {
        match self.entry(&self.selected)? {
            Entry::Dep { node, .. } => Some(node.key()),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        match &mut self.mode {
            Mode::Search(buf) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    self.query = std::mem::take(buf);
                    self.mode = Mode::Normal;
                    self.search(true);
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            },
            Mode::Why(_) | Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('q' | 'w' | '?') | KeyCode::Enter
                ) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Normal => self.on_normal_key(key.code),
        }
    }

    fn on_normal_key(&mut self, code: KeyCode) {
        self.message = None;
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => self.show_log = false,
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => self.move_by(15),
            KeyCode::PageUp => self.move_by(-15),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX / 2),
            KeyCode::Right | KeyCode::Char('l') => {
                let id = self.selected.clone();
                if self.children_count(&id) > 0 {
                    if self.expanded.insert(id.clone()) {
                        self.fix_selection();
                    } else {
                        self.move_by(1);
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let id = self.selected.clone();
                if !self.expanded.remove(&id)
                    && let Some(p) = Self::parent(&id)
                {
                    self.selected = p;
                }
                self.fix_selection();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let id = self.selected.clone();
                if !self.expanded.remove(&id) && self.children_count(&id) > 0 {
                    self.expanded.insert(id);
                }
                self.fix_selection();
            }
            KeyCode::Char('E') => {
                let id = self.selected.clone();
                self.set_expanded_recursive(&id, true);
                self.fix_selection();
            }
            KeyCode::Char('C') => {
                let id = self.selected.clone();
                self.set_expanded_recursive(&id, false);
                self.fix_selection();
            }
            KeyCode::Char('/') => self.mode = Mode::Search(String::new()),
            KeyCode::Char('n') => self.search(true),
            KeyCode::Char('N') => self.search(false),
            KeyCode::Char('w') => match self.selected_key() {
                Some(k) => self.mode = Mode::Why(k),
                None => self.message = Some("select a dependency to ask why".into()),
            },
            KeyCode::Char('m') => self.show_log = !self.show_log,
            KeyCode::Char('r') => {
                if !matches!(self.status, Status::Running { .. }) {
                    self.reload();
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }
}
