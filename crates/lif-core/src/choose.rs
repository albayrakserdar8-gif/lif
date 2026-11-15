//! The chooser modes: `choose-tree`, `choose-buffer`, `choose-client` and
//! `find-window` (tmux's tree, buffer and client modes).
//!
//! The model builds the list ([`Model::choose_items`]): one [`ChooseItem`]
//! per session, window and pane (or buffer, or client), in display order,
//! each with the target a command gets for it. [`ChooseState`] is the mode
//! itself, pure: which item is selected, what is expanded and tagged, the
//! sort order and filter, and what each key does. The server shows it in
//! the pane (it draws [`ChooseState::screen`]), runs the commands it asks
//! for, and gives it a fresh list whenever the model has changed.
//!
//! Layout, keys, prompts and default formats are tmux 3.8's, read off its
//! screen. Prompts are on the status line, as all of lif's are.

use crate::cmd::{Args, CmdError};
use crate::exec::expand_prompt_template;
use crate::format::{self, Vars};
use crate::host::Host;
use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use crate::keys::KeyCode;
use crate::model::{CmdContext, CmdOutput, Model};
use crate::target::{Flags, Kind};
use crate::time::{Tm, WallClock};
use lif_types::{Key, KeyEvent, Mods};
use std::collections::BTreeSet;

/// Most items a chooser lists; the rest are left out. A server has far
/// fewer sessions, windows and panes than this.
pub const MAX_CHOOSE_ITEMS: usize = 10_000;

/// Longest line of an item, in bytes of its expanded format.
const MAX_ITEM_TEXT: usize = 1024;

/// Most of a buffer shown in its preview, in bytes.
const MAX_PREVIEW_TEXT: usize = 16 * 1024;

/// What a chooser lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChooseKind {
    /// Sessions, their windows and their panes.
    Tree,
    /// Paste buffers.
    Buffers,
    /// Attached clients.
    Clients,
}

impl ChooseKind {
    /// The sort orders, in the order the `O` key steps through them; the
    /// first is the default.
    pub fn sort_orders(self) -> &'static [&'static str] {
        match self {
            ChooseKind::Tree => &["index", "name", "activity"],
            ChooseKind::Buffers => &["creation", "name", "size"],
            ChooseKind::Clients => &["name", "size", "creation", "activity"],
        }
    }

    fn default_template(self) -> &'static str {
        match self {
            ChooseKind::Tree => "switch-client -t '%%'",
            ChooseKind::Buffers => "paste-buffer -p -b '%%'",
            ChooseKind::Clients => "detach-client -t '%%'",
        }
    }
}

/// How a chooser was asked for, and how its list is built. The mode
/// changes `sort`, `reverse` and `filter` as keys ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooseSpec {
    /// What is listed.
    pub kind: ChooseKind,
    /// `-F`: the text of each item (tmux's default for the kind if none).
    pub format: Option<String>,
    /// `-f`: items whose filter expands to nothing or 0 are left out,
    /// unless that leaves none.
    pub filter: Option<String>,
    /// A format for the shortcut key of each line (tmux's `-K`). Always
    /// `None` for now: lines get tmux's default shortcuts.
    pub key_format: Option<String>,
    /// Index into [`ChooseKind::sort_orders`].
    pub sort: usize,
    /// `-r`.
    pub reverse: bool,
    /// The command run for the chosen item, `%%` being its target.
    pub template: String,
    /// `-s`: sessions start collapsed.
    pub sessions_collapsed: bool,
    /// `-w`: windows start collapsed.
    pub windows_collapsed: bool,
    /// The preview is shown (`-N` starts without it).
    pub preview: bool,
    /// `find-window`: only these windows (and their sessions and panes).
    pub only_windows: Option<Vec<WindowId>>,
    /// `-Z` zoomed the pane: unzoom it when the mode ends.
    pub zoomed: bool,
}

/// What the preview of an item shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// Nothing.
    None,
    /// These panes side by side, each with a label in a box (one pane:
    /// only the pane).
    Panes(Vec<(PaneId, String)>),
    /// Text (a buffer's contents).
    Text(String),
}

/// One line of a chooser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooseItem {
    /// What it stands for, unique in the list and stable across rebuilds
    /// (`$1`, `@3`, `%5`, a buffer's or a client's name).
    pub id: String,
    /// Its target, what `%%` becomes.
    pub target: String,
    /// 0 for sessions, buffers and clients; 1 for windows; 2 for panes.
    pub depth: u8,
    /// Shown before the text (`name: text`): a session's name, a window's
    /// or pane's index.
    pub name: String,
    /// The expanded format.
    pub text: String,
    /// What the preview shows for it.
    pub preview: Preview,
    /// The question `x` asks and the command a yes runs.
    pub kill: Option<(String, String)>,
}

/// What the server should do after the mode handled something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChooseOutcome {
    /// Nothing changed.
    None,
    /// Draw the mode again.
    Redraw,
    /// Leave the mode, then run this command for the client.
    Choose(String),
    /// Run these commands for the client and stay in the mode.
    Run(Vec<String>),
    /// Ask on the status line; the answer comes back through
    /// [`ChooseState::prompt_accept`].
    Prompt {
        /// The prompt text.
        label: String,
    },
    /// The sort order or filter changed: build the list again.
    Rebuild,
    /// Leave the mode.
    Exit,
}

