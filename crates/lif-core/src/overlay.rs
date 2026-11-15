//! Popups and menus: `display-popup` and `display-menu`.
//!
//! Both are drawn over a client's panes and take its keys while they are
//! up. The model works out where they go and what they hold; the server
//! owns the popup's program and draws both. A client has at most one of
//! them at a time.
//!
//! Sizes, positions (including tmux's off-by-one centre), borders, the
//! menu's layout and the exit statuses are tmux 3.8's, read off its screen.

use crate::cmd::{Arg, Args, CmdError};
use crate::host::{MouseAt, Host, JobStatus, SpawnCommand};
use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use crate::keys::KeyCode;
use crate::layout::SizeSpec;
use crate::model::{CmdContext, CmdOutput, Model};
use crate::queue::{Block, Then, WaitKey};
use crate::target::{Flags, Found, Kind};
use lif_types::{KeyEvent, Rect, Size};

/// Most items of one menu.
pub const MAX_MENU_ITEMS: usize = 200;

/// Longest name of a menu item or title, in characters.
const MAX_MENU_TEXT: usize = 200;

/// The lines a popup's or menu's border is drawn with (`-b`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderLines {
    /// `┌─┐`, tmux's default.
    #[default]
    Single,
    /// `╭─╮`.
    Rounded,
    /// `╔═╗`.
    Double,
    /// `┏━┓`.
    Heavy,
    /// `+-+`.
    Simple,
    /// Blanks.
    Padded,
    /// No border at all (`-B`).
    None,
}

impl BorderLines {
    fn parse(s: &str) -> Result<BorderLines, CmdError> {
        Ok(match s {
            "single" => BorderLines::Single,
            "rounded" => BorderLines::Rounded,
            "double" => BorderLines::Double,
            "heavy" => BorderLines::Heavy,
            "simple" => BorderLines::Simple,
            "padded" => BorderLines::Padded,
            "none" => BorderLines::None,
            _ => return Err(CmdError(format!("invalid border lines: {s}"))),
        })
    }

    /// The glyphs: top left, top right, bottom left, bottom right,
    /// horizontal, vertical, and the tees a menu's separator joins with.
    pub fn glyphs(self) -> [char; 8] {
        match self {
            BorderLines::Single => ['┌', '┐', '└', '┘', '─', '│', '├', '┤'],
            BorderLines::Rounded => ['╭', '╮', '╰', '╯', '─', '│', '├', '┤'],
            BorderLines::Double => ['╔', '╗', '╚', '╝', '═', '║', '╠', '╣'],
            BorderLines::Heavy => ['┏', '┓', '┗', '┛', '━', '┃', '┣', '┫'],
            BorderLines::Simple => ['+', '+', '+', '+', '-', '|', '+', '+'],
            BorderLines::Padded | BorderLines::None => [' '; 8],
        }
    }
}

/// When a popup goes away by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupClose {
    /// Never: it stays, showing what the program left, until Escape.
    Never,
    /// When the program exits (`-E`).
    OnExit,
    /// When the program exits with status 0 (`-EE`).
    OnSuccess,
}

/// A popup to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupSpec {
    /// Handed to [`Model::job_done`] when the popup closes, with the
    /// program's status if it closed because the program exited (0
    /// otherwise).
    pub id: u64,
    /// Where it goes on the client's screen, border included.
    pub rect: Rect,
    /// Its border.
    pub border: BorderLines,
    /// On the top border (empty: none).
    pub title: String,
    /// Style of the inside (`-s`) and of the border (`-S`), tmux style
    /// strings; empty: the terminal's default.
    pub style: String,
    /// See `style`.
    pub border_style: String,
    /// What runs in it.
    pub command: SpawnCommand,
    /// Its directory.
    pub cwd: String,
    /// Variables on top of its environment.
    pub env: Vec<(String, Option<String>)>,
    /// The session and window it belongs to, and the pane it was opened
    /// for (its `LIF_PANE`).
    pub session: SessionId,
    /// See `session`.
    pub window: WindowId,
    /// See `session`.
    pub pane: PaneId,
    /// When it closes by itself.
    pub close: PopupClose,
    /// `-k`: once the program has exited, any key closes it (otherwise
    /// only Escape and C-c).
    pub any_key: bool,
    /// The `tmux-compat` option: its program gets `TMUX` too.
    pub tmux_compat: bool,
}

