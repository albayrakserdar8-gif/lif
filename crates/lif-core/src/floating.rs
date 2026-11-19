//! Floating panes (tmux 3.8's `new-pane`): panes over the tiled ones, each
//! in its own bordered rectangle, back to front. Sizes and positions are
//! tmux's: `-x`/`-y`/`-X`/`-Y` (cells or `N%` of the window) give the
//! rectangle with its border, the pane is one cell inside it; without them
//! a pane is half the window wide and a quarter high (inside), cascaded
//! from the top left by four columns and two lines. `move-pane` moves one
//! (`-P` positions, `-X`/`-Y`, `-D`/`-U`/`-L`/`-R`, `-z` depth) and
//! `resize-pane` resizes one.
//!
//! Not here (tmux has them): modal panes (`-O`, `-K`, `-C`, `-D`), panes
//! above a zoomed one (`-A`), dragging with the mouse (`-M`), the `move`
//! key table. The last tiled pane of a window closes it with its floating
//! panes; tmux keeps such a window.

use crate::cmd::{Args, CmdError};
use crate::host::{Host, Redraw};
use crate::model::{CmdContext, CmdOutput, Model, Pane};
use crate::layout::SizeSpec;
use crate::options::OptionSet;
use crate::target::{Flags, Kind};
use lif_types::{Rect, Size};

type R = Result<CmdOutput, CmdError>;

/// A dimension (`-x 30`, `-x 50%`) of a window `of` cells wide or high.
fn dim(v: &str, of: u16) -> Result<u16, CmdError> {
    match SizeSpec::parse(v) {
        Some(SizeSpec::Cells(n)) => Ok(n.min(u16::MAX as u32) as u16),
        Some(SizeSpec::Percent(p)) => Ok((of as u32 * p.min(100) / 100) as u16),
        None => Err(CmdError(format!("invalid size: {v}"))),
    }
}

/// `r` inside a window of `ws`: at least 3x3 (a border and a cell),
/// at most the window, and moved back in if it hangs out.
fn clamp(r: Rect, ws: Size) -> Rect {
    let cols = r.cols.clamp(3.min(ws.cols), ws.cols.max(1));
    let rows = r.rows.clamp(3.min(ws.rows), ws.rows.max(1));
    Rect::new(r.x.min(ws.cols.saturating_sub(cols)), r.y.min(ws.rows.saturating_sub(rows)), cols, rows)
}

/// Where `move-pane -P` puts a rectangle of `c` x `r` in a window `ws`.
fn position(name: &str, c: u16, r: u16, ws: Size) -> Option<(u16, u16)> {
    let (right, bottom) = (ws.cols.saturating_sub(c), ws.rows.saturating_sub(r));
    let (cx, cy) = (right / 2, bottom / 2);
    let quarter = |x: u16, w: u16| (x + w / 2).saturating_sub(c / 2);
    let (hw, hh) = (ws.cols / 2, ws.rows / 2);
    Some(match name {
        "top-left" => (0, 0),
        "top-centre" => (cx, 0),
        "top-right" => (right, 0),
        "centre-left" => (0, cy),
        "centre" => (cx, cy),
        "centre-right" => (right, cy),
        "bottom-left" => (0, bottom),
        "bottom-centre" => (cx, bottom),
        "bottom-right" => (right, bottom),
        "top-left-centre" => (quarter(0, hw), (hh / 2).saturating_sub(r / 2)),
        "top-right-centre" => (quarter(hw, ws.cols - hw), (hh / 2).saturating_sub(r / 2)),
        "bottom-left-centre" => (quarter(0, hw), (hh + (ws.rows - hh) / 2).saturating_sub(r / 2)),
        "bottom-right-centre" => (quarter(hw, ws.cols - hw), (hh + (ws.rows - hh) / 2).saturating_sub(r / 2)),
        _ => return None,
    })
}