/// What an answered prompt is for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Asking {
    Search,
    Filter,
    /// A command for the tagged items (or the selected one).
    Command,
    /// These commands if the answer is yes.
    Confirm(Vec<String>),
}

/// One row of the list as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooseLine {
    /// The whole line: shortcut, tree drawing, name and text.
    pub text: String,
    /// It is the selected item (drawn in `mode-style`).
    pub selected: bool,
}

/// What the mode shows in a pane of some size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooseScreen {
    /// The visible lines of the list, from the top of the pane.
    pub lines: Vec<ChooseLine>,
    /// Rows the list takes. The preview box takes the rest, if there is
    /// one.
    pub list_rows: u16,
    /// The title on the preview box; `None`: no preview, the list has the
    /// whole pane.
    pub title: Option<String>,
    /// What is inside the box.
    pub preview: Preview,
}

/// A chooser mode in a pane.
#[derive(Clone, Debug)]
pub struct ChooseState {
    spec: ChooseSpec,
    items: Vec<ChooseItem>,
    /// Ids of expanded sessions and windows.
    expanded: BTreeSet<String>,
    tagged: BTreeSet<String>,
    /// Id of the selected item.
    current: String,
    /// Where the mode started (`H`).
    home: String,
    /// First visible line.
    offset: usize,
    search: String,
    asking: Option<Asking>,
    /// Rows of the list when it was last drawn, for paging.
    page: usize,
}

fn key_is(key: &KeyEvent, name: &str) -> bool {
    KeyCode::from_event(key).is_some_and(|k| KeyCode::parse(name) == Some(k))
}

/// tmux's shortcut for line `n`: 0-9, then M-a to M-z.
fn shortcut(n: usize) -> Option<String> {
    match n {
        0..=9 => Some(n.to_string()),
        10..=35 => Some(format!("M-{}", (b'a' + (n - 10) as u8) as char)),
        _ => None,
    }
}

impl ChooseState {
    /// A mode for `spec` showing `items`, with `start` (an item id)
    /// selected if it is there.
    pub fn new(spec: ChooseSpec, items: Vec<ChooseItem>, start: Option<&str>) -> ChooseState {
        let mut expanded = BTreeSet::new();
        for it in &items {
            let open = match it.depth {
                0 => !spec.sessions_collapsed,
                1 => !spec.sessions_collapsed && !spec.windows_collapsed,
                _ => false,
            };
            if open {
                expanded.insert(it.id.clone());
            }
        }
        let mut s = ChooseState {
            spec,
            items,
            expanded,
            tagged: BTreeSet::new(),
            current: String::new(),
            home: String::new(),
            offset: 0,
            search: String::new(),
            asking: None,
            page: 10,
        };
        // The start item, or the nearest line above it that is shown.
        let vis = s.visible();
        let want = start.and_then(|id| s.items.iter().position(|i| i.id == id));
        let pick = match want {
            Some(w) => vis.iter().copied().rfind(|v| *v <= w).or(vis.first().copied()),
            None => vis.first().copied(),
        };
        if let Some(i) = pick {
            s.current = s.items[i].id.clone();
        }
        s.home = s.current.clone();
        s
    }

    /// How the list is built now (the mode changes sort and filter).
    pub fn spec(&self) -> &ChooseSpec {
        &self.spec
    }

    /// A fresh list (the model changed, or the mode asked for a rebuild).
    /// The selection stays on its item, or near where it was.
    pub fn set_items(&mut self, items: Vec<ChooseItem>) {
        let old = self.visible().iter().position(|i| self.items[*i].id == self.current).unwrap_or(0);
        // New sessions and windows appear as the mode was started.
        for it in &items {
            if !self.items.iter().any(|o| o.id == it.id) {
                let open = match it.depth {
                    0 => !self.spec.sessions_collapsed,
                    1 => !self.spec.sessions_collapsed && !self.spec.windows_collapsed,
                    _ => false,
                };
                if open {
                    self.expanded.insert(it.id.clone());
                }
            }
        }
        self.items = items;
        self.tagged.retain(|t| self.items.iter().any(|i| i.id == *t));
        self.expanded.retain(|t| self.items.iter().any(|i| i.id == *t));
        let vis = self.visible();
        if !vis.iter().any(|i| self.items[*i].id == self.current) {
            self.current = vis.get(old.min(vis.len().saturating_sub(1))).map(|i| self.items[*i].id.clone()).unwrap_or_default();
        }
    }