/// One line of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// A line across the menu.
    Separator,
    /// Something to choose.
    Entry {
        /// What it says.
        name: String,
        /// The key that chooses it, as shown (`f`, `C-r`); empty: none.
        key: String,
        /// The command it runs (empty: it only closes the menu).
        command: String,
        /// It can be chosen (a name starting with `-` cannot).
        enabled: bool,
    },
}

/// What a key or click on a menu leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    /// Nothing.
    None,
    /// Draw it again.
    Redraw,
    /// Close it and run this command (empty: just close).
    Run(String),
    /// Close it.
    Close,
}

/// A menu on a client's screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuState {
    /// Where it goes, border included.
    pub rect: Rect,
    /// Centred on its top border.
    pub title: String,
    /// Its lines.
    pub items: Vec<MenuItem>,
    /// The selected line, if any.
    pub selected: Option<usize>,
    /// Its border.
    pub border: BorderLines,
    /// Styles of the menu, the selected item and the border (`-s`,
    /// `-H`, `-S`); empty: defaults.
    pub style: String,
    /// See `style`.
    pub selected_style: String,
    /// See `style`.
    pub border_style: String,
    /// The pane its commands run for.
    pub pane: Option<PaneId>,
    /// Opened with a mouse button held, without `-O`: letting the button
    /// go chooses the item under the mouse or closes the menu, as in tmux.
    pub release_chooses: bool,
}

fn width(s: &str) -> usize {
    crate::status::text_width(s)
}

impl MenuState {
    fn enabled(&self, i: usize) -> bool {
        matches!(self.items.get(i), Some(MenuItem::Entry { enabled: true, .. }))
    }

    /// The next (or previous) line that can be chosen, wrapping around.
    fn step(&mut self, forward: bool) -> MenuOutcome {
        let n = self.items.len();
        if !(0..n).any(|i| self.enabled(i)) {
            return MenuOutcome::None;
        }
        let mut i = match (self.selected, forward) {
            (Some(i), _) => i,
            (None, true) => n - 1,
            (None, false) => 0,
        };
        loop {
            i = if forward { (i + 1) % n } else { (i + n - 1) % n };
            if self.enabled(i) {
                self.selected = Some(i);
                return MenuOutcome::Redraw;
            }
        }
    }

    fn choose(&self, i: usize) -> MenuOutcome {
        match self.items.get(i) {
            Some(MenuItem::Entry { command, enabled: true, .. }) => MenuOutcome::Run(command.clone()),
            _ => MenuOutcome::None,
        }
    }

    /// A key while the menu is up: every key is the menu's.
    pub fn key(&mut self, key: &KeyEvent) -> MenuOutcome {
        let Some(code) = KeyCode::from_event(key) else { return MenuOutcome::None };
        let name = code.to_string();
        // An item's own key first: it may be q or j.
        for (i, it) in self.items.iter().enumerate() {
            if let MenuItem::Entry { key: k, enabled: true, .. } = it
                && !k.is_empty()
                && KeyCode::parse(k) == Some(code)
            {
                return self.choose(i);
            }
        }
        match name.as_str() {
            "Up" | "k" | "C-p" | "BTab" => self.step(false),
            "Down" | "j" | "C-n" | "Tab" => self.step(true),
            "Enter" => match self.selected {
                Some(i) => self.choose(i),
                None => MenuOutcome::Close,
            },
            "q" | "Escape" | "C-c" | "C-g" => MenuOutcome::Close,
            _ => MenuOutcome::None,
        }
    }

    /// The mouse at client cell (`x`, `y`): moving over an item selects
    /// it, a click chooses it; a click outside the menu closes it.
    pub fn mouse(&mut self, x: u16, y: u16, click: bool) -> MenuOutcome {
        let r = self.rect;
        let inside = x > r.x && x + 1 < r.right() && y > r.y && y + 1 < r.bottom();
        if !inside {
            return if click { MenuOutcome::Close } else { MenuOutcome::None };
        }
        let i = (y - r.y - 1) as usize;
        if !self.enabled(i) {
            return MenuOutcome::None;
        }
        if click {
            return self.choose(i);
        }
        if self.selected == Some(i) {
            return MenuOutcome::None;
        }
        self.selected = Some(i);
        MenuOutcome::Redraw
    }

    /// The button that opened the menu was let go at (`x`, `y`): on an
    /// item that is chosen; anywhere else the menu closes.
    pub fn mouse_release(&self, x: u16, y: u16) -> MenuOutcome {
        let r = self.rect;
        let inside = x > r.x && x + 1 < r.right() && y > r.y && y + 1 < r.bottom();
        if !inside {
            return MenuOutcome::Close;
        }
        match self.choose((y - r.y - 1) as usize) {
            MenuOutcome::None => MenuOutcome::Close,
            o => o,
        }
    }