impl Model {
    /// `new-pane`: a floating pane (with `-L`, a split, as `split-window`).
    pub(crate) fn cmd_new_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if a.has('L') {
            return self.cmd_split(a, ctx, host);
        }
        for flag in ['A', 'C', 'D', 'I', 'K', 'M', 'O', 'W'] {
            if a.has(flag) {
                return Err(CmdError(format!("new-pane -{flag} is not supported")));
            }
        }
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let wid = f.w.ok_or_else(|| CmdError::new("no current window"))?;
        let ws = self.windows[&wid].size;
        let n = self.windows[&wid].floating.len() as u16 + 1;
        let cols = match a.get('x') {
            Some(v) => dim(v, ws.cols)?,
            None => ws.cols / 2 + 2,
        };
        let rows = match a.get('y') {
            Some(v) => dim(v, ws.rows)?,
            None => ws.rows / 4 + 2,
        };
        let x = match a.get('X') {
            Some(v) => dim(v, ws.cols)?,
            None => (4 * n).saturating_sub(1),
        };
        let y = match a.get('Y') {
            Some(v) => dim(v, ws.rows)?,
            None => (2 * n).saturating_sub(1),
        };
        let rect = clamp(Rect::new(x, y, cols, rows), ws);
        self.new_floating(a, ctx, &f, rect, host)
    }

    /// Start a floating pane at `rect` of `f`'s window.
    fn new_floating(&mut self, a: &Args, ctx: &CmdContext, f: &crate::target::Found, rect: Rect, host: &mut dyn Host) -> R {
        let wid = f.w.ok_or_else(|| CmdError::new("no current window"))?;
        let inner = crate::model::floating_inner(rect);
        let cwd_default = match f.p {
            Some(p) => {
                let info = host.pane_info(p);
                if info.current_path.is_empty() { self.sessions[&f.s].cwd.clone() } else { info.current_path.to_string() }
            }
            None => self.sessions[&f.s].cwd.clone(),
        };
        let cwd = self.cwd_arg(a, Some(f), ctx, &*host, cwd_default);
        let pid = self.alloc_pane()?;
        let command = self.spawn_command(f.s, &a.strings(0));
        let mut spec = self.spawn_spec(pid, wid, f.s, command.clone(), cwd.clone(), inner.size(), ctx.client, false);
        spec.env.extend(crate::environ::flag_env(a));
        host.spawn_pane(&spec).map_err(|e| CmdError(format!("create pane failed: {e}")))?;
        let point = self.tick();
        let title = a.get('T').map(|t| t.chars().filter(|c| !c.is_control()).collect());
        self.panes.insert(pid, Pane { id: pid, window: wid, size: inner.size(), command, cwd, dead: false, exit_status: None, active_point: point, options: OptionSet::default(), title, hooks: Default::default(), input_off: false });
        let w = self.windows.get_mut(&wid).expect("window");
        w.panes.push(pid);
        w.floating.push((pid, rect));
        if !a.has('d') {
            self.set_active_pane(wid, pid, host);
        }
        self.sync_window(wid, host);
        host.redraw(Redraw::Window(wid));
        self.redraw_status_of(wid, host);
        self.print_new(a, crate::exec::NEW_WINDOW, f.s, wid, pid, ctx, &*host)
    }

    /// `split-window` of a floating pane: a new floating pane of the same
    /// size right below it (or as far down as the window allows), as tmux.
    pub(crate) fn split_floating(&mut self, a: &Args, ctx: &CmdContext, f: &crate::target::Found, host: &mut dyn Host) -> R {
        let (wid, target) = (f.w.expect("window"), f.p.expect("pane"));
        let r = self.windows[&wid].floating.iter().find(|x| x.0 == target).map(|x| x.1).expect("floating");
        let ws = self.windows[&wid].size;
        let below = clamp(Rect::new(r.x, r.y.saturating_add(r.rows), r.cols, r.rows), ws);
        self.new_floating(a, ctx, f, below, host)
    }

    /// `move-pane`: with a position flag, move a floating pane; without, it
    /// is `join-pane`.
    pub(crate) fn cmd_move_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if !['D', 'L', 'P', 'R', 'U', 'X', 'Y', 'z'].iter().any(|c| a.has(*c)) {
            return self.cmd_join_pane(a, ctx, host);
        }
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, p) = (f.w.ok_or_else(|| CmdError::new("no current window"))?, f.p.ok_or_else(|| CmdError::new("no current pane"))?);
        let w = &self.windows[&wid];
        let Some(i) = w.floating.iter().position(|x| x.0 == p) else { return Err(CmdError::new("pane is not floating")) };
        let ws = w.size;
        let mut r = w.floating[i].1;
        let amount = || -> Result<u16, CmdError> {
            match a.value(0) {
                Some(v) => v.parse::<u16>().map_err(|_| CmdError(format!("adjustment invalid: {v}"))),
                None => Ok(1),
            }
        };
        let mut order: Option<usize> = None;
        let len = w.floating.len();
        if let Some(pos) = a.get('P') {
            match pos {
                "front" => order = Some(len - 1),
                "back" => order = Some(0),
                "forward" => order = Some((i + 1).min(len - 1)),
                "backward" => order = Some(i.saturating_sub(1)),
                "forward-loop" => order = Some(if i + 1 >= len { 0 } else { i + 1 }),
                "backward-loop" => order = Some(if i == 0 { len - 1 } else { i - 1 }),
                _ => {
                    let (x, y) = position(pos, r.cols, r.rows, ws).ok_or_else(|| CmdError(format!("invalid position: {pos}")))?;
                    r.x = x;
                    r.y = y;
                }
            }
        }
        if let Some(v) = a.get('X') {
            r.x = dim(v, ws.cols)?;
        }
        if let Some(v) = a.get('Y') {
            r.y = dim(v, ws.rows)?;
        }
        if a.has('L') {
            r.x = r.x.saturating_sub(amount()?);
        }
        if a.has('R') {
            r.x = r.x.saturating_add(amount()?);
        }
        if a.has('U') {
            r.y = r.y.saturating_sub(amount()?);
        }
        if a.has('D') {
            r.y = r.y.saturating_add(amount()?);
        }
        if let Some(z) = a.get('z') {
            // 0 is the front.
            let z: usize = z.parse().map_err(|_| CmdError(format!("invalid z-index: {z}")))?;
            order = Some(len - 1 - z.min(len - 1));
        }
        let w = self.windows.get_mut(&wid).expect("window");
        w.floating[i].1 = clamp(r, ws);
        if let Some(to) = order {
            let item = w.floating.remove(i);
            w.floating.insert(to.min(w.floating.len()), item);
        }
        self.sync_window(wid, host);
        host.redraw(Redraw::Window(wid));
        Ok(CmdOutput::default())
    }

    /// `resize-pane` of a floating pane: `-x`/`-y` its size with the
    /// border, `-L`/`-R`/`-U`/`-D` narrower, wider, shorter, taller.
    /// Returns false when `p` is not floating.
    pub(crate) fn resize_floating(&mut self, a: &Args, wid: crate::ids::WindowId, p: crate::ids::PaneId, host: &mut dyn Host) -> Result<bool, CmdError> {
        let w = &self.windows[&wid];
        let Some(i) = w.floating.iter().position(|x| x.0 == p) else { return Ok(false) };
        let ws = w.size;
        let mut r = w.floating[i].1;
        if let Some(v) = a.get('x') {
            r.cols = dim(v, ws.cols)?;
        }
        if let Some(v) = a.get('y') {
            r.rows = dim(v, ws.rows)?;
        }
        let n = match a.value(0) {
            Some(v) => v.parse::<u16>().map_err(|_| CmdError(format!("adjustment invalid: {v}")))?,
            None => 1,
        };
        if a.has('L') {
            r.cols = r.cols.saturating_sub(n);
        }
        if a.has('R') {
            r.cols = r.cols.saturating_add(n);
        }
        if a.has('U') {
            r.rows = r.rows.saturating_sub(n);
        }
        if a.has('D') {
            r.rows = r.rows.saturating_add(n);
        }
        self.windows.get_mut(&wid).expect("window").floating[i].1 = clamp(r, ws);
        self.sync_window(wid, host);
        host.redraw(Redraw::Window(wid));
        Ok(true)
    }

    /// The window is now `ws`: its floating panes move and shrink to fit.
    pub(crate) fn fit_floating(&mut self, wid: crate::ids::WindowId) {
        if let Some(w) = self.windows.get_mut(&wid) {
            let ws = w.size;
            for f in &mut w.floating {
                f.1 = clamp(f.1, ws);
            }
        }
    }
}