    /// Nothing is listed (every session, buffer or client is gone).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Indexes of the items shown: those whose ancestors are all expanded.
    fn visible(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.items.len());
        // The depth below which lines are hidden by a collapsed ancestor.
        let mut hidden_below: Option<u8> = None;
        for (i, it) in self.items.iter().enumerate() {
            if let Some(d) = hidden_below {
                if it.depth > d {
                    continue;
                }
                hidden_below = None;
            }
            out.push(i);
            if self.has_children(i) && !self.expanded.contains(&it.id) {
                hidden_below = Some(it.depth);
            }
        }
        out
    }

    fn has_children(&self, i: usize) -> bool {
        self.items.get(i + 1).is_some_and(|n| n.depth > self.items[i].depth)
    }

    fn current_index(&self) -> Option<usize> {
        self.items.iter().position(|i| i.id == self.current)
    }

    fn select(&mut self, vis: &[usize], pos: usize) {
        if let Some(i) = vis.get(pos) {
            self.current = self.items[*i].id.clone();
        }
    }

    fn step(&mut self, by: isize) -> ChooseOutcome {
        let vis = self.visible();
        if vis.is_empty() {
            return ChooseOutcome::None;
        }
        let pos = vis.iter().position(|i| self.items[*i].id == self.current).unwrap_or(0) as isize;
        let n = vis.len() as isize;
        // One step wraps around, as in tmux; a page stops at the ends.
        let next = if by.abs() == 1 { (pos + by).rem_euclid(n) } else { (pos + by).clamp(0, n - 1) };
        self.select(&vis, next as usize);
        ChooseOutcome::Redraw
    }

    /// The command for item `i`: the template with its target.
    fn command_for(&self, template: &str, i: usize) -> String {
        expand_prompt_template(template, &[self.items[i].target.as_str()])
    }

    /// The tagged items in list order, or the selected one if none is.
    fn tagged_or_current(&self) -> Vec<usize> {
        let t: Vec<usize> = (0..self.items.len()).filter(|i| self.tagged.contains(&self.items[*i].id)).collect();
        if t.is_empty() { self.current_index().into_iter().collect() } else { t }
    }

    fn ask(&mut self, what: Asking, label: impl Into<String>) -> ChooseOutcome {
        self.asking = Some(what);
        ChooseOutcome::Prompt { label: label.into() }
    }

    /// Select the next (or previous) item whose line contains the search
    /// text, ignoring case, expanding what hides it.
    fn search_next(&mut self, forward: bool) -> ChooseOutcome {
        if self.search.is_empty() || self.items.is_empty() {
            return ChooseOutcome::None;
        }
        let needle = self.search.to_lowercase();
        let n = self.items.len();
        let start = self.current_index().unwrap_or(0);
        for k in 1..=n {
            let i = if forward { (start + k) % n } else { (start + n - k) % n };
            let it = &self.items[i];
            if it.name.to_lowercase().contains(&needle) || it.text.to_lowercase().contains(&needle) {
                // Open every ancestor.
                let mut depth = it.depth;
                for j in (0..i).rev() {
                    if self.items[j].depth < depth {
                        depth = self.items[j].depth;
                        self.expanded.insert(self.items[j].id.clone());
                    }
                }
                self.current = self.items[i].id.clone();
                return ChooseOutcome::Redraw;
            }
        }
        ChooseOutcome::None
    }

    /// A key for the mode.
    pub fn key(&mut self, key: &KeyEvent) -> ChooseOutcome {
        let Some(cur) = self.current_index() else {
            return ChooseOutcome::Exit;
        };
        let plain = |c: char| key.key == Key::Char(c) && key.mods == Mods::NONE;
        let is = |name: &str| key_is(key, name);
        // Shortcuts first: the digits and M-letters shown on the lines.
        let vis = self.visible();
        if let Some(code) = KeyCode::from_event(key) {
            let name = code.to_string();
            for (line, i) in vis.iter().enumerate() {
                let sc = match &self.spec.key_format {
                    None => shortcut(line),
                    Some(_) => None,
                };
                if sc.as_deref() == Some(name.as_str()) {
                    return ChooseOutcome::Choose(self.command_for(&self.spec.template.clone(), *i));
                }
            }
        }
        let kind = self.spec.kind;
        if is("Enter") {
            return ChooseOutcome::Choose(self.command_for(&self.spec.template.clone(), cur));
        }
        if plain('q') || is("Escape") || is("C-g") {
            return ChooseOutcome::Exit;
        }
        if is("Up") || plain('k') || is("C-p") {
            return self.step(-1);
        }
        if is("Down") || plain('j') || is("C-n") {
            return self.step(1);
        }
        if is("PPage") || is("C-b") {
            return self.step(-(self.page.max(1) as isize));
        }
        if is("NPage") || is("C-f") {
            return self.step(self.page.max(1) as isize);
        }
        if is("Home") || plain('g') {
            self.select(&vis, 0);
            return ChooseOutcome::Redraw;
        }
        if is("End") || plain('G') {
            self.select(&vis, vis.len().saturating_sub(1));
            return ChooseOutcome::Redraw;
        }
        if plain('+') || is("Right") || plain('l') {
            if self.has_children(cur) {
                self.expanded.insert(self.current.clone());
            }
            return ChooseOutcome::Redraw;
        }
        if plain('-') || is("Left") || plain('h') {
            if self.has_children(cur) && self.expanded.contains(&self.current) {
                self.expanded.remove(&self.current);
            } else if let Some(parent) = (0..cur).rev().find(|j| self.items[*j].depth < self.items[cur].depth) {
                // On a leaf (or a closed item): up to its parent.
                self.current = self.items[parent].id.clone();
            }
            return ChooseOutcome::Redraw;
        }
        if is("M-+") {
            for i in 0..self.items.len() {
                if self.has_children(i) {
                    self.expanded.insert(self.items[i].id.clone());
                }
            }
            return ChooseOutcome::Redraw;
        }
        if is("M--") {
            self.expanded.clear();
            // The selection moves to its session, the only lines left.
            if let Some(top) = (0..=cur).rev().find(|j| self.items[*j].depth == 0) {
                self.current = self.items[top].id.clone();
            }
            return ChooseOutcome::Redraw;
        }
        if plain('t') {
            if !self.tagged.remove(&self.current) {
                self.tagged.insert(self.current.clone());
            }
            return self.step(1);
        }
        if plain('T') {
            self.tagged.clear();
            return ChooseOutcome::Redraw;
        }
        if is("C-t") {
            self.tagged = self.items.iter().map(|i| i.id.clone()).collect();
            return ChooseOutcome::Redraw;
        }
        if is("C-s") {
            return self.ask(Asking::Search, "(search)");
        }
        if plain('n') {
            return self.search_next(true);
        }
        if plain('N') {
            return self.search_next(false);
        }
        if plain('f') {
            return self.ask(Asking::Filter, "(filter)");
        }
        if plain('c') {
            self.spec.filter = None;
            return ChooseOutcome::Rebuild;
        }
        if plain('O') {
            self.spec.sort = (self.spec.sort + 1) % kind.sort_orders().len();
            return ChooseOutcome::Rebuild;
        }
        if plain('r') {
            self.spec.reverse = !self.spec.reverse;
            return ChooseOutcome::Rebuild;
        }
        if plain('v') {
            self.spec.preview = !self.spec.preview;
            return ChooseOutcome::Redraw;
        }
        if plain('H') {
            if self.items.iter().any(|i| i.id == self.home) {
                self.current = self.home.clone();
                let i = self.current_index().unwrap_or(0);
                let mut depth = self.items[i].depth;
                for j in (0..i).rev() {
                    if self.items[j].depth < depth {
                        depth = self.items[j].depth;
                        self.expanded.insert(self.items[j].id.clone());
                    }
                }
            }
            return ChooseOutcome::Redraw;
        }
        if plain(':') {
            let label = match self.tagged.len() {
                0 => "(current)".to_string(),
                n => format!("({n} tagged)"),
            };
            return self.ask(Asking::Command, label);
        }
        let all = |s: &ChooseState, template: &str| -> Vec<String> { s.tagged_or_current().iter().map(|i| s.command_for(template, *i)).collect() };
        match kind {
            ChooseKind::Tree => {
                if plain('x')
                    && let Some((question, command)) = self.items[cur].kill.clone()
                {
                    return self.ask(Asking::Confirm(vec![command]), question);
                }
                if plain('X') && !self.tagged.is_empty() {
                    let commands: Vec<String> =
                        self.items.iter().filter(|i| self.tagged.contains(&i.id)).filter_map(|i| i.kill.as_ref().map(|k| k.1.clone())).collect();
                    let label = format!("Kill {} tagged?", commands.len());
                    return self.ask(Asking::Confirm(commands), label);
                }
            }
            ChooseKind::Buffers => {
                if plain('p') {
                    return ChooseOutcome::Choose(self.command_for("paste-buffer -p -b '%%'", cur));
                }
                if plain('P') {
                    return ChooseOutcome::Run(all(self, "paste-buffer -p -b '%%'"));
                }
                if plain('d') {
                    return ChooseOutcome::Run(vec![self.command_for("delete-buffer -b '%%'", cur)]);
                }
                if plain('D') {
                    return ChooseOutcome::Run(all(self, "delete-buffer -b '%%'"));
                }
            }
            ChooseKind::Clients => {
                let one = |s: &ChooseState, t: &str| ChooseOutcome::Run(vec![s.command_for(t, cur)]);
                if plain('d') {
                    return one(self, "detach-client -t '%%'");
                }
                if plain('D') {
                    return ChooseOutcome::Run(all(self, "detach-client -t '%%'"));
                }
                if plain('x') {
                    return one(self, "detach-client -P -t '%%'");
                }
                if plain('X') {
                    return ChooseOutcome::Run(all(self, "detach-client -P -t '%%'"));
                }
                if plain('z') {
                    return one(self, "suspend-client -t '%%'");
                }
                if plain('Z') {
                    return ChooseOutcome::Run(all(self, "suspend-client -t '%%'"));
                }
            }
        }
        ChooseOutcome::None
    }

    /// Line `row` of the list was clicked (`double`: chosen); `None` for
    /// the wheel, which moves the selection by `wheel` lines.
    pub fn mouse(&mut self, row: Option<u16>, double: bool, wheel: isize) -> ChooseOutcome {
        let Some(row) = row else {
            return if wheel == 0 { ChooseOutcome::None } else { self.step(wheel.signum()) };
        };
        let vis = self.visible();
        let Some(i) = vis.get(self.offset + row as usize).copied() else { return ChooseOutcome::None };
        self.current = self.items[i].id.clone();
        if double {
            return ChooseOutcome::Choose(self.command_for(&self.spec.template.clone(), i));
        }
        ChooseOutcome::Redraw
    }

    /// The prompt asked for was answered.
    pub fn prompt_accept(&mut self, text: &str) -> ChooseOutcome {
        match self.asking.take() {
            Some(Asking::Search) => {
                self.search = text.to_string();
                self.search_next(true)
            }
            Some(Asking::Filter) => {
                self.spec.filter = (!text.is_empty()).then(|| text.to_string());
                ChooseOutcome::Rebuild
            }
            Some(Asking::Command) if !text.trim().is_empty() => {
                ChooseOutcome::Run(self.tagged_or_current().iter().map(|i| self.command_for(text, *i)).collect())
            }
            Some(Asking::Confirm(commands)) if matches!(text, "y" | "Y") => ChooseOutcome::Run(commands),
            _ => ChooseOutcome::Redraw,
        }
    }

    /// The prompt was cancelled.
    pub fn prompt_cancel(&mut self) -> ChooseOutcome {
        self.asking = None;
        ChooseOutcome::Redraw
    }

    /// Whether the mode waits for a yes or no (one key answers).
    pub fn confirming(&self) -> bool {
        matches!(self.asking, Some(Asking::Confirm(_)))
    }

    /// What to draw in a pane `cols` by `rows`. Scrolls the list so the
    /// selection is on it.
    pub fn screen(&mut self, cols: u16, rows: u16) -> ChooseScreen {
        let vis = self.visible();
        let sy = rows as usize;
        // tmux's mode_tree_set_height: two thirds for a long list, half
        // for a short one, and no preview in a small pane.
        let mut height = if self.spec.preview { (sy / 3) * 2 } else { sy };
        if self.spec.preview {
            if height > vis.len() {
                height = sy / 2;
            }
            if height < 10 || sy - height < 2 {
                height = sy;
            }
        }
        self.page = height.max(1);
        let pos = vis.iter().position(|i| self.items[*i].id == self.current).unwrap_or(0);
        if pos < self.offset {
            self.offset = pos;
        } else if height > 0 && pos >= self.offset + height {
            self.offset = pos + 1 - height;
        }
        self.offset = self.offset.min(vis.len().saturating_sub(height.min(vis.len())));
        let mut lines = Vec::with_capacity(height.min(vis.len()));
        for (line, &i) in vis.iter().enumerate().skip(self.offset).take(height) {
            lines.push(ChooseLine { text: self.line_text(&vis, line, i, cols as usize), selected: self.items[i].id == self.current });
        }
        let cur = self.current_index();
        let title = (height < sy).then(|| {
            let name = cur.map(|i| self.items[i].name.as_str()).unwrap_or("");
            let order = self.spec.kind.sort_orders()[self.spec.sort];
            let rev = if self.spec.reverse { ", reversed" } else { "" };
            let filter = match &self.spec.filter {
                Some(_) => " (filter: active)",
                None => "",
            };
            match self.spec.kind {
                ChooseKind::Buffers => format!("{name} (sort: {order}{rev}){filter}"),
                _ => format!("{name} (sort: {order}{rev}){filter} (view: preview)"),
            }
        });
        let preview = match (&title, cur) {
            (Some(_), Some(i)) => self.items[i].preview.clone(),
            _ => Preview::None,
        };
        ChooseScreen { lines, list_rows: height as u16, title, preview }
    }

    /// `(key) tree name: text` for line number `line` showing item `i`.
    fn line_text(&self, vis: &[usize], line: usize, i: usize, cols: usize) -> String {
        let it = &self.items[i];
        let mut s = String::new();
        if let Some(k) = shortcut(line).filter(|_| self.spec.key_format.is_none()) {
            s.push_str(&format!("({k}) "));
        } else if self.spec.key_format.is_none() {
            // Past the shortcuts the text stays in its column.
            s.push_str("    ");
        }
        // The tree: for each ancestor level, a line if it has more
        // siblings below; then the branch to this item.
        if it.depth > 0 {
            let later = |from: usize, depth: u8| -> bool {
                // Whether another item of `depth` follows before the
                // tree goes back above it.
                for &j in vis.iter().skip_while(|j| **j <= from) {
                    let d = self.items[j].depth;
                    if d == depth {
                        return true;
                    }
                    if d < depth {
                        return false;
                    }
                }
                false
            };
            for d in 1..it.depth {
                s.push_str(if later(i, d) { "│   " } else { "    " });
            }
            s.push_str(if later(i, it.depth) { "├─→ " } else { "└─→ " });
        }
        if self.has_children(i) {
            s.push_str(if self.expanded.contains(&it.id) { "- " } else { "+ " });
        }
        s.push_str(&it.name);
        if self.tagged.contains(&it.id) {
            s.push('*');
        }
        s.push_str(": ");
        s.push_str(&it.text);
        // Cut to the pane (by characters; the server cuts by cells too).
        if s.chars().count() > cols {
            s = s.chars().take(cols).collect();
        }
        s
    }
}