    /// The text of each line between the borders: ` name   (key) `;
    /// `None` for a separator.
    pub fn lines(&self) -> Vec<Option<String>> {
        let (names, keys) = self.columns();
        self.items
            .iter()
            .map(|it| match it {
                MenuItem::Separator => None,
                MenuItem::Entry { name, key, .. } => {
                    let mut s = format!(" {name}{}", " ".repeat(names - width(name)));
                    if keys > 0 {
                        let k = if key.is_empty() { String::new() } else { format!("({key})") };
                        s.push_str(&format!(" {k}{}", " ".repeat(keys - width(&k))));
                    }
                    s.push(' ');
                    Some(s)
                }
            })
            .collect()
    }

    /// Widths of the name column and of the key column (0: no item has a
    /// key).
    fn columns(&self) -> (usize, usize) {
        let mut names = 0;
        let mut keys = 0;
        for it in &self.items {
            if let MenuItem::Entry { name, key, .. } = it {
                names = names.max(width(name));
                if !key.is_empty() {
                    keys = keys.max(width(key) + 2);
                }
            }
        }
        (names, keys)
    }

    /// Width and height of the menu, border included.
    fn size(&self) -> (u32, u32) {
        let (names, keys) = self.columns();
        let inner = 1 + names + if keys > 0 { 1 + keys } else { 0 } + 1;
        (inner.max(width(&self.title)) as u32 + 2, self.items.len() as u32 + 2)
    }
}