/// tmux's default format of a tree item (its `tree_mode_format` without
/// the parts for marked panes and session groups, which lif has not).
const TREE_FORMAT: &str = "#{?pane_format,#{pane_current_command}#{?pane_active,*,}#{?#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}},: \"#{pane_title}\",},#{?window_format,#{window_name}#{window_flags}#{?#{&&:#{==:#{window_panes},1},#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}}},: \"#{pane_title}\",},#{session_windows} windows#{?session_attached, (attached),}}}";
const BUFFER_FORMAT: &str = "#{choose_time}: #{buffer_sample}";
const CLIENT_FORMAT: &str = "#{choose_time}: session #{session_name}";

/// The variables only chooser formats have, over the usual ones.
struct ItemVars<'a> {
    inner: &'a dyn Vars,
    /// `pane`, `window` or `session`: which of `pane_format`,
    /// `window_format`, `session_format` is 1.
    level: &'a str,
    time: i64,
    line: usize,
}

impl Vars for ItemVars<'_> {
    fn var(&self, name: &str) -> Option<String> {
        match name {
            "pane_format" | "window_format" | "session_format" => {
                Some(if name.strip_suffix("_format") == Some(self.level) { "1" } else { "0" }.to_string())
            }
            // tmux's `t/p:` time: hours and minutes of today.
            "choose_time" => {
                let tm = Tm::from_clock(self.inner.now().at(self.time));
                Some(crate::time::strftime("%H:%M", &tm))
            }
            "line" => Some(self.line.to_string()),
            _ => self.inner.var(name),
        }
    }
    fn now(&self) -> WallClock {
        self.inner.now()
    }
    fn shell(&self, command: &str) -> Option<String> {
        self.inner.shell(command)
    }
}

fn cut_text(mut s: String) -> String {
    if s.len() > MAX_ITEM_TEXT {
        let mut end = MAX_ITEM_TEXT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s.retain(|c| !c.is_control());
    s
}

type R = Result<CmdOutput, CmdError>;

impl Model {
    /// The list of a chooser, in display order: every session with its
    /// windows and their panes (or the buffers, or the attached clients),
    /// sorted and filtered as `spec` says, at most
    /// [`MAX_CHOOSE_ITEMS`].
    pub fn choose_items(&self, spec: &ChooseSpec, client: Option<ClientId>, now: WallClock, host: &dyn Host) -> Vec<ChooseItem> {
        let filtered = self.choose_build(spec, client, now, host, true);
        // A filter that leaves nothing is ignored, as in tmux.
        if filtered.is_empty() && spec.filter.is_some() { self.choose_build(spec, client, now, host, false) } else { filtered }
    }

    fn choose_build(&self, spec: &ChooseSpec, client: Option<ClientId>, now: WallClock, host: &dyn Host, filter: bool) -> Vec<ChooseItem> {
        let order = spec.kind.sort_orders()[spec.sort];
        let keep = |vars: &dyn Vars| match (&spec.filter, filter) {
            (Some(f), true) => format::is_true(&format::expand(f, vars)),
            _ => true,
        };
        let mut out = Vec::new();
        match spec.kind {
            ChooseKind::Tree => {
                let fmt = spec.format.as_deref().unwrap_or(TREE_FORMAT);
                let mut sessions: Vec<SessionId> = self.sessions.keys().copied().collect();
                match order {
                    "name" => sessions.sort_by(|a, b| self.sessions[a].name.cmp(&self.sessions[b].name)),
                    "activity" => sessions.sort_by(|a, b| self.sessions[b].activity.cmp(&self.sessions[a].activity)),
                    _ => {}
                }
                if spec.reverse {
                    sessions.reverse();
                }
                for sid in sessions {
                    let s = &self.sessions[&sid];
                    let mut windows: Vec<WindowId> = s.windows.values().copied().collect();
                    if order == "name" {
                        windows.sort_by(|a, b| self.windows[a].name.cmp(&self.windows[b].name));
                    }
                    if spec.reverse {
                        windows.reverse();
                    }
                    if let Some(only) = &spec.only_windows {
                        windows.retain(|w| only.contains(w));
                    }
                    let at = out.len();
                    for wid in windows {
                        let w = &self.windows[&wid];
                        let wat = out.len();
                        let mut panes: Vec<PaneId> = w.panes.clone();
                        if spec.reverse {
                            panes.reverse();
                        }
                        for pid in &panes {
                            let c = self.fctx(host, Some(sid), Some(wid), Some(*pid), client, now);
                            let vars = ItemVars { inner: &c, level: "pane", time: 0, line: 0 };
                            if !keep(&vars) {
                                continue;
                            }
                            let idx = self.pane_index(*pid).unwrap_or(0);
                            out.push(ChooseItem {
                                id: pid.to_string(),
                                target: format!("={}:{}.{pid}", s.name, w.index),
                                depth: 2,
                                name: idx.to_string(),
                                text: cut_text(format::expand(fmt, &vars)),
                                preview: Preview::Panes(vec![(*pid, String::new())]),
                                kill: Some((format!("Kill pane {idx}?"), format!("kill-pane -t {pid}"))),
                            });
                        }
                        // A window is listed if a pane of it is (or it
                        // passes the filter itself).
                        let c = self.fctx(host, Some(sid), Some(wid), Some(w.active), client, now);
                        let vars = ItemVars { inner: &c, level: "window", time: 0, line: 0 };
                        if out.len() == wat && !keep(&vars) {
                            continue;
                        }
                        let labels = w
                            .panes
                            .iter()
                            .map(|p| {
                                let pc = self.fctx(host, Some(sid), Some(wid), Some(*p), client, now);
                                (*p, cut_text(format::expand("#{pane_index}:#{pane_title}", &pc)))
                            })
                            .collect();
                        out.insert(
                            wat,
                            ChooseItem {
                                id: wid.to_string(),
                                target: format!("={}:{}.", s.name, w.index),
                                depth: 1,
                                name: w.index.to_string(),
                                text: cut_text(format::expand(fmt, &vars)),
                                preview: Preview::Panes(labels),
                                kill: Some((format!("Kill window {}?", w.index), format!("kill-window -t {wid}"))),
                            },
                        );
                    }
                    let active = self.windows.get(&s.current).map(|w| w.active);
                    let c = self.fctx(host, Some(sid), Some(s.current), active, client, now);
                    let vars = ItemVars { inner: &c, level: "session", time: 0, line: 0 };
                    if out.len() == at && (spec.only_windows.is_some() || !keep(&vars)) {
                        continue;
                    }
                    let labels = s
                        .windows
                        .values()
                        .filter_map(|wid| self.windows.get(wid))
                        .map(|w| (w.active, cut_text(format!("{}:{}", w.index, w.name))))
                        .collect();
                    out.insert(
                        at,
                        ChooseItem {
                            id: sid.to_string(),
                            target: format!("={}:", s.name),
                            depth: 0,
                            name: s.name.clone(),
                            text: cut_text(format::expand(fmt, &vars)),
                            preview: Preview::Panes(labels),
                            kill: Some((format!("Kill session {}?", s.name), format!("kill-session -t {sid}"))),
                        },
                    );
                    if out.len() >= MAX_CHOOSE_ITEMS {
                        break;
                    }
                }
            }
            ChooseKind::Buffers => {
                let fmt = spec.format.as_deref().unwrap_or(BUFFER_FORMAT);
                let mut idx: Vec<usize> = (0..self.buffers.len()).collect();
                match order {
                    "name" => idx.sort_by(|a, b| self.buffers[*a].name.cmp(&self.buffers[*b].name)),
                    "size" => idx.sort_by(|a, b| self.buffers[*b].data.len().cmp(&self.buffers[*a].data.len())),
                    _ => {}
                }
                if spec.reverse {
                    idx.reverse();
                }
                for i in idx.into_iter().take(MAX_CHOOSE_ITEMS) {
                    let b = &self.buffers[i];
                    let mut c = self.fctx(host, None, None, None, client, now);
                    c.buffer = Some(b);
                    let vars = ItemVars { inner: &c, level: "", time: b.created, line: 0 };
                    if !keep(&vars) {
                        continue;
                    }
                    let shown = &b.data[..b.data.len().min(MAX_PREVIEW_TEXT)];
                    out.push(ChooseItem {
                        id: b.name.clone(),
                        target: b.name.clone(),
                        depth: 0,
                        name: b.name.clone(),
                        text: cut_text(format::expand(fmt, &vars)),
                        preview: Preview::Text(String::from_utf8_lossy(shown).into_owned()),
                        kill: None,
                    });
                }
            }
            ChooseKind::Clients => {
                let fmt = spec.format.as_deref().unwrap_or(CLIENT_FORMAT);
                let mut clients: Vec<&crate::model::Client> = self.clients.values().filter(|c| c.session.is_some()).collect();
                match order {
                    "size" => clients.sort_by_key(|c| std::cmp::Reverse(c.info.size.cols as u32 * c.info.size.rows as u32)),
                    "creation" => clients.sort_by_key(|c| c.id),
                    "activity" => clients.sort_by_key(|c| std::cmp::Reverse(c.activity)),
                    _ => clients.sort_by(|a, b| a.info.name.cmp(&b.info.name)),
                }
                if spec.reverse {
                    clients.reverse();
                }
                for cl in clients.into_iter().take(MAX_CHOOSE_ITEMS) {
                    let sid = cl.session;
                    let cw = sid.and_then(|s| self.sessions.get(&s)).map(|s| s.current);
                    let cp = cw.and_then(|w| self.windows.get(&w)).map(|w| w.active);
                    let c = self.fctx(host, sid, cw, cp, Some(cl.id), now);
                    let vars = ItemVars { inner: &c, level: "", time: now.unix_secs, line: 0 };
                    if !keep(&vars) {
                        continue;
                    }
                    out.push(ChooseItem {
                        id: cl.info.name.clone(),
                        target: cl.info.name.clone(),
                        depth: 0,
                        name: cl.info.name.clone(),
                        text: cut_text(format::expand(fmt, &vars)),
                        preview: cp.map_or(Preview::None, |p| Preview::Panes(vec![(p, String::new())])),
                        kill: None,
                    });
                }
            }
        }
        out.truncate(MAX_CHOOSE_ITEMS);
        out
    }

    /// The item a chooser starts on for `client`: its current pane (the
    /// mode moves up to the nearest line that is shown).
    pub fn choose_start(&self, kind: ChooseKind, client: ClientId) -> Option<String> {
        let c = self.clients.get(&client)?;
        match kind {
            ChooseKind::Tree => {
                let s = self.sessions.get(&c.session?)?;
                Some(self.windows.get(&s.current)?.active.to_string())
            }
            ChooseKind::Buffers => None,
            ChooseKind::Clients => Some(c.info.name.clone()),
        }
    }

    /// The mode in `pane` is over: undo the zoom `-Z` made, if the pane
    /// is still the zoomed one.
    pub fn chooser_closed(&mut self, pane: PaneId, zoomed: bool, host: &mut dyn Host) {
        if !zoomed {
            return;
        }
        let Some(wid) = self.panes.get(&pane).map(|p| p.window) else { return };
        let Some(w) = self.windows.get_mut(&wid) else { return };
        if w.layout.zoomed() == Some(pane) {
            w.layout.unzoom();
            self.sync_window(wid, host);
        }
    }

    fn choose_spec(&mut self, a: &Args, kind: ChooseKind, ctx: &CmdContext, host: &mut dyn Host) -> Result<(ClientId, PaneId, ChooseSpec), CmdError> {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let pane = f.p.ok_or_else(|| CmdError::new("no current pane"))?;
        // tmux needs a client to show the mode to; so does lif.
        let client = ctx.client.filter(|c| self.clients.get(c).is_some_and(|c| c.session.is_some()));
        let client = client.or_else(|| self.clients.values().find(|c| c.session == Some(f.s)).map(|c| c.id));
        let client = client.or_else(|| self.clients.values().find(|c| c.session.is_some()).map(|c| c.id));
        let client = client.ok_or_else(|| CmdError::new("no clients"))?;
        let orders = kind.sort_orders();
        let sort = match a.get('O') {
            // tmux calls the tree's activity order "time".
            Some(o) => orders.iter().position(|x| *x == o || (o == "time" && matches!(*x, "activity" | "creation"))).ok_or_else(|| CmdError(format!("invalid sort order: {o}")))?,
            None => 0,
        };
        let mut zoomed = false;
        if a.has('Z')
            && let Some(wid) = f.w
            && self.windows[&wid].panes.len() > 1
            && self.windows[&wid].layout.zoomed().is_none()
        {
            self.windows.get_mut(&wid).expect("window").layout.zoom(pane);
            self.sync_window(wid, host);
            zoomed = true;
        }
        let spec = ChooseSpec {
            kind,
            format: a.get('F').map(str::to_string),
            filter: a.get('f').map(str::to_string),
            key_format: None,
            sort,
            reverse: a.has('r'),
            template: a.value(0).unwrap_or(kind.default_template()).to_string(),
            sessions_collapsed: a.has('s'),
            windows_collapsed: a.has('w'),
            preview: !a.has('N'),
            only_windows: None,
            zoomed,
        };
        Ok((client, pane, spec))
    }

    pub(crate) fn cmd_choose(&mut self, a: &Args, kind: ChooseKind, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let (client, pane, spec) = self.choose_spec(a, kind, ctx, host)?;
        let zoomed = spec.zoomed;
        if let Err(e) = host.choose(client, pane, spec) {
            self.chooser_closed(pane, zoomed, host);
            return Err(CmdError(e));
        }
        Ok(CmdOutput::default())
    }

    /// `find-window`: the tree, with only the windows that match.
    pub(crate) fn cmd_find_window(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if a.has('r') {
            return Err(CmdError::new("find-window -r is not supported: lif matches with shell patterns"));
        }
        let pattern = a.value(0).unwrap_or("");
        // tmux matches `*pattern*` against names, titles and contents
        // unless flags pick some of them.
        let (mut names, mut titles, mut contents) = (a.has('N'), a.has('T'), a.has('C'));
        if !names && !titles && !contents {
            (names, titles, contents) = (true, true, true);
        }
        let fold = |s: &str| if a.has('i') { s.to_lowercase() } else { s.to_string() };
        let glob = format!("*{}*", fold(pattern));
        let hit = |s: &str| format::glob(glob.as_bytes(), fold(s).as_bytes());
        let mut only = Vec::new();
        for (wid, w) in &self.windows {
            let mut found = names && hit(&w.name);
            for p in &w.panes {
                if found {
                    break;
                }
                let title = self.fctx(&*host, Some(w.session), Some(*wid), Some(*p), None, ctx.now).var("pane_title").unwrap_or_default();
                found = (titles && hit(&title))
                    || (contents && {
                        let range = crate::host::CaptureRange {
                            start: crate::host::LinePos::Line(0),
                            end: crate::host::LinePos::ScreenEnd,
                            join_wrapped: false,
                            keep_trailing: false,
                            styled: false,
                        };
                        host.capture_pane(*p, range).lines().any(hit)
                    });
            }
            if found {
                only.push(*wid);
            }
        }
        let (client, pane, mut spec) = self.choose_spec(a, ChooseKind::Tree, ctx, host)?;
        // (Its argument is the text to find, not a command.)
        spec.template = ChooseKind::Tree.default_template().to_string();
        // No match: the whole tree, as tmux shows it.
        spec.only_windows = (!only.is_empty()).then_some(only);
        let zoomed = spec.zoomed;
        if let Err(e) = host.choose(client, pane, spec) {
            self.chooser_closed(pane, zoomed, host);
            return Err(CmdError(e));
        }
        Ok(CmdOutput::default())
    }
}