/// tmux's `#[...]` style directives, which menu names and titles may
/// hold: lif draws those plain.
fn strip_styles(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("#[") {
        out.push_str(&rest[..i]);
        match rest[i..].find(']') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    let clean: String = out.chars().filter(|c| !c.is_control()).take(MAX_MENU_TEXT).collect();
    clean
}

/// Where a popup or menu of `w` by `h` goes on a screen of `screen`:
/// `x` and `y` are tmux's position arguments (`C` centre, `R` right
/// edge, `P` by the pane, `S` by the status line, a number: the left
/// column, and for `y` the row below the bottom). It is moved to fit.
#[allow(clippy::too_many_arguments)]
fn place(
    screen: Size,
    w: u32,
    h: u32,
    x: Option<&str>,
    y: Option<&str>,
    pane: Rect,
    status_row: Option<u16>,
    mouse: Option<MouseAt>,
) -> Result<Rect, CmdError> {
    let (sx, sy) = (screen.cols as i64, screen.rows as i64);
    let (w, h) = (w as i64, h as i64);
    // tmux's centre is one cell up and left of the middle.
    let centre_x = sx / 2 - w / 2 - 1;
    let centre_bottom = sy / 2 + h / 2 - 1;
    let left = match x.unwrap_or("C") {
        // Centred on the mouse, when a mouse key opened it; from where
        // the window's entry in the status line starts.
        "M" if mouse.is_some() => mouse.map_or(0, |m| m.screen.0 as i64) - w / 2,
        "W" if mouse.is_some_and(|m| m.status_x.is_some()) => mouse.and_then(|m| m.status_x).unwrap_or(0) as i64,
        "C" | "M" | "W" => centre_x,
        "R" => sx - w,
        "P" => pane.x as i64,
        n => n.parse::<i64>().map_err(|_| CmdError(format!("invalid position: {n}")))?,
    };
    let bottom = match y.unwrap_or("C") {
        "M" if mouse.is_some() => mouse.map_or(0, |m| m.screen.1 as i64) + h,
        "C" | "M" => centre_bottom,
        "W" if mouse.is_none() => centre_bottom,
        "P" => pane.bottom() as i64,
        // Next to the status line: above it at the bottom, below it at
        // the top.
        "S" | "W" => match status_row {
            Some(0) => 1 + h,
            Some(r) => r as i64,
            None => sy,
        },
        n => n.parse::<i64>().map_err(|_| CmdError(format!("invalid position: {n}")))?,
    };
    let left = left.clamp(0, (sx - w).max(0));
    let top = (bottom - h).clamp(0, (sy - h).max(0));
    Ok(Rect::new(left as u16, top as u16, w as u16, h as u16))
}

type R = Result<CmdOutput, CmdError>;

impl Model {
    /// The attached client an overlay goes to: `-c`, the one that ran the
    /// command, or any client of the target's session.
    fn overlay_client(&self, a: &Args, ctx: &CmdContext, f: &Found) -> Result<ClientId, CmdError> {
        if let Some(name) = a.get('c') {
            return self
                .clients
                .values()
                .find(|c| (c.info.name == name || c.id.to_string() == name) && c.session.is_some())
                .map(|c| c.id)
                .ok_or_else(|| CmdError(format!("can't find client: {name}")));
        }
        let attached = |c: &ClientId| self.clients.get(c).is_some_and(|c| c.session.is_some());
        ctx.client
            .filter(attached)
            .or_else(|| self.clients.values().find(|c| c.session == Some(f.s)).map(|c| c.id))
            .or_else(|| self.clients.values().find(|c| c.session.is_some()).map(|c| c.id))
            .ok_or_else(|| CmdError::new("no clients"))
    }

    /// The target pane's rectangle and the status row on `client`'s
    /// screen, for positions.
    fn overlay_anchor(&self, client: ClientId, f: &Found) -> (Size, Rect, Option<u16>) {
        let size = self.clients[&client].info.size;
        let view = self.client_view(client);
        let pane = view.as_ref().and_then(|v| v.panes.iter().find(|(p, _)| Some(*p) == f.p).map(|(_, r)| *r));
        (size, pane.unwrap_or(Rect::new(0, 0, size.cols, size.rows)), view.and_then(|v| v.status_row))
    }

    pub(crate) fn cmd_display_popup(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let client = self.overlay_client(a, ctx, &f)?;
        if a.has('C') {
            host.close_overlay(client);
            return Ok(CmdOutput::default());
        }
        // One at a time: a second popup is not opened (tmux 3.8).
        if host.has_overlay(client) {
            return Ok(CmdOutput::default());
        }
        let (screen, pane_rect, status_row) = self.overlay_anchor(client, &f);
        // -b, -s and -S, or the popup-* options of the target's window.
        let option = |name: &str| self.option_text(name, Some(f.s), f.w).unwrap_or_default();
        let border = match (a.has('B'), a.get('b')) {
            (true, _) => BorderLines::None,
            (false, Some(b)) => BorderLines::parse(b)?,
            (false, None) => BorderLines::parse(&option("popup-border-lines"))?,
        };
        let style = a.get('s').map(str::to_string).unwrap_or_else(|| option("popup-style"));
        let border_style = a.get('S').map(str::to_string).unwrap_or_else(|| option("popup-border-style"));
        let edge = if border == BorderLines::None { 0 } else { 2 };
        let dim = |flag: char, of: u16, what: &str| -> Result<u32, CmdError> {
            let n = match a.get(flag) {
                None => of as u32 / 2,
                Some(v) => match SizeSpec::parse(v) {
                    Some(SizeSpec::Cells(n)) => n,
                    Some(SizeSpec::Percent(p)) => (of as u64 * p as u64 / 100) as u32,
                    None => return Err(CmdError(format!("{what} invalid"))),
                },
            };
            if n < 1 + edge {
                return Err(CmdError(format!("{what} too small")));
            }
            if n > of as u32 {
                return Err(CmdError(format!("{what} too large")));
            }
            Ok(n)
        };
        let h = dim('h', screen.rows, "height")?;
        let w = dim('w', screen.cols, "width")?;
        let rect = place(screen, w, h, a.get('x'), a.get('y'), pane_rect, status_row, self.mouse)?;
        self.check_park()?;
        let found = &f;
        let expand = |s: &str| self.expand_found(found, ctx, &*host, s);
        let cwd = match a.get('d') {
            Some(d) => expand(d),
            None => self.default_cwd(ctx, Some(f.s)),
        };
        let mut env = self.environ_overrides(Some(f.s));
        env.extend(crate::environ::flag_env(a));
        let title = a.get('T').map(|t| strip_styles(&expand(t))).unwrap_or_default();
        let id = self.next_job();
        let spec = PopupSpec {
            id,
            rect,
            border,
            title,
            style,
            border_style,
            command: self.spawn_command(f.s, &a.strings(0)),
            cwd,
            env,
            session: f.s,
            window: f.w.ok_or_else(|| CmdError::new("no current window"))?,
            pane: f.p.ok_or_else(|| CmdError::new("no current pane"))?,
            close: match a.count('E') {
                0 => PopupClose::Never,
                1 => PopupClose::OnExit,
                _ => PopupClose::OnSuccess,
            },
            any_key: a.has('k'),
            tmux_compat: self.gopt("tmux-compat").as_flag(),
        };
        host.popup(client, spec).map_err(CmdError)?;
        // The commands after it wait until it closes, as in tmux.
        self.q.block = Some(Block { key: WaitKey::Job(id), then: Then::Status, background: false });
        Ok(CmdOutput::default())
    }

    pub(crate) fn cmd_display_menu(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let client = self.overlay_client(a, ctx, &f)?;
        if host.has_overlay(client) {
            return Ok(CmdOutput::default());
        }
        let found = &f;
        let expand = |s: &str| self.expand_found(found, ctx, &*host, s);
        // name key command, or a lone empty name: a separator.
        let args = a.values();
        let text = |v: &Arg| match v {
            Arg::Str(s) => s.clone(),
            Arg::Block(b) => b.to_string(),
        };
        let mut items: Vec<MenuItem> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let raw = text(&args[i]);
            if raw.is_empty() {
                // No separator first, and never two in a row.
                if !matches!(items.last(), None | Some(MenuItem::Separator)) {
                    items.push(MenuItem::Separator);
                }
                i += 1;
                continue;
            }
            if i + 2 >= args.len() {
                return Err(CmdError::new("not enough arguments"));
            }
            let (key, command) = (text(&args[i + 1]), text(&args[i + 2]));
            i += 3;
            // A name that expands to nothing leaves the item out.
            let name = strip_styles(&expand(&raw));
            if name.is_empty() {
                continue;
            }
            let (name, enabled) = match name.strip_prefix('-') {
                Some(n) => (n.to_string(), false),
                None => (name, true),
            };
            if !key.is_empty() && KeyCode::parse(&key).is_none() {
                return Err(CmdError(format!("unknown key: {key}")));
            }
            if items.len() >= MAX_MENU_ITEMS {
                return Err(CmdError(format!("too many menu items ({MAX_MENU_ITEMS})")));
            }
            items.push(MenuItem::Entry { name, key: if enabled { key } else { String::new() }, command, enabled });
        }
        if matches!(items.last(), Some(MenuItem::Separator)) {
            items.pop();
        }
        if items.is_empty() {
            return Ok(CmdOutput::default());
        }
        // -b, -s, -H and -S, or the menu-* options of the target's window.
        let option = |name: &str| self.option_text(name, Some(f.s), f.w).unwrap_or_default();
        let border = match a.get('b') {
            Some(b) => BorderLines::parse(b)?,
            None => BorderLines::parse(&option("menu-border-lines"))?,
        };
        let mut menu = MenuState {
            rect: Rect::default(),
            title: a.get('T').map(|t| strip_styles(&expand(t))).unwrap_or_default(),
            items,
            selected: None,
            border,
            style: a.get('s').map(str::to_string).unwrap_or_else(|| option("menu-style")),
            selected_style: a.get('H').map(str::to_string).unwrap_or_else(|| option("menu-selected-style")),
            border_style: a.get('S').map(str::to_string).unwrap_or_else(|| option("menu-border-style")),
            pane: f.p,
            release_chooses: self.mouse.is_some() && !a.has('O'),
        };
        // Opened by a key: the first item is selected (`-C` picks another;
        // `-C -` or `-M`, as for a menu opened with the mouse, none).
        match a.get('C') {
            Some("-") => {}
            None if a.has('M') || self.mouse.is_some() => {}
            Some(n) => {
                let i: usize = n.parse().map_err(|_| CmdError(format!("starting choice invalid: {n}")))?;
                if matches!(menu.items.get(i), Some(MenuItem::Entry { enabled: true, .. })) {
                    menu.selected = Some(i);
                }
            }
            None => {
                menu.step(true);
            }
        }
        let (screen, pane_rect, status_row) = self.overlay_anchor(client, &f);
        let (w, h) = menu.size();
        if w > screen.cols as u32 || h > screen.rows as u32 {
            return Err(CmdError::new("menu too large for the terminal"));
        }
        menu.rect = place(screen, w, h, a.get('x'), a.get('y'), pane_rect, status_row, self.mouse)?;
        host.menu(client, menu).map_err(CmdError)?;
        Ok(CmdOutput::default())
    }
}

/// The exit status a popup's waiter gets.
pub(crate) fn popup_status(status: JobStatus) -> i32 {
    match status {
        JobStatus::Exited(n) => n,
        JobStatus::Signaled(s) => 128 + s,
    }
}
