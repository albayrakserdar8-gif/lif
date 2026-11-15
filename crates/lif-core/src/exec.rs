//! Command execution.

use crate::bindings::{Binding, PREFIX_TABLE, ROOT_TABLE};
use crate::cmd::{Arg, Args, CmdError, Command, CommandList, commands_from_args, escape_arg, parse_string};
use crate::format;
use crate::host::{
    CaptureRange, ConfigError, CopyModeOptions, DetachReason, Host, LinePos, PaneInput, Prompt, Redraw,
};
use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use crate::keys::KeyCode;
use crate::layout::{Axis, Direction, Layout, Preset, PresetParams, SizeSpec, SplitSize};
use crate::model::{CmdContext, CmdOutput, Model, PasteBuffer, Session, Window, Pane};
use crate::options::{self, OptionSet, Scope, Value};
use crate::target::{Flags, Found, Kind};
use lif_types::Size;
use std::collections::BTreeMap;

type R = Result<CmdOutput, CmdError>;

/// Most repeats `send-keys -N` accepts (tmux takes any u32: `-N 4294967295`
/// would queue for most of an hour on the server's one thread). Copy-mode
/// commands (`-X`) never need more: no history has that many lines.
const MAX_REPEAT: u32 = 1_000_000;

/// Most input one `send-keys` sends, in bytes, all repeats counted (a key
/// counts as [`KEY_INPUT_BYTES`]). Checked before anything is sent: a
/// command over it sends nothing. Pastes of real text stay far below it.
const MAX_SEND_BYTES: u64 = 1 << 20;

/// What a key counts for against [`MAX_SEND_BYTES`]: about the longest
/// encoding a key gets (kitty keyboard protocol with modifiers and event
/// type, `ESC [ 57441 ; 16 : 3 u`).
const KEY_INPUT_BYTES: u64 = 16;

/// tmux's CMD_SOURCE_FILE_DEPTH_LIMIT: how deeply `source-file` may nest.
const MAX_SOURCE_DEPTH: u32 = 50;

thread_local! {
    /// How many `source-file` commands are running inside each other. Not a
    /// model field because only the call stack it guards cares; the server
    /// runs the model on one thread.
    static SOURCE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Longest message or pane-number display `-d` accepts, in milliseconds:
/// tmux's limit (UINT_MAX, about 49 days). The server adds it to the clock,
/// which a larger value would overflow.
fn parse_delay(d: &str) -> Result<u64, CmdError> {
    match d.parse::<u64>() {
        Ok(ms) if ms <= u32::MAX as u64 => Ok(ms),
        Ok(_) => err(format!("delay too large: {d}")),
        Err(_) if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) => err(format!("delay too large: {d}")),
        Err(_) => err(format!("delay invalid: {d}")),
    }
}

const LIST_SESSIONS: &str = "#{session_name}: #{session_windows} windows (created #{t:session_created})#{?session_grouped, (group ,}#{session_group}#{?session_grouped,),}#{?session_attached, (attached),}";
const LIST_WINDOWS: &str = "#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] [layout #{window_layout}] #{window_id}#{?window_active, (active),}";
const LIST_WINDOWS_ALL: &str = "#{session_name}:#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] ";
const LIST_PANES: &str = "#{pane_index}: [#{pane_width}x#{pane_height}] [history #{history_size}/#{history_limit}, #{history_bytes} bytes] #{pane_id}#{?pane_active, (active),}#{?pane_dead, (dead),}";
const LIST_CLIENTS: &str = "#{client_name}: #{session_name} [#{client_width}x#{client_height} #{client_termname}]#{?client_flags, (,}#{client_flags}#{?client_flags,),}";
const LIST_BUFFERS: &str = "#{buffer_name}: #{buffer_size} bytes: \"#{buffer_sample}\"";
const DISPLAY_MESSAGE: &str = "[#{session_name}] #{window_index}:#{window_name}, current pane #{pane_index} - (%H:%M %d-%b-%y)";
const NEW_WINDOW: &str = "#{session_name}:#{window_index}.#{pane_index}";
const NEW_SESSION: &str = "#{session_name}:";

/// `list-commands -F`: tmux's `command_list_*` variables over the usual ones.
struct CommandVars<'a> {
    entry: &'a crate::cmd::CmdEntry,
    inner: &'a dyn format::Vars,
}

impl format::Vars for CommandVars<'_> {
    fn var(&self, name: &str) -> Option<String> {
        match name {
            "command_list_name" => Some(self.entry.name.to_string()),
            "command_list_alias" => Some(self.entry.alias.unwrap_or("").to_string()),
            "command_list_usage" => Some(usage(self.entry)),
            _ => self.inner.var(name),
        }
    }
    fn now(&self) -> crate::WallClock {
        self.inner.now()
    }
}

/// A usage line built from a command's flag template: `[-AdD] [-c value]
/// [argument ...]` (tmux's hand-written lines name each value; lif's
/// table does not have the names).
fn usage(e: &crate::cmd::CmdEntry) -> String {
    let t: Vec<char> = e.template.chars().collect();
    let (mut flags, mut values) = (String::new(), Vec::new());
    let mut i = 0;
    while i < t.len() {
        if t.get(i + 1) == Some(&':') {
            values.push(format!("[-{} value]", t[i]));
            i += 2;
        } else {
            flags.push(t[i]);
            i += 1;
        }
    }
    let mut parts = Vec::new();
    if !flags.is_empty() {
        parts.push(format!("[-{flags}]"));
    }
    parts.extend(values);
    match (e.lower, e.upper) {
        (_, Some(0)) => {}
        (0, Some(1)) => parts.push("[argument]".into()),
        (0, _) => parts.push("[argument ...]".into()),
        _ => parts.push("argument ...".into()),
    }
    parts.join(" ")
}

fn ok() -> R {
    Ok(CmdOutput::default())
}

fn text(s: String) -> R {
    Ok(CmdOutput { stdout: s })
}

fn err<T>(s: impl Into<String>) -> Result<T, CmdError> {
    Err(CmdError(s.into()))
}

fn need_window(f: &Found) -> Result<WindowId, CmdError> {
    f.w.ok_or_else(|| CmdError::new("no current window"))
}

fn need_pane(f: &Found) -> Result<PaneId, CmdError> {
    f.p.ok_or_else(|| CmdError::new("no current pane"))
}

/// Substitute prompt answers into a `command-prompt` template as tmux does:
/// `%1`..`%9` are the answers by position, each `%%` takes the next answer
/// in turn, and a tripled `%%%` (or `%1%`-style) inserts the answer with
/// `"\$;~` escaped.
pub fn expand_prompt_template(template: &str, answers: &[&str]) -> String {
    let mut t = template.to_string();
    for (i, ans) in answers.iter().enumerate() {
        let idx = (i + 1) as u8;
        let b = t.as_bytes();
        let mut out = String::with_capacity(t.len() + ans.len());
        let mut replaced = false;
        let mut k = 0;
        while k < b.len() {
            if b[k] == b'%' && k + 1 < b.len() {
                let n = b[k + 1];
                let hit = if n.is_ascii_digit() && n - b'0' == idx {
                    true
                } else if n == b'%' && !replaced {
                    replaced = true;
                    true
                } else {
                    false
                };
                if hit {
                    // Answers pasted into prompts and templates with many
                    // slots multiply: past the limit nothing runs (a cut
                    // command could do something else entirely).
                    if out.len() + 2 * ans.len() > MAX_TEMPLATE_OUTPUT {
                        return String::new();
                    }
                    k += 2;
                    let quoted = k < b.len() && b[k] == b'%';
                    if quoted {
                        k += 1;
                        for c in ans.chars() {
                            if "\"\\$;~".contains(c) {
                                out.push('\\');
                            }
                            out.push(c);
                        }
                    } else {
                        out.push_str(ans);
                    }
                    continue;
                }
            }
            let ch = t[k..].chars().next().expect("in bounds");
            out.push(ch);
            k += ch.len_utf8();
        }
        t = out;
    }
    t
}

/// Most output a list command (`list-windows -F ...`) builds: each
/// item's format is bounded (`format::MAX_BYTES`), their number is not.
const MAX_LIST_OUTPUT: usize = 64 * 1024 * 1024;

fn check_list_output(out: &str) -> Result<(), CmdError> {
    if out.len() > MAX_LIST_OUTPUT {
        return err(format!("output too large (more than {MAX_LIST_OUTPUT} bytes)"));
    }
    Ok(())
}

/// Longest command [`expand_prompt_template`] builds.
const MAX_TEMPLATE_OUTPUT: usize = 1024 * 1024;

fn parse_size_arg(s: &str, of: u32, what: &str) -> Result<u32, CmdError> {
    match SizeSpec::parse(s) {
        Some(SizeSpec::Cells(n)) => Ok(n),
        Some(SizeSpec::Percent(p)) => Ok((of as u64 * p as u64 / 100) as u32),
        None => err(format!("{what} invalid")),
    }
}

impl Model {
    /// Run one command. `ctx` says who runs it (default targets come from
    /// the client's current session / window / pane); effects on the outside
    /// world go through `host`. On error nothing half-done is left behind.
    pub fn execute(&mut self, cmd: &Command, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let a = cmd.args();
        match cmd.name() {
            "attach-session" => self.cmd_attach(a, ctx, host),
            "bind-key" => self.cmd_bind(a),
            "break-pane" => self.cmd_break_pane(a, ctx, host),
            "capture-pane" => self.cmd_capture(a, ctx, host),
            "command-prompt" => self.cmd_command_prompt(a, ctx, host),
            "confirm-before" => self.cmd_confirm(a, ctx, host),
            "copy-mode" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
                let opts = CopyModeOptions {
                    page_up: a.has('u'),
                    page_down: a.has('d'),
                    exit_at_bottom: a.has('e'),
                    hide_position: a.has('H'),
                    mouse_drag: a.has('M'),
                    cancel: a.has('q'),
                };
                host.enter_copy_mode(ctx.client, need_pane(&f)?, opts);
                ok()
            }
            "delete-buffer" => {
                let i = self.buffer_index(a.get('b'))?;
                self.buffers.remove(i);
                ok()
            }
            "detach-client" => self.cmd_detach(a, ctx, host),
            "display-message" => self.cmd_display(a, ctx, host),
            "display-panes" => {
                let cid = self.attached_client(ctx, a.get('t'))?;
                let sid = self.clients[&cid].session;
                let cw = self.sessions.get(&sid.expect("attached")).map(|s| s.current);
                let ms = match a.get('d') {
                    Some(d) => parse_delay(d)?,
                    None => self.wopt(cw, "display-panes-time").as_num() as u64,
                };
                let template = a.strings(0).into_iter().next().unwrap_or_else(|| "select-pane -t '%%'".to_string());
                host.display_panes(cid, ms, &template, !a.has('N'));
                ok()
            }
            "suspend-client" => {
                let cid = self.attached_client(ctx, a.get('t'))?;
                host.suspend_client(cid).map(|()| CmdOutput::default()).map_err(CmdError)
            }
            "has-session" => self.find_target(ctx, a.get('t'), Kind::Session, Flags::default()).map(|_| CmdOutput::default()),
            "kill-pane" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
                let p = need_pane(&f)?;
                if a.has('a') {
                    let w = need_window(&f)?;
                    let others: Vec<PaneId> = self.windows[&w].panes.iter().copied().filter(|x| *x != p).collect();
                    for o in others {
                        self.remove_pane(o, true, host);
                    }
                } else {
                    self.remove_pane(p, true, host);
                }
                ok()
            }
            "kill-server" => {
                self.shutdown(host);
                ok()
            }
            "kill-session" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
                if a.has('C') {
                    let wids: Vec<WindowId> = self.sessions[&f.s].windows.values().copied().collect();
                    for w in wids {
                        if let Some(w) = self.windows.get_mut(&w) {
                            w.bell = false;
                            w.activity = false;
                        }
                    }
                    host.redraw(Redraw::Status(f.s));
                } else if a.has('a') {
                    let others: Vec<SessionId> = self.sessions.keys().copied().filter(|s| *s != f.s).collect();
                    for s in others {
                        self.destroy_session(s, host);
                    }
                } else {
                    self.destroy_session(f.s, host);
                }
                ok()
            }
            "kill-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                let w = need_window(&f)?;
                if a.has('a') {
                    let others: Vec<WindowId> = self.sessions[&f.s].windows.values().copied().filter(|x| *x != w).collect();
                    for o in others {
                        self.remove_window(o, None, host);
                    }
                } else {
                    self.remove_window(w, None, host);
                }
                ok()
            }
            "last-pane" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                let w = need_window(&f)?;
                let last = self.last_pane_of(w).ok_or_else(|| CmdError::new("no last pane"))?;
                self.select_pane_zoom(w, last, a.has('Z'), host);
                ok()
            }
            "last-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
                let last = self.sessions[&f.s].last.first().copied().ok_or_else(|| CmdError::new("no last window"))?;
                self.select_window(f.s, last, host);
                ok()
            }
            "list-buffers" => {
                let mut out = String::new();
                for b in &self.buffers {
                    let mut c = self.fctx(&*host, None, None, None, ctx.client, ctx.now);
                    c.buffer = Some(b);
                    if let Some(filter) = a.get('f')
                        && !format::is_true(&format::expand(filter, &c))
                    {
                        continue;
                    }
                    out.push_str(&format::expand(a.get('F').unwrap_or(LIST_BUFFERS), &c));
                    check_list_output(&out)?;
                    out.push('\n');
                }
                text(out)
            }
            "list-clients" => {
                let only = match a.get('t') {
                    Some(t) => Some(self.find_target(ctx, Some(t), Kind::Session, Flags::default())?.s),
                    None => None,
                };
                let mut out = String::new();
                for c in self.clients.values() {
                    let Some(sid) = c.session else { continue };
                    if only.is_some_and(|o| o != sid) {
                        continue;
                    }
                    let cw = self.sessions.get(&sid).map(|s| s.current);
                    let cp = cw.and_then(|w| self.windows.get(&w)).map(|w| w.active);
                    let fc = self.fctx(&*host, Some(sid), cw, cp, Some(c.id), ctx.now);
                    if let Some(filter) = a.get('f')
                        && !format::is_true(&format::expand(filter, &fc))
                    {
                        continue;
                    }
                    out.push_str(&format::expand(a.get('F').unwrap_or(LIST_CLIENTS), &fc));
                    check_list_output(&out)?;
                    out.push('\n');
                }
                text(out)
            }
            "list-commands" => {
                let mut out = String::new();
                for e in crate::cmd::supported_entries() {
                    if let Some(name) = a.value(0)
                        && name != e.name
                        && e.alias != Some(name)
                    {
                        continue;
                    }
                    if let Some(fmt) = a.get('F') {
                        let inner = self.fctx(&*host, None, None, None, ctx.client, ctx.now);
                        out.push_str(&format::expand(fmt, &CommandVars { entry: e, inner: &inner }));
                        check_list_output(&out)?;
                        out.push('\n');
                        continue;
                    }
                    match e.alias {
                        Some(al) => out.push_str(&format!("{} ({al})\n", e.name)),
                        None => out.push_str(&format!("{}\n", e.name)),
                    }
                }
                text(out)
            }
            "list-keys" => self.cmd_list_keys(a),
            "list-panes" => self.cmd_list_panes(a, ctx, host),
            "list-sessions" => {
                let mut out = String::new();
                for sid in self.sessions_by_name() {
                    let s = &self.sessions[&sid];
                    let p = self.windows.get(&s.current).map(|w| w.active);
                    let c = self.fctx(&*host, Some(sid), Some(s.current), p, ctx.client, ctx.now);
                    if let Some(filter) = a.get('f')
                        && !format::is_true(&format::expand(filter, &c))
                    {
                        continue;
                    }
                    out.push_str(&format::expand(a.get('F').unwrap_or(LIST_SESSIONS), &c));
                    check_list_output(&out)?;
                    out.push('\n');
                }
                text(out)
            }
            "list-windows" => self.cmd_list_windows(a, ctx, host),
            "move-window" => self.cmd_move_window(a, ctx, host),
            "new-session" => self.cmd_new_session(a, ctx, host),
            "new-window" => self.cmd_new_window(a, ctx, host),
            "next-layout" | "previous-layout" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                let w = need_window(&f)?;
                let next = match self.windows[&w].last_preset {
                    None => Preset::ALL[0],
                    Some(p) if cmd.name() == "next-layout" => p.next(),
                    Some(p) => p.previous(),
                };
                self.apply_preset(w, next, host);
                ok()
            }
            "next-window" | "previous-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
                self.step_window(f.s, cmd.name() == "next-window", a.has('a'), host)
            }
            "paste-buffer" => self.cmd_paste(a, ctx, host),
            "refresh-client" => {
                let cid = self.attached_client(ctx, a.get('t'))?;
                if a.has('S') {
                    host.redraw(Redraw::Status(self.clients[&cid].session.expect("attached")));
                } else {
                    host.redraw(Redraw::Client(cid));
                }
                ok()
            }
            "rename-session" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
                let name = self.expand_found(&f, ctx, &*host, a.value(0).unwrap_or(""));
                if name.is_empty() {
                    return err(format!("invalid session: {name}"));
                }
                if let Some(other) = self.session_by_name(&name)
                    && other != f.s
                {
                    return err(format!("duplicate session: {name}"));
                }
                self.sessions.get_mut(&f.s).expect("session").name = name;
                host.redraw(Redraw::Status(f.s));
                ok()
            }
            "rename-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                let w = need_window(&f)?;
                let name = self.expand_found(&f, ctx, &*host, a.value(0).unwrap_or(""));
                let win = self.windows.get_mut(&w).expect("window");
                win.name = name;
                win.options.set("automatic-rename", Value::Flag(false));
                host.redraw(Redraw::Status(f.s));
                ok()
            }
            "resize-pane" => self.cmd_resize_pane(a, ctx, host),
            "respawn-pane" => self.cmd_respawn(a, ctx, host),
            "rotate-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                let w = need_window(&f)?;
                self.rotate(w, a.has('D'), a.has('Z'), host);
                ok()
            }
            "select-layout" => self.cmd_select_layout(a, ctx, host),
            "select-pane" => self.cmd_select_pane(a, ctx, host),
            "select-window" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                if a.has('l') {
                    let last = self.sessions[&f.s].last.first().copied().ok_or_else(|| CmdError::new("no last window"))?;
                    self.select_window(f.s, last, host);
                    return ok();
                }
                if a.has('n') || a.has('p') {
                    return self.step_window(f.s, a.has('n'), false, host);
                }
                let w = need_window(&f)?;
                if a.has('T') && self.sessions[&f.s].current == w {
                    let last = self.sessions[&f.s].last.first().copied().ok_or_else(|| CmdError::new("no last window"))?;
                    self.select_window(f.s, last, host);
                } else {
                    self.select_window(f.s, w, host);
                }
                ok()
            }
            "send-keys" => self.cmd_send_keys(a, ctx, host),
            "send-prefix" => {
                let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
                let p = need_pane(&f)?;
                let name = if a.has('2') { "prefix2" } else { "prefix" };
                if let Value::Key(Some(k)) = self.sopt(Some(f.s), name) {
                    let ev = k.to_event();
                    host.send_to_pane(p, PaneInput::Key(&ev)).map_err(CmdError)?;
                }
                ok()
            }
            "set-buffer" => self.cmd_set_buffer(a),
            "set-option" => self.cmd_set_option(a, a.has('w'), ctx, host),
            "set-window-option" => self.cmd_set_option(a, true, ctx, host),
            "show-buffer" => {
                let i = self.buffer_index(a.get('b'))?;
                let mut s = String::from_utf8_lossy(&self.buffers[i].data).into_owned();
                if !s.ends_with('\n') {
                    s.push('\n');
                }
                text(s)
            }
            "show-messages" => {
                let mut out = String::new();
                for m in &self.messages {
                    out.push_str(m);
                    out.push('\n');
                }
                text(out)
            }
            "show-options" => self.cmd_show_options(a, a.has('w'), ctx),
            "show-window-options" => self.cmd_show_options(a, true, ctx),
            "source-file" => self.cmd_source_file(a, ctx, host),
            "split-window" => self.cmd_split(a, ctx, host),
            "start-server" => ok(),
            "swap-pane" => self.cmd_swap_pane(a, ctx, host),
            "swap-window" => self.cmd_swap_window(a, ctx, host),
            "switch-client" => self.cmd_switch(a, ctx, host),
            "unbind-key" => {
                let table = if a.has('n') { ROOT_TABLE } else { a.get('T').unwrap_or(PREFIX_TABLE) };
                if a.has('a') {
                    if a.value(0).is_some() {
                        return err("key given with -a");
                    }
                    self.bindings.unbind_all(table);
                    return ok();
                }
                let Some(name) = a.value(0) else { return err("missing key") };
                match KeyCode::parse(name) {
                    Some(k) => {
                        self.bindings.unbind(table, &k);
                        ok()
                    }
                    None if a.has('q') => ok(),
                    None => err(format!("unknown key: {name}")),
                }
            }
            other => err(format!("unsupported command: {other}")),
        }
    }

    /// Run a list of commands in order, stopping at the first error. The
    /// output of the commands that ran is concatenated.
    pub fn execute_list(&mut self, list: &CommandList, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let mut out = CmdOutput::default();
        for c in list.commands() {
            let o = self.execute(c, ctx, host)?;
            out.stdout.push_str(&o.stdout);
        }
        Ok(out)
    }

    /// Store a paste buffer (copy mode uses this). With no name it gets an
    /// automatic one (`buffer0000`...) and the oldest automatic buffers
    /// beyond `buffer-limit` are dropped.
    pub fn set_buffer(&mut self, name: Option<&str>, data: Vec<u8>) {
        match name {
            Some(n) => {
                self.buffers.retain(|b| b.name != n);
                self.buffers.insert(0, PasteBuffer { name: n.to_string(), data, automatic: false });
            }
            None => {
                // The counter wraps after 2^32 copies; skip names still in
                // use rather than replace a buffer.
                let name = loop {
                    let n = format!("buffer{:04}", self.next_buffer);
                    self.next_buffer = self.next_buffer.wrapping_add(1);
                    if !self.buffers.iter().any(|b| b.name == n) {
                        break n;
                    }
                };
                self.buffers.insert(0, PasteBuffer { name, data, automatic: true });
                let limit = self.gopt("buffer-limit").as_num().max(1) as usize;
                let mut auto = 0;
                self.buffers.retain(|b| {
                    if !b.automatic {
                        return true;
                    }
                    auto += 1;
                    auto <= limit
                });
            }
        }
    }

    // ----- helpers -----------------------------------------------------

    fn expand_found(&self, f: &Found, ctx: &CmdContext, host: &dyn Host, fmt: &str) -> String {
        format::expand(fmt, &self.fctx(host, Some(f.s), f.w, f.p, ctx.client, ctx.now))
    }

    #[allow(clippy::too_many_arguments)]
    fn print_new(&self, a: &Args, default: &str, s: SessionId, w: WindowId, p: PaneId, ctx: &CmdContext, host: &dyn Host) -> R {
        if !a.has('P') {
            return ok();
        }
        let fmt = a.get('F').unwrap_or(default);
        let mut out = format::expand(fmt, &self.fctx(host, Some(s), Some(w), Some(p), ctx.client, ctx.now));
        out.push('\n');
        text(out)
    }

    fn find_client(&self, name: &str) -> Option<ClientId> {
        self.clients.values().find(|c| c.info.name == name || c.id.to_string() == name).map(|c| c.id)
    }

    /// The client a command acts on: `-t/-c name` or the invoking client.
    fn target_client(&self, ctx: &CmdContext, name: Option<&str>) -> Result<ClientId, CmdError> {
        match name {
            Some(n) => self.find_client(n).ok_or_else(|| CmdError(format!("can't find client: {n}"))),
            None => ctx.client.filter(|c| self.clients.contains_key(c)).ok_or_else(|| CmdError::new("no current client")),
        }
    }

    fn attached_client(&self, ctx: &CmdContext, name: Option<&str>) -> Result<ClientId, CmdError> {
        let c = self.target_client(ctx, name)?;
        if self.clients[&c].session.is_none() {
            return err("no current client");
        }
        Ok(c)
    }

    fn buffer_index(&self, name: Option<&str>) -> Result<usize, CmdError> {
        match name {
            Some(n) => self.buffers.iter().position(|b| b.name == n).ok_or_else(|| CmdError(format!("no buffer {n}"))),
            None if self.buffers.is_empty() => err("no buffer"),
            None => Ok(0),
        }
    }

    fn last_pane_of(&self, w: WindowId) -> Option<PaneId> {
        let win = &self.windows[&w];
        win.last_panes.first().copied().or_else(|| {
            if win.panes.len() == 2 { win.panes.iter().copied().find(|p| *p != win.active) } else { None }
        })
    }

    /// Make `p` active; unzoom first, re-zoom on `p` when `keep_zoom`.
    fn select_pane_zoom(&mut self, w: WindowId, p: PaneId, keep_zoom: bool, host: &mut dyn Host) {
        let win = self.windows.get_mut(&w).expect("window");
        let was_zoomed = win.layout.unzoom();
        self.set_active_pane(w, p, host);
        if was_zoomed {
            if keep_zoom {
                self.windows.get_mut(&w).expect("window").layout.zoom(p);
            }
            self.sync_window(w, host);
        }
    }

    fn unzoom(&mut self, w: WindowId, host: &mut dyn Host) {
        if self.windows.get_mut(&w).is_some_and(|win| win.layout.unzoom()) {
            self.sync_window(w, host);
        }
    }

    fn step_window(&mut self, sid: SessionId, forward: bool, alert: bool, host: &mut dyn Host) -> R {
        let s = &self.sessions[&sid];
        let keys: Vec<(u32, WindowId)> = s.windows.iter().map(|(i, w)| (*i, *w)).collect();
        let pos = keys.iter().position(|(_, w)| *w == s.current).unwrap_or(0);
        let n = keys.len();
        let msg = if forward { "no next window" } else { "no previous window" };
        for step in 1..n {
            let i = if forward { (pos + step) % n } else { (pos + n - step) % n };
            let wid = keys[i].1;
            if alert && !self.windows.get(&wid).is_some_and(|w| w.bell || w.activity) {
                continue;
            }
            self.select_window(sid, wid, host);
            return ok();
        }
        err(msg)
    }

    fn preset_params(&self, w: WindowId) -> PresetParams {
        let sz = |name: &str, d: SizeSpec| SizeSpec::parse(self.wopt(Some(w), name).as_str()).unwrap_or(d);
        PresetParams {
            main_pane_width: sz("main-pane-width", SizeSpec::Cells(80)),
            main_pane_height: sz("main-pane-height", SizeSpec::Cells(24)),
            other_pane_width: sz("other-pane-width", SizeSpec::Cells(0)),
            other_pane_height: sz("other-pane-height", SizeSpec::Cells(0)),
            tiled_max_columns: self.wopt(Some(w), "tiled-layout-max-columns").as_num() as u32,
        }
    }

    fn apply_preset(&mut self, w: WindowId, preset: Preset, host: &mut dyn Host) {
        let params = self.preset_params(w);
        let win = self.windows.get_mut(&w).expect("window");
        win.old_layout = Some(win.layout.clone());
        let panes = win.panes.clone();
        let size = win.size;
        win.layout.apply_preset(preset, &panes, size, &params);
        win.last_preset = Some(preset);
        self.sync_window(w, host);
    }

    fn rotate(&mut self, w: WindowId, down: bool, keep_zoom: bool, host: &mut dyn Host) {
        let win = self.windows.get_mut(&w).expect("window");
        if win.panes.len() < 2 {
            return;
        }
        let was_zoomed = win.layout.unzoom();
        let old = win.panes.clone();
        let mut new = old.clone();
        if down {
            new.rotate_right(1);
        } else {
            new.rotate_left(1);
        }
        win.layout.reassign(&old, &new);
        let pos = old.iter().position(|p| *p == win.active).unwrap_or(0);
        let active = new[pos];
        win.panes = new;
        self.set_active_pane(w, active, host);
        if was_zoomed && keep_zoom {
            self.windows.get_mut(&w).expect("window").layout.zoom(active);
        }
        self.sync_window(w, host);
    }

    /// tmux's `winlink_shuffle_up`: make room at `idx` by moving the windows
    /// from `idx` up to the next gap one index up. Fails, changing nothing,
    /// when no gap is left at or below the maximum index.
    fn shuffle_up(&mut self, sid: SessionId, idx: u32) -> Result<(), CmdError> {
        let mut last = self.shuffle_gap(sid, idx)?;
        let s = self.sessions.get_mut(&sid).expect("session");
        while last > idx {
            if let Some(w) = s.windows.remove(&(last - 1)) {
                s.windows.insert(last, w);
                if let Some(win) = self.windows.get_mut(&w) {
                    win.index = last;
                }
            }
            last -= 1;
        }
        Ok(())
    }

    /// The first free index at or after `idx` that `shuffle_up` would move
    /// windows into.
    fn shuffle_gap(&self, sid: SessionId, idx: u32) -> Result<u32, CmdError> {
        let s = &self.sessions[&sid];
        (idx..=crate::model::MAX_WINDOW_INDEX).find(|i| !s.windows.contains_key(i)).ok_or_else(|| CmdError::new("no free window index"))
    }

    /// An index a target named, refused above the maximum: tmux's targets
    /// stop at INT_MAX, and indices above it leave no room for `-a`.
    fn check_index(i: u32) -> Result<u32, CmdError> {
        if i > crate::model::MAX_WINDOW_INDEX {
            return Err(Self::too_big(i));
        }
        Ok(i)
    }

    fn too_big(i: u32) -> CmdError {
        CmdError(format!("window index too big: {i} (the largest is {})", crate::model::MAX_WINDOW_INDEX))
    }

    /// The index `-a` / `-b` insert at (after shuffling), or `None` without
    /// those flags. Checks that the shuffle has room, so a caller can refuse
    /// before it starts a process or takes a pane out of its window.
    fn insert_index(&self, f: &Found, a: &Args) -> Result<Option<u32>, CmdError> {
        if !a.has('a') && !a.has('b') {
            return Ok(None);
        }
        let base = match f.idx {
            Some(i) => Self::check_index(i)?,
            None => self.windows[&self.sessions[&f.s].current].index,
        };
        let idx = if a.has('a') {
            match base.checked_add(1) {
                Some(i) if i <= crate::model::MAX_WINDOW_INDEX => i,
                _ => return err("no free window index"),
            }
        } else {
            base
        };
        self.shuffle_gap(f.s, idx)?;
        Ok(Some(idx))
    }

    /// Everything [`Model::place_index`] can refuse for lack of an index,
    /// without changing anything.
    fn check_placement(&self, f: &Found, a: &Args) -> Result<(), CmdError> {
        if self.insert_index(f, a)?.is_some() {
            return Ok(());
        }
        match f.idx {
            Some(i) => Self::check_index(i).map(|_| ()),
            None => self.first_free_index(f.s).map(|_| ()),
        }
    }

    /// Resolve the index for a new or moved window (`-t`, `-a`, `-b`, `-k`)
    /// and make room there (`-a` / `-b` shift windows up, `-k` kills the
    /// window in the way). The second value is a window that `-k` displaced
    /// but that could not be killed yet because it is the session's only
    /// window; pass it to [`Model::replace_killed`] after linking the new
    /// window. On error nothing has changed.
    fn place_index(&mut self, f: &Found, a: &Args, moving: Option<WindowId>, what: &str, host: &mut dyn Host) -> Result<(u32, Option<WindowId>), CmdError> {
        let sid = f.s;
        self.check_placement(f, a).map_err(|e| CmdError(format!("{what}{}", e.0)))?;
        if let Some(idx) = self.insert_index(f, a)? {
            self.shuffle_up(sid, idx)?;
            return Ok((idx, None));
        }
        match f.idx {
            Some(i) => {
                if let Some(&existing) = self.sessions[&sid].windows.get(&i) {
                    if Some(existing) == moving {
                        return err(format!("same index: {i}"));
                    }
                    if !a.has('k') {
                        return err(if what.is_empty() { format!("index in use: {i}") } else { format!("{what}index {i} in use") });
                    }
                    if self.sessions[&sid].windows.len() == 1 && moving.is_none_or(|m| self.windows[&m].session != sid) {
                        // Killing the only window now would destroy the
                        // session the window goes into; replace it instead.
                        return Ok((i, Some(existing)));
                    }
                    self.remove_window_inner(existing, None, false, host);
                }
                Ok((i, None))
            }
            None => Ok((self.first_free_index(sid)?, None)),
        }
    }

    /// Finish a `-k` replacement of a session's only window by `new`.
    fn replace_killed(&mut self, old: Option<WindowId>, new: WindowId, host: &mut dyn Host) {
        let Some(old) = old else { return };
        let Some(w) = self.windows.remove(&old) else { return };
        for p in &w.panes {
            host.kill_pane(*p);
            self.panes.remove(p);
        }
        if let Some(s) = self.sessions.get_mut(&w.session) {
            s.last.retain(|x| *x != old);
            if s.current == old {
                s.current = new;
            }
        }
    }

    /// Link `wid` at `idx`, which [`Model::place_index`] made free (or
    /// holds the `-k` window that [`Model::replace_killed`] drops next).
    fn insert_window(&mut self, sid: SessionId, idx: u32, wid: WindowId) {
        let s = self.sessions.get_mut(&sid).expect("session");
        s.windows.insert(idx, wid);
        let w = self.windows.get_mut(&wid).expect("window");
        w.index = idx;
        w.session = sid;
    }

    /// Default directory for a new window / session: the client's when it is
    /// not attached, else the session's.
    fn default_cwd(&self, ctx: &CmdContext, sid: Option<SessionId>) -> String {
        if let Some(c) = ctx.client.and_then(|c| self.clients.get(&c))
            && c.session.is_none()
            && !c.info.cwd.is_empty()
        {
            return c.info.cwd.clone();
        }
        sid.and_then(|s| self.sessions.get(&s)).map(|s| s.cwd.clone()).unwrap_or_default()
    }

    fn cwd_arg(&self, a: &Args, f: Option<&Found>, ctx: &CmdContext, host: &dyn Host, default: String) -> String {
        match a.get('c') {
            Some(c) => {
                let c = match f {
                    Some(f) => self.expand_found(f, ctx, host, c),
                    None => format::expand(c, &self.fctx(host, None, None, None, ctx.client, ctx.now)),
                };
                if c.starts_with('/') || default.is_empty() || c.starts_with('~') || c.is_empty() {
                    c
                } else {
                    format!("{}/{}", default.trim_end_matches('/'), c)
                }
            }
            None => default,
        }
    }

    /// Create a window holding one new pane and start its process. On
    /// failure nothing remains. The window is not linked into the session.
    #[allow(clippy::too_many_arguments)]
    fn spawn_window(&mut self, sid: SessionId, name: Option<String>, args: &[String], cwd: String, size: Size, client: Option<ClientId>, host: &mut dyn Host) -> Result<(WindowId, PaneId), CmdError> {
        let failed = |e: CmdError| CmdError(format!("create window failed: {}", e.0));
        let pid = self.alloc_pane().map_err(failed)?;
        let wid = self.alloc_window().map_err(failed)?;
        let command = self.spawn_command(sid, args);
        let mut options = OptionSet::default();
        let auto_name = name.is_none();
        if !auto_name {
            options.set("automatic-rename", Value::Flag(false));
        }
        let wname = name.unwrap_or_else(|| command.program_name());
        let layout = Layout::new(pid, size);
        let psize = layout.size();
        let point = self.tick();
        self.windows.insert(
            wid,
            Window {
                id: wid,
                session: sid,
                index: 0,
                name: wname,
                layout,
                panes: vec![pid],
                active: pid,
                last_panes: Vec::new(),
                bell: false,
                activity: false,
                options,
                last_preset: None,
                old_layout: None,
                size,
            },
        );
        self.panes.insert(
            pid,
            Pane { id: pid, window: wid, size: psize, command: command.clone(), cwd: cwd.clone(), dead: false, exit_status: None, active_point: point },
        );
        let spec = self.spawn_spec(pid, wid, sid, command, cwd, psize, client, false);
        if let Err(e) = host.spawn_pane(&spec) {
            self.windows.remove(&wid);
            self.panes.remove(&pid);
            return err(format!("create window failed: {e}"));
        }
        Ok((wid, pid))
    }

    // ----- sessions and clients ----------------------------------------

    fn cmd_new_session(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if a.has('E') {
            // A new session's panes, and an attach, always take the client's
            // environment: there is no update-environment to skip.
            return err("new-session -E is not supported: lif has no update-environment option");
        }
        let client = ctx.client.and_then(|c| self.clients.get(&c)).map(|c| (c.id, c.info.clone(), c.session));
        if a.has('A')
            && let Some(name) = a.get('s')
            && let Some(sid) = self.session_by_name(name)
        {
            let cid = client.as_ref().map(|c| c.0).ok_or_else(|| CmdError::new("no current client"))?;
            // -D and -X are attach-session's -d and -x.
            if a.has('D') || a.has('X') {
                self.kick_others(sid, cid, a.has('X'), host);
            }
            self.switch_client(cid, sid, host);
            return ok();
        }
        let detached = a.has('d') || client.is_none();
        if !detached
            && let Some((_, info, _)) = &client
            && !info.terminal
        {
            return err("open terminal failed: not a terminal");
        }
        let name = match a.get('s') {
            Some(n) => {
                let n = format::expand(n, &self.fctx(&*host, None, None, None, ctx.client, ctx.now));
                if n.is_empty() {
                    return err(format!("invalid session: {n}"));
                }
                if self.session_by_name(&n).is_some() {
                    return err(format!("duplicate session: {n}"));
                }
                Some(n)
            }
            None => None,
        };
        let cwd_default = match &client {
            Some((_, info, None)) => info.cwd.clone(),
            Some((_, _, Some(s))) => self.sessions.get(s).map(|s| s.cwd.clone()).unwrap_or_default(),
            None => String::new(),
        };
        let cwd = self.cwd_arg(a, None, ctx, &*host, cwd_default);
        let (dw, dh) = options::parse_dim(self.sopt(None, "default-size").as_str()).unwrap_or((80, 24));
        let parse_dim = |v: Option<&str>, d: u16, what: &str| -> Result<u16, CmdError> {
            match v {
                None | Some("-") => Ok(d),
                Some(s) => s
                    .parse::<u16>()
                    .ok()
                    .filter(|n| *n > 0 && *n <= crate::MAX_WINDOW_DIM)
                    .ok_or_else(|| CmdError(format!("{what} invalid (1 to {})", crate::MAX_WINDOW_DIM))),
            }
        };
        let mut size = Size::new(parse_dim(a.get('x'), dw, "width")?, parse_dim(a.get('y'), dh, "height")?);
        if !crate::model::size_allowed(size.cols, size.rows) {
            return err(format!("size too large: {}x{} (at most {} cells)", size.cols, size.rows, crate::MAX_WINDOW_CELLS));
        }
        if !detached && let Some((_, info, _)) = &client {
            let sl = self.sopt(None, "status").as_num() as u16;
            size = Size::new(info.size.cols.max(1), info.size.rows.saturating_sub(sl).max(1));
        }
        // tmux names unnamed sessions after their id, skipping taken names.
        let mut id = self.alloc_session()?;
        let name = match name {
            Some(n) => n,
            None => loop {
                let n = id.0.to_string();
                if self.session_by_name(&n).is_none() {
                    break n;
                }
                id = self.alloc_session()?;
            },
        };
        let tick = self.tick();
        self.sessions.insert(
            id,
            Session {
                id,
                name,
                windows: BTreeMap::new(),
                current: WindowId(u32::MAX),
                last: Vec::new(),
                created: ctx.now.unix_secs,
                activity: tick,
                cwd: cwd.clone(),
                options: OptionSet::default(),
                size,
            },
        );
        let wname = a.get('n').map(|n| format::expand(n, &self.fctx(&*host, Some(id), None, None, ctx.client, ctx.now)));
        let args = a.strings(0);
        let (wid, pid) = match self.spawn_window(id, wname, &args, cwd, size, ctx.client, host) {
            Ok(x) => x,
            Err(e) => {
                self.sessions.remove(&id);
                return Err(e);
            }
        };
        let base = self.base_index(id);
        self.insert_window(id, base, wid);
        self.sessions.get_mut(&id).expect("session").current = wid;
        if !detached && let Some((cid, _, _)) = client {
            self.switch_client(cid, id, host);
        }
        self.print_new(a, NEW_SESSION, id, wid, pid, ctx, &*host)
    }

    fn cmd_attach(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if self.sessions.is_empty() {
            return err("no sessions");
        }
        let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags { prefer_unattached: true, ..Flags::default() })?;
        let cid = self.target_client(ctx, None)?;
        if !self.clients[&cid].info.terminal {
            return err("open terminal failed: not a terminal");
        }
        if let Some(dir) = a.get('c') {
            let d = self.expand_found(&f, ctx, &*host, dir);
            self.sessions.get_mut(&f.s).expect("session").cwd = d;
        }
        if let Some(w) = f.w {
            self.select_window(f.s, w, host);
            if let Some(p) = f.p {
                self.set_active_pane(w, p, host);
            }
        }
        if a.has('d') || a.has('x') {
            self.kick_others(f.s, cid, a.has('x'), host);
        }
        self.switch_client(cid, f.s, host);
        ok()
    }

    /// `attach -d` / `-x`: detach every other client of `sid`; with `hup`
    /// also send SIGHUP to their parent processes.
    fn kick_others(&mut self, sid: SessionId, keep: ClientId, hup: bool, host: &mut dyn Host) {
        let name = self.sessions[&sid].name.clone();
        for other in self.clients_of(sid) {
            if other != keep {
                self.detach(other, DetachReason::Kicked { session: name.clone() }, host);
                if hup {
                    host.hangup_client_parent(other);
                }
            }
        }
    }

    fn cmd_detach(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let hup = a.has('P');
        let detach = |m: &mut Model, c: ClientId, host: &mut dyn Host| {
            let session = m.clients[&c].session.and_then(|s| m.sessions.get(&s)).map(|s| s.name.clone()).unwrap_or_default();
            m.detach(c, DetachReason::Detached { session }, host);
            if hup {
                host.hangup_client_parent(c);
            }
        };
        if let Some(t) = a.get('s') {
            let f = self.find_target(ctx, Some(t), Kind::Session, Flags::default())?;
            for c in self.clients_of(f.s) {
                detach(self, c, host);
            }
            return ok();
        }
        let target = self.target_client(ctx, a.get('t'))?;
        if a.has('a') {
            let others: Vec<ClientId> = self.clients.values().filter(|c| c.id != target && c.session.is_some()).map(|c| c.id).collect();
            for c in others {
                detach(self, c, host);
            }
            return ok();
        }
        if self.clients[&target].session.is_none() {
            return err("no current client");
        }
        detach(self, target, host);
        ok()
    }

    fn cmd_switch(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let cid = self.attached_client(ctx, a.get('c'))?;
        if let Some(t) = a.get('T') {
            self.clients.get_mut(&cid).expect("client").keys.set_table(t);
            if let Some(s) = self.clients[&cid].session {
                host.redraw(Redraw::Status(s));
            }
            return ok();
        }
        let cur = self.clients[&cid].session.expect("attached");
        let target = if a.has('n') || a.has('p') {
            let order = self.sessions_by_name();
            let pos = order.iter().position(|s| *s == cur).unwrap_or(0);
            let n = order.len();
            let next = if a.has('n') { order[(pos + 1) % n] } else { order[(pos + n - 1) % n] };
            if next == cur {
                return err(if a.has('n') { "can't find next session" } else { "can't find previous session" });
            }
            next
        } else if a.has('l') {
            match self.clients[&cid].last_session.filter(|s| self.sessions.contains_key(s)) {
                Some(s) => s,
                None => return err("can't find last session"),
            }
        } else {
            let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags { prefer_unattached: true, ..Flags::default() })?;
            if let Some(w) = f.w {
                self.select_window(f.s, w, host);
                // Another pane of a zoomed window: unzoom, or with -Z zoom
                // that pane instead (tmux 3.8).
                if let Some(p) = f.p
                    && self.windows[&w].active != p
                {
                    self.select_pane_zoom(w, p, a.has('Z'), host);
                }
            }
            f.s
        };
        self.switch_client(cid, target, host);
        ok()
    }

    fn cmd_display(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let client = match a.get('c') {
            Some(c) => Some(self.target_client(ctx, Some(c))?),
            None => ctx.client.filter(|c| self.clients.get(c).is_some_and(|c| c.session.is_some())),
        };
        let fmt = a.get('F').or(a.value(0)).unwrap_or(DISPLAY_MESSAGE);
        let msg = if a.has('l') {
            fmt.to_string()
        } else {
            format::expand_time(fmt, &self.fctx(&*host, Some(f.s), f.w, f.p, client.or(ctx.client), ctx.now))
        };
        match client {
            Some(c) if !a.has('p') => {
                let ms = match a.get('d') {
                    Some(d) => parse_delay(d)?,
                    None => self.sopt(Some(f.s), "display-time").as_num() as u64,
                };
                host.show_message(c, &msg, ms);
                ok()
            }
            _ => text(msg + "\n"),
        }
    }

    fn cmd_command_prompt(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if a.has('i') {
            // Copy mode's incremental searches (tmux's own -i bindings) are
            // run by copy mode itself, without this command.
            return err("command-prompt -i is not supported (incremental prompts work only for copy-mode searches)");
        }
        let cid = self.attached_client(ctx, a.get('t'))?;
        let mut template = match a.values().first() {
            Some(Arg::Str(s)) => s.clone(),
            Some(Arg::Block(b)) => b.to_string(),
            None => "%1".to_string(),
        };
        if a.has('F') {
            let f = self.current_state(ctx, false)?;
            template = self.expand_found(&f, ctx, &*host, &template);
        }
        let split = |s: Option<&str>| -> Vec<String> { s.map(|s| s.split(',').map(|x| x.to_string()).collect()).unwrap_or_default() };
        let mut prompts: Vec<String> = split(a.get('p')).into_iter().map(|p| format!("{p} ")).collect();
        if prompts.is_empty() {
            prompts.push(match a.values().first() {
                Some(_) => format!("({}) ", template.split_whitespace().next().unwrap_or("")),
                None => ":".to_string(),
            });
        }
        let mut inputs = split(a.get('I'));
        if !inputs.is_empty() {
            let f = self.current_state(ctx, false)?;
            inputs = inputs.iter().map(|i| self.expand_found(&f, ctx, &*host, i)).collect();
        }
        host.prompt(
            cid,
            Prompt::Command { prompts, inputs, template, single_key: a.has('1'), key_name: a.has('k'), numeric: a.has('N') },
        );
        ok()
    }

    fn cmd_confirm(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let cid = self.attached_client(ctx, a.get('c').or(a.get('t')))?;
        let command = match a.values().first() {
            Some(Arg::Block(b)) => b.to_string(),
            Some(Arg::Str(s)) => s.clone(),
            None => return err("no command"),
        };
        commands_from_args(a.values())?;
        let prompt = match a.get('p') {
            Some(p) => {
                let f = self.current_state(ctx, false)?;
                format!("{} ", self.expand_found(&f, ctx, &*host, p))
            }
            None => format!("Confirm '{}'? (y/n) ", command.split_whitespace().next().unwrap_or("")),
        };
        host.prompt(cid, Prompt::Confirm { prompt, command, default_yes: a.has('y') });
        ok()
    }

    // ----- windows -------------------------------------------------------

    fn cmd_new_window(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags { window_index: true, ..Flags::default() })?;
        let sid = f.s;
        let name = a.get('n').map(|n| self.expand_found(&f, ctx, &*host, n));
        if a.has('S')
            && let Some(n) = &name
            && let Some(w) = self.sessions[&sid].windows.values().copied().find(|w| self.windows[w].name == *n)
        {
            if !a.has('d') {
                self.select_window(sid, w, host);
            }
            let p = self.windows[&w].active;
            return self.print_new(a, NEW_WINDOW, sid, w, p, ctx, &*host);
        }
        if let Some(i) = f.idx
            && !a.has('a')
            && !a.has('b')
            && !a.has('k')
            && self.sessions[&sid].windows.contains_key(&i)
        {
            return err(format!("create window failed: index {i} in use"));
        }
        // Refuse before starting a process that would only be killed.
        self.check_placement(&f, a).map_err(|e| CmdError(format!("create window failed: {}", e.0)))?;
        let cwd_default = self.default_cwd(ctx, Some(sid));
        let cwd = self.cwd_arg(a, Some(&f), ctx, &*host, cwd_default);
        let size = self.new_window_size(sid);
        let (wid, pid) = self.spawn_window(sid, name, &a.strings(0), cwd, size, ctx.client, host)?;
        let (idx, displaced) = match self.place_index(&f, a, None, "create window failed: ", host) {
            Ok(x) => x,
            Err(e) => {
                host.kill_pane(pid);
                self.windows.remove(&wid);
                self.panes.remove(&pid);
                return Err(e);
            }
        };
        if !self.sessions.contains_key(&sid) {
            host.kill_pane(pid);
            self.windows.remove(&wid);
            self.panes.remove(&pid);
            return err("session gone");
        }
        self.insert_window(sid, idx, wid);
        self.replace_killed(displaced, wid, host);
        if !a.has('d') {
            self.select_window(sid, wid, host);
        }
        if let Some(sz) = self.desired_size(sid, Some(wid))
            && sz != self.windows[&wid].size
        {
            self.resize_window(wid, sz, host);
        }
        host.redraw(Redraw::Status(sid));
        self.print_new(a, NEW_WINDOW, sid, wid, pid, ctx, &*host)
    }

    fn cmd_list_windows(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let sessions = if a.has('a') {
            self.sessions_by_name()
        } else {
            vec![self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?.s]
        };
        let default = if a.has('a') { LIST_WINDOWS_ALL } else { LIST_WINDOWS };
        let mut out = String::new();
        for sid in sessions {
            for wid in self.sessions[&sid].windows.values() {
                let w = &self.windows[wid];
                let c = self.fctx(&*host, Some(sid), Some(*wid), Some(w.active), ctx.client, ctx.now);
                if let Some(filter) = a.get('f')
                    && !format::is_true(&format::expand(filter, &c))
                {
                    continue;
                }
                out.push_str(&format::expand(a.get('F').unwrap_or(default), &c));
                check_list_output(&out)?;
                out.push('\n');
            }
        }
        text(out)
    }

    fn cmd_list_panes(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let mut wins: Vec<(SessionId, WindowId)> = Vec::new();
        let prefix;
        if a.has('a') {
            prefix = "#{session_name}:#{window_index}.";
            for sid in self.sessions_by_name() {
                wins.extend(self.sessions[&sid].windows.values().map(|w| (sid, *w)));
            }
        } else if a.has('s') {
            prefix = "#{window_index}.";
            let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
            wins.extend(self.sessions[&f.s].windows.values().map(|w| (f.s, *w)));
        } else {
            prefix = "";
            let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
            wins.push((f.s, need_window(&f)?));
        }
        let default = format!("{prefix}{LIST_PANES}");
        let fmt = a.get('F').unwrap_or(&default);
        let mut out = String::new();
        for (sid, wid) in wins {
            for p in &self.windows[&wid].panes {
                let c = self.fctx(&*host, Some(sid), Some(wid), Some(*p), ctx.client, ctx.now);
                if let Some(filter) = a.get('f')
                    && !format::is_true(&format::expand(filter, &c))
                {
                    continue;
                }
                out.push_str(&format::expand(fmt, &c));
                check_list_output(&out)?;
                out.push('\n');
            }
        }
        text(out)
    }

    /// Move window `wid` to `idx` in `dst` (another session or the same).
    fn relink_window(&mut self, wid: WindowId, dst: SessionId, idx: u32, select: bool, host: &mut dyn Host) {
        let src = self.windows[&wid].session;
        let old_idx = self.windows[&wid].index;
        let s = self.sessions.get_mut(&src).expect("session");
        s.windows.remove(&old_idx);
        let was_current = s.current == wid;
        s.last.retain(|w| *w != wid);
        if src != dst {
            if s.windows.is_empty() {
                self.insert_window(dst, idx, wid);
                self.destroy_session(src, host);
            } else {
                if was_current {
                    let next = s.last.first().copied().unwrap_or_else(|| {
                        s.windows.range(..old_idx).next_back().or_else(|| s.windows.iter().next_back()).map(|(_, w)| *w).expect("not empty")
                    });
                    s.last.retain(|x| *x != next);
                    s.current = next;
                }
                if self.sopt(Some(src), "renumber-windows").as_flag() {
                    self.renumber(src);
                }
                host.redraw(Redraw::Session(src));
                self.insert_window(dst, idx, wid);
            }
            if let Some(sz) = self.desired_size(dst, Some(wid)) {
                self.resize_window(wid, sz, host);
            }
        } else {
            self.insert_window(dst, idx, wid);
        }
        if select {
            self.select_window(dst, wid, host);
        }
        host.redraw(Redraw::Session(dst));
    }

    fn cmd_move_window(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        if a.has('r') {
            let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
            self.renumber(f.s);
            host.redraw(Redraw::Status(f.s));
            return ok();
        }
        let src = self.find_target(ctx, a.get('s'), Kind::Window, Flags::default())?;
        let wid = need_window(&src)?;
        let dst = self.find_target(ctx, a.get('t'), Kind::Window, Flags { window_index: true, ..Flags::default() })?;
        let (idx, displaced) = self.place_index(&dst, a, Some(wid), "", host)?;
        if !self.windows.contains_key(&wid) || !self.sessions.contains_key(&dst.s) {
            return ok();
        }
        self.relink_window(wid, dst.s, idx, !a.has('d'), host);
        self.replace_killed(displaced, wid, host);
        ok()
    }

    fn cmd_swap_window(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let src = self.find_target(ctx, a.get('s'), Kind::Window, Flags::default())?;
        let dst = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
        let (ws, wd) = (need_window(&src)?, need_window(&dst)?);
        if ws == wd {
            return ok();
        }
        let (ss, sd) = (src.s, dst.s);
        let (is, id) = (self.windows[&ws].index, self.windows[&wd].index);
        self.sessions.get_mut(&ss).expect("session").windows.insert(is, wd);
        self.sessions.get_mut(&sd).expect("session").windows.insert(id, ws);
        {
            let w = self.windows.get_mut(&ws).expect("window");
            w.index = id;
            w.session = sd;
        }
        {
            let w = self.windows.get_mut(&wd).expect("window");
            w.index = is;
            w.session = ss;
        }
        // tmux keeps the current and last windows as indexes (winlinks), so
        // they now show the other window.
        let swap = |x: WindowId| if x == ws { wd } else if x == wd { ws } else { x };
        let both = if ss == sd { vec![ss] } else { vec![ss, sd] };
        for sid in both {
            let s = self.sessions.get_mut(&sid).expect("session");
            s.current = swap(s.current);
            for l in &mut s.last {
                *l = swap(*l);
            }
        }
        if ss != sd {
            for (sid, w) in [(ss, wd), (sd, ws)] {
                if let Some(sz) = self.desired_size(sid, Some(w)) {
                    self.resize_window(w, sz, host);
                }
            }
        }
        // Verified against tmux 3.8: with -d the destination index becomes
        // current (the moved window keeps the focus); without it the current
        // index stays and shows the other window.
        if a.has('d') {
            self.select_window(sd, ws, host);
            if ss != sd {
                self.select_window(ss, wd, host);
            }
        }
        host.redraw(Redraw::Session(ss));
        host.redraw(Redraw::Session(sd));
        ok()
    }

    // ----- panes ----------------------------------------------------------

    fn cmd_split(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, target) = (need_window(&f)?, need_pane(&f)?);
        let axis = if a.has('h') { Axis::LeftRight } else { Axis::TopBottom };
        let cur = {
            let p = &self.panes[&target];
            if axis == Axis::LeftRight { p.size.cols } else { p.size.rows }
        } as u32;
        let size = if let Some(l) = a.get('l') {
            match SizeSpec::parse(l) {
                Some(SizeSpec::Cells(n)) => SplitSize::Cells(n),
                Some(SizeSpec::Percent(p)) => SplitSize::Percent(p),
                None => return err("size invalid"),
            }
        } else if let Some(p) = a.get('p') {
            match p.parse::<u32>() {
                Ok(p) if p <= 100 => SplitSize::Percent(p),
                _ => return err("percentage invalid"),
            }
        } else {
            SplitSize::Half
        };
        let _ = cur;
        let before = a.has('b');
        let cwd_default = {
            let info = host.pane_info(target);
            if info.current_path.is_empty() { self.sessions[&f.s].cwd.clone() } else { info.current_path.to_string() }
        };
        let cwd = self.cwd_arg(a, Some(&f), ctx, &*host, cwd_default);
        let pid = self.alloc_pane()?;
        let was_zoomed = self.windows.get_mut(&wid).expect("window").layout.unzoom();
        let saved = self.windows[&wid].layout.clone();
        if let Err(e) = self.windows.get_mut(&wid).expect("window").layout.split(target, pid, axis, size, before) {
            if was_zoomed {
                self.windows.get_mut(&wid).expect("window").layout.zoom(target);
            }
            return err(e.to_string());
        }
        let psize = self.windows[&wid].layout.unzoomed_geometry().rect(pid).map(|r| r.size()).unwrap_or_default();
        let command = self.spawn_command(f.s, &a.strings(0));
        let spec = self.spawn_spec(pid, wid, f.s, command.clone(), cwd.clone(), psize, ctx.client, false);
        if let Err(e) = host.spawn_pane(&spec) {
            let w = self.windows.get_mut(&wid).expect("window");
            w.layout = saved;
            if was_zoomed {
                w.layout.zoom(target);
            }
            return err(format!("create pane failed: {e}"));
        }
        let point = self.tick();
        self.panes.insert(pid, Pane { id: pid, window: wid, size: psize, command, cwd, dead: false, exit_status: None, active_point: point });
        let w = self.windows.get_mut(&wid).expect("window");
        let pos = w.panes.iter().position(|p| *p == target).expect("target in window");
        w.panes.insert(if before { pos } else { pos + 1 }, pid);
        if !a.has('d') {
            self.set_active_pane(wid, pid, host);
        }
        if a.has('Z') {
            let w = self.windows.get_mut(&wid).expect("window");
            let z = if a.has('d') { w.active } else { pid };
            w.layout.zoom(z);
        }
        self.sync_window(wid, host);
        host.redraw(Redraw::Status(f.s));
        self.print_new(a, NEW_WINDOW, f.s, wid, pid, ctx, &*host)
    }

    fn cmd_select_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, p) = (need_window(&f)?, need_pane(&f)?);
        if a.has('l') {
            let last = self.last_pane_of(wid).ok_or_else(|| CmdError::new("no last pane"))?;
            self.select_pane_zoom(wid, last, a.has('Z'), host);
            return ok();
        }
        let dir = if a.has('L') {
            Some(Direction::Left)
        } else if a.has('R') {
            Some(Direction::Right)
        } else if a.has('U') {
            Some(Direction::Up)
        } else if a.has('D') {
            Some(Direction::Down)
        } else {
            None
        };
        let target = match dir {
            Some(d) => {
                let w = &self.windows[&wid];
                let rec = |id: PaneId| self.panes.get(&id).map(|p| p.active_point).unwrap_or(0);
                match w.layout.neighbor(p, d, &w.panes, rec) {
                    Some(n) => n,
                    None => return ok(),
                }
            }
            None => p,
        };
        if target != self.windows[&wid].active {
            self.select_pane_zoom(wid, target, a.has('Z'), host);
        }
        ok()
    }

    fn cmd_resize_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, p) = (need_window(&f)?, need_pane(&f)?);
        if a.has('Z') {
            let w = self.windows.get_mut(&wid).expect("window");
            if !w.layout.unzoom() {
                w.layout.zoom(p);
            }
            self.sync_window(wid, host);
            host.redraw(Redraw::Status(f.s));
            return ok();
        }
        let wsize = self.windows[&wid].size;
        if let Some(x) = a.get('x') {
            let n = parse_size_arg(x, wsize.cols as u32, "width")?;
            self.unzoom(wid, host);
            self.windows.get_mut(&wid).expect("window").layout.resize_pane_to(p, Axis::LeftRight, n);
        }
        if let Some(y) = a.get('y') {
            let n = parse_size_arg(y, wsize.rows as u32, "height")?;
            self.unzoom(wid, host);
            self.windows.get_mut(&wid).expect("window").layout.resize_pane_to(p, Axis::TopBottom, n);
        }
        let amount = match a.value(0) {
            Some(v) => v.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| CmdError(format!("adjustment invalid: {v}")))?,
            None => 1,
        };
        for (flag, dir) in [('L', Direction::Left), ('R', Direction::Right), ('U', Direction::Up), ('D', Direction::Down)] {
            if a.has(flag) {
                self.unzoom(wid, host);
                self.windows.get_mut(&wid).expect("window").layout.resize_pane(p, dir, amount);
            }
        }
        self.sync_window(wid, host);
        ok()
    }

    fn cmd_swap_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let dst = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (dw, dp) = (need_window(&dst)?, need_pane(&dst)?);
        let (sw, sp) = if a.has('D') || a.has('U') {
            let w = &self.windows[&dw];
            let pos = w.panes.iter().position(|p| *p == dp).expect("pane in window");
            let n = w.panes.len();
            let i = if a.has('D') { (pos + 1) % n } else { (pos + n - 1) % n };
            (dw, w.panes[i])
        } else {
            let src = self.find_target(ctx, a.get('s'), Kind::Pane, Flags::default())?;
            (need_window(&src)?, need_pane(&src)?)
        };
        if sp == dp {
            return ok();
        }
        let keep_zoom = a.has('Z');
        let src_zoomed = self.windows.get_mut(&sw).expect("window").layout.unzoom();
        let dst_zoomed = self.windows.get_mut(&dw).expect("window").layout.unzoom();
        if sw == dw {
            let w = self.windows.get_mut(&sw).expect("window");
            w.layout.swap(sp, dp);
            let (i, j) = (w.panes.iter().position(|p| *p == sp).expect("src"), w.panes.iter().position(|p| *p == dp).expect("dst"));
            w.panes.swap(i, j);
        } else {
            let (si, di) = (
                self.windows[&sw].panes.iter().position(|p| *p == sp).expect("src"),
                self.windows[&dw].panes.iter().position(|p| *p == dp).expect("dst"),
            );
            {
                let w = self.windows.get_mut(&sw).expect("window");
                w.layout.replace(sp, dp);
                w.panes[si] = dp;
                w.last_panes.retain(|p| *p != sp);
                if w.active == sp {
                    w.active = dp;
                }
            }
            {
                let w = self.windows.get_mut(&dw).expect("window");
                w.layout.replace(dp, sp);
                w.panes[di] = sp;
                w.last_panes.retain(|p| *p != dp);
                if w.active == dp {
                    w.active = sp;
                }
            }
            self.panes.get_mut(&sp).expect("pane").window = dw;
            self.panes.get_mut(&dp).expect("pane").window = sw;
        }
        if !a.has('d') {
            if sw != dw {
                self.set_active_pane(sw, dp, host);
                self.set_active_pane(dw, sp, host);
            } else {
                self.set_active_pane(sw, dp, host);
            }
        } else {
            if self.windows[&sw].active == sp {
                self.set_active_pane(sw, dp, host);
            }
            if self.windows[&dw].active == dp {
                self.set_active_pane(dw, sp, host);
            }
        }
        for (w, z) in [(sw, src_zoomed), (dw, dst_zoomed)] {
            if z && keep_zoom {
                let win = self.windows.get_mut(&w).expect("window");
                let act = win.active;
                win.layout.zoom(act);
            }
            self.sync_window(w, host);
        }
        ok()
    }

    fn cmd_break_pane(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let src = self.find_target(ctx, a.get('s'), Kind::Pane, Flags::default())?;
        let (sw, p) = (need_window(&src)?, need_pane(&src)?);
        let dst = match a.get('t') {
            Some(t) => self.find_target(ctx, Some(t), Kind::Window, Flags { window_index: true, ..Flags::default() })?,
            None => Found { s: src.s, w: None, p: None, idx: None },
        };
        let name = a.get('n').map(|n| self.expand_found(&src, ctx, &*host, n));
        if self.windows[&sw].panes.len() == 1 {
            // Only one pane: move the whole window, as tmux does.
            let (idx, displaced) = if dst.idx.is_none() && !a.has('a') && !a.has('b') {
                (self.first_free_index(dst.s)?, None)
            } else {
                self.place_index(&dst, a, Some(sw), "", host)?
            };
            if let Some(n) = name {
                let w = self.windows.get_mut(&sw).expect("window");
                w.name = n;
                w.options.set("automatic-rename", Value::Flag(false));
            }
            self.relink_window(sw, dst.s, idx, !a.has('d'), host);
            self.replace_killed(displaced, sw, host);
            let p = self.windows[&sw].active;
            return self.print_new(a, NEW_WINDOW, dst.s, sw, p, ctx, &*host);
        }
        // Both can fail; take them before the pane leaves its window.
        let wid = self.alloc_window()?;
        let (idx, displaced) = self.place_index(&dst, a, None, "", host)?;
        // Take the pane out of its window.
        {
            let w = self.windows.get_mut(&sw).expect("window");
            w.layout.unzoom();
            w.layout.remove(p);
            let pos = w.panes.iter().position(|x| *x == p).expect("pane");
            w.last_panes.retain(|x| *x != p);
            if w.active == p {
                let next = w.last_panes.first().copied().or_else(|| if pos > 0 { Some(w.panes[pos - 1]) } else { w.panes.get(pos + 1).copied() });
                if let Some(n) = next {
                    w.active = n;
                    w.last_panes.retain(|x| *x != n);
                }
            }
            w.panes.remove(pos);
        }
        self.sync_window(sw, host);
        let size = self.new_window_size(dst.s);
        let mut options = OptionSet::default();
        let wname = match name {
            Some(n) => {
                options.set("automatic-rename", Value::Flag(false));
                n
            }
            None => {
                let info = host.pane_info(p);
                if info.current_command.is_empty() { self.panes[&p].command.program_name() } else { info.current_command.to_string() }
            }
        };
        self.windows.insert(
            wid,
            Window {
                id: wid,
                session: dst.s,
                index: 0,
                name: wname,
                layout: Layout::new(p, size),
                panes: vec![p],
                active: p,
                last_panes: Vec::new(),
                bell: false,
                activity: false,
                options,
                last_preset: None,
                old_layout: None,
                size,
            },
        );
        self.panes.get_mut(&p).expect("pane").window = wid;
        self.insert_window(dst.s, idx, wid);
        self.replace_killed(displaced, wid, host);
        if !a.has('d') {
            self.select_window(dst.s, wid, host);
        }
        self.sync_window(wid, host);
        host.redraw(Redraw::Session(dst.s));
        self.print_new(a, NEW_WINDOW, dst.s, wid, p, ctx, &*host)
    }

    fn cmd_respawn(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, p) = (need_window(&f)?, need_pane(&f)?);
        if !self.panes[&p].dead && !a.has('k') {
            let s = &self.sessions[&f.s].name;
            let widx = self.windows[&wid].index;
            let pidx = self.pane_index(p).unwrap_or(0);
            return err(format!("respawn pane failed: pane {s}:{widx}.{pidx} still active"));
        }
        let args = a.strings(0);
        let command = if args.is_empty() { self.panes[&p].command.clone() } else { self.spawn_command(f.s, &args) };
        let start = self.panes[&p].cwd.clone();
        let cwd = self.cwd_arg(a, Some(&f), ctx, &*host, start);
        host.kill_pane(p);
        let size = self.panes[&p].size;
        let spec = self.spawn_spec(p, wid, f.s, command.clone(), cwd.clone(), size, ctx.client, true);
        let r = host.spawn_pane(&spec);
        let pane = self.panes.get_mut(&p).expect("pane");
        match r {
            Ok(()) => {
                pane.dead = false;
                pane.exit_status = None;
                pane.command = command;
                pane.cwd = cwd;
                host.redraw(Redraw::Window(wid));
                ok()
            }
            Err(e) => {
                pane.dead = true;
                host.redraw(Redraw::Window(wid));
                err(format!("respawn pane failed: {e}"))
            }
        }
    }

    fn cmd_select_layout(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let (wid, p) = (need_window(&f)?, need_pane(&f)?);
        if a.has('o') {
            let w = self.windows.get_mut(&wid).expect("window");
            if let Some(old) = w.old_layout.take()
                && old.panes().len() == w.panes.len()
                && old.panes().iter().all(|x| w.panes.contains(x))
            {
                let cur = std::mem::replace(&mut w.layout, old);
                w.old_layout = Some(cur);
                let size = w.size;
                w.layout.resize(size);
            }
            self.sync_window(wid, host);
            return ok();
        }
        if a.has('E') {
            let w = self.windows.get_mut(&wid).expect("window");
            w.old_layout = Some(w.layout.clone());
            w.layout.unzoom();
            w.layout.spread_out(p);
            self.sync_window(wid, host);
            return ok();
        }
        let last = self.windows[&wid].last_preset;
        let preset = if a.has('n') {
            Some(last.map(|p| p.next()).unwrap_or(Preset::ALL[0]))
        } else if a.has('p') {
            Some(last.map(|p| p.previous()).unwrap_or(Preset::ALL[0]))
        } else {
            match a.value(0) {
                None => match last {
                    Some(p) => Some(p),
                    None => return ok(),
                },
                Some(name) => match Preset::lookup(name) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        let w = &self.windows[&wid];
                        match Layout::parse(name, &w.panes, w.size) {
                            Ok(l) => {
                                let w = self.windows.get_mut(&wid).expect("window");
                                w.old_layout = Some(std::mem::replace(&mut w.layout, l));
                                self.sync_window(wid, host);
                                return ok();
                            }
                            Err(pe) if name.contains(',') => return err(pe.to_string()),
                            Err(_) => return err(e.to_string()),
                        }
                    }
                },
            }
        };
        if let Some(p) = preset {
            self.apply_preset(wid, p, host);
        }
        ok()
    }

    // ----- input, buffers --------------------------------------------------

    fn cmd_send_keys(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let p = need_pane(&f)?;
        let count = match a.get('N') {
            Some(n) => match n.parse::<u64>() {
                Ok(c) if c > MAX_REPEAT as u64 => return err(format!("repeat count too large: {n}")),
                Ok(c) if c > 0 => c as u32,
                _ if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.bytes().any(|b| b != b'0') => {
                    return err(format!("repeat count too large: {n}"));
                }
                _ => return err(format!("repeat count invalid: {n}")),
            },
            None => 1,
        };
        let keys: Vec<String> = a
            .strings(0)
            .into_iter()
            .map(|k| if a.has('F') { self.expand_found(&f, ctx, &*host, &k) } else { k })
            .collect();
        if a.has('X') {
            let Some((name, rest)) = keys.split_first() else { return err("no command specified") };
            // tmux prints copy-mode arguments after `--` (`goto-line -- "%%"`).
            let rest = match rest.first() {
                Some(d) if d == "--" => &rest[1..],
                _ => rest,
            };
            if a.has('R') {
                host.send_to_pane(p, PaneInput::Reset).map_err(CmdError)?;
            }
            return host.mode_command(ctx.client, p, name, rest, count).map(|()| CmdOutput::default()).map_err(CmdError);
        }
        // Everything is parsed and measured before anything is sent, so a
        // command over the limits sends nothing.
        enum Item<'k> {
            Key(lif_types::KeyEvent),
            Text(&'k str),
        }
        let (items, raw) = if a.has('H') {
            let mut bytes = Vec::with_capacity(keys.len());
            for k in &keys {
                let b = u8::from_str_radix(k.trim_start_matches("0x"), 16).map_err(|_| CmdError::new("invalid key"))?;
                bytes.push(b);
            }
            (Vec::new(), bytes)
        } else {
            let items: Vec<Item> = keys
                .iter()
                .map(|k| match KeyCode::parse(k).filter(|_| !a.has('l')) {
                    Some(code) => Item::Key(code.to_event()),
                    None => Item::Text(k),
                })
                .collect();
            (items, Vec::new())
        };
        let per_round: u64 = raw.len() as u64
            + items.iter().map(|i| match i {
                Item::Key(_) => KEY_INPUT_BYTES,
                Item::Text(t) => t.len() as u64,
            }).sum::<u64>();
        let total = per_round * count as u64;
        if total > MAX_SEND_BYTES {
            return err(format!("too much input: {total} bytes (at most {MAX_SEND_BYTES} per command)"));
        }
        if a.has('R') {
            host.send_to_pane(p, PaneInput::Reset).map_err(CmdError)?;
        }
        if items.is_empty() && raw.is_empty() {
            return ok();
        }
        for _ in 0..count {
            if !raw.is_empty() {
                host.send_to_pane(p, PaneInput::Bytes(&raw)).map_err(CmdError)?;
            }
            for i in &items {
                let input = match i {
                    Item::Key(ev) => PaneInput::Key(ev),
                    Item::Text(t) => PaneInput::Text(t),
                };
                host.send_to_pane(p, input).map_err(CmdError)?;
            }
        }
        ok()
    }

    /// tmux's `source-file` for lif's TOML files: every command that can
    /// run does; problems (the file's and the commands') are logged for
    /// `show-messages` and returned as the error, one per line. Files may
    /// source files, at most [`MAX_SOURCE_DEPTH`] deep (a file that sources
    /// itself would otherwise recurse until the stack overflows).
    fn cmd_source_file(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let depth = SOURCE_DEPTH.get();
        if depth >= MAX_SOURCE_DEPTH {
            return err("too many nested files");
        }
        SOURCE_DEPTH.set(depth + 1);
        let r = self.source_files(a, ctx, host);
        SOURCE_DEPTH.set(depth);
        r
    }

    fn source_files(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let mut out = String::new();
        let mut errors: Vec<String> = Vec::new();
        for path in a.strings(0) {
            let path = if a.has('F') {
                let c = self.fctx(&*host, None, None, None, ctx.client, ctx.now);
                format::expand(&path, &c)
            } else {
                path
            };
            let file = match host.read_config(&path) {
                Ok(f) => f,
                Err(ConfigError::Missing(_)) if a.has('q') => continue,
                Err(ConfigError::Missing(m) | ConfigError::Invalid(m)) => {
                    self.log_message(m.clone());
                    errors.push(m);
                    continue;
                }
            };
            for (is_error, d) in file.diagnostics {
                self.log_message(d.clone());
                if is_error {
                    errors.push(d);
                } else {
                    out.push_str(&d);
                    out.push('\n');
                }
            }
            if a.has('n') {
                continue;
            }
            for (cmd, at) in &file.commands {
                if a.has('v') {
                    out.push_str(&format!("{at}: {cmd}\n"));
                }
                match parse_string(cmd).and_then(|l| self.execute_list(&l, ctx, host)) {
                    Ok(o) => out.push_str(&o.stdout),
                    Err(e) => {
                        let m = format!("{at}: {e}");
                        self.log_message(m.clone());
                        errors.push(m);
                    }
                }
            }
        }
        if errors.is_empty() { text(out) } else { Err(CmdError(errors.join("\n"))) }
    }

    fn cmd_capture(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let p = need_pane(&f)?;
        let pos = |v: Option<&str>, dash: LinePos, default: LinePos| -> Result<LinePos, CmdError> {
            match v {
                None => Ok(default),
                Some("-") => Ok(dash),
                // Clamped (the host clamps to the lines there are anyway) so the
                // host's line arithmetic cannot overflow.
                Some(s) => s
                    .parse::<i64>()
                    .map(|n| LinePos::Line(n.clamp(-(i32::MAX as i64), i32::MAX as i64)))
                    .map_err(|_| CmdError(format!("line invalid: {s}"))),
            }
        };
        let range = CaptureRange {
            start: pos(a.get('S'), LinePos::HistoryStart, LinePos::Line(0))?,
            end: pos(a.get('E'), LinePos::ScreenEnd, LinePos::ScreenEnd)?,
            join_wrapped: a.has('J'),
            keep_trailing: a.has('N') || a.has('J'),
            styled: a.has('e'),
        };
        let out = host.capture_pane(p, range);
        if a.has('p') {
            return text(out);
        }
        self.set_buffer(a.get('b'), out.into_bytes());
        ok()
    }

    fn cmd_paste(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        let p = need_pane(&f)?;
        let i = match a.get('b') {
            Some(_) => self.buffer_index(a.get('b'))?,
            None if self.buffers.is_empty() => return ok(),
            None => 0,
        };
        let data = &self.buffers[i].data;
        let data: Vec<u8> = if a.has('r') {
            data.clone()
        } else {
            let sep = a.get('s').unwrap_or("\r").as_bytes();
            let mut out = Vec::with_capacity(data.len());
            for &b in data {
                if b == b'\n' {
                    out.extend_from_slice(sep);
                } else {
                    out.push(b);
                }
            }
            out
        };
        host.send_to_pane(p, PaneInput::Paste { data: &data, bracketed: a.has('p') }).map_err(CmdError)?;
        if a.has('d') {
            self.buffers.remove(i);
        }
        ok()
    }

    fn cmd_set_buffer(&mut self, a: &Args) -> R {
        let data = a.value(0);
        if let Some(new) = a.get('n') {
            let i = self.buffer_index(a.get('b'))?;
            if data.is_none() {
                self.buffers.retain(|b| b.name != new);
                let i = self.buffer_index(a.get('b')).unwrap_or(i.min(self.buffers.len().saturating_sub(1)));
                let b = &mut self.buffers[i];
                b.name = new.to_string();
                b.automatic = false;
                return ok();
            }
        }
        let Some(data) = data else { return err("no data specified") };
        let mut bytes = data.as_bytes().to_vec();
        if a.has('a')
            && let Ok(i) = self.buffer_index(a.get('b'))
        {
            let mut old = self.buffers[i].data.clone();
            old.extend_from_slice(&bytes);
            bytes = old;
            let name = self.buffers[i].name.clone();
            let automatic = self.buffers[i].automatic;
            self.buffers.remove(i);
            self.buffers.insert(0, PasteBuffer { name, data: bytes, automatic });
            return ok();
        }
        self.set_buffer(a.get('b'), bytes);
        ok()
    }

    // ----- keys and options --------------------------------------------------

    fn cmd_bind(&mut self, a: &Args) -> R {
        let name = a.value(0).unwrap_or("");
        let key = KeyCode::parse(name).ok_or_else(|| CmdError(format!("unknown key: {name}")))?;
        let table = if a.has('n') { ROOT_TABLE } else { a.get('T').unwrap_or(PREFIX_TABLE) };
        let commands = if a.values().len() > 1 { commands_from_args(&a.values()[1..])? } else { CommandList::default() };
        self.bindings.bind(table, Binding { key, repeat: a.has('r'), note: a.get('N').map(|s| s.to_string()), commands });
        ok()
    }

    fn cmd_list_keys(&mut self, a: &Args) -> R {
        let table = a.get('T');
        if let Some(t) = table
            && !self.bindings.has_table(t)
        {
            return err(format!("table {t} doesn't exist"));
        }
        let key = match a.value(0) {
            Some(k) => Some(KeyCode::parse(k).ok_or_else(|| CmdError(format!("invalid key: {k}")))?),
            None => None,
        };
        if a.has('N') {
            let prefix = a.get('P').unwrap_or("C-b ");
            let mut out = String::new();
            let tables: Vec<String> = match table {
                Some(t) => vec![t.to_string()],
                None => vec![PREFIX_TABLE.to_string(), ROOT_TABLE.to_string()],
            };
            for t in tables {
                for b in self.bindings.bindings(&t) {
                    if key.is_some_and(|k| k != b.key) {
                        continue;
                    }
                    if let Some(n) = &b.note {
                        let p = if t == PREFIX_TABLE { prefix } else { "" };
                        out.push_str(&format!("{p}{} {n}\n", b.key));
                    }
                }
            }
            return text(out);
        }
        let mut out = self.bindings.list(table, key.as_ref());
        if a.has('1')
            && let Some(first) = out.lines().next()
        {
            out = format!("{first}\n");
        }
        text(out)
    }

    fn cmd_set_option(&mut self, a: &Args, window_flag: bool, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let name = a.value(0).unwrap_or("");
        let value: Option<String> = a.strings(1).into_iter().next();
        let global = a.has('g');
        // Work out which table to change.
        let (scope, def) = if name.starts_with('@') {
            (if window_flag { Scope::Window } else if a.has('s') { Scope::Server } else { Scope::Session }, None)
        } else {
            match options::find(name) {
                Ok(d) => (d.scope, Some(d)),
                Err(_) if a.has('q') => return ok(),
                Err(e) => return err(e),
            }
        };
        let target = match scope {
            Scope::Server => None,
            Scope::Session if global => None,
            Scope::Window if global => None,
            Scope::Session => Some(self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?),
            Scope::Window => Some(self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?),
        };
        let value = match (value, a.has('F')) {
            (Some(v), true) => {
                let f = match &target {
                    Some(f) => *f,
                    None => self.current_state(ctx, false).unwrap_or(Found { s: SessionId(u32::MAX), w: None, p: None, idx: None }),
                };
                Some(self.expand_found(&f, ctx, &*host, &v))
            }
            (v, _) => v,
        };
        let (sid, wid) = (target.map(|f| f.s), target.and_then(|f| f.w));
        let set: &mut OptionSet = match (scope, &target) {
            (Scope::Server, _) => &mut self.server_opts,
            (Scope::Session, None) => &mut self.session_opts,
            (Scope::Window, None) => &mut self.window_opts,
            (Scope::Session, Some(f)) => &mut self.sessions.get_mut(&f.s).expect("session").options,
            (Scope::Window, Some(f)) => &mut self.windows.get_mut(&need_window(f)?).expect("window").options,
        };
        let is_global = target.is_none();
        match def {
            None => {
                if a.has('u') {
                    set.unset_user(name);
                } else {
                    if a.has('o') && set.user(name).is_some() {
                        return err(format!("already set: {name}"));
                    }
                    let mut v = value.unwrap_or_default();
                    if a.has('a') {
                        v = format!("{}{v}", set.user(name).unwrap_or(""));
                    }
                    set.set_user(name, v);
                }
            }
            Some(def) => {
                if a.has('u') {
                    if is_global {
                        let d = OptionSet::defaults(def.scope);
                        set.set(def.name, d.get(def.name).expect("default").clone());
                    } else {
                        set.unset(def.name);
                    }
                } else {
                    // Global tables hold every option (defaults included), so
                    // tmux refuses -o there always (tmux 3.8).
                    if a.has('o') && (is_global || set.get(def.name).is_some()) {
                        return err(format!("already set: {}", def.name));
                    }
                    let cur = set.get(def.name).cloned();
                    let cur = match cur {
                        Some(c) => c,
                        None => self.opt_value(def.name, def.scope, sid, wid).clone(),
                    };
                    let v = options::parse_value(def, &cur, value.as_deref(), a.has('a')).map_err(CmdError)?;
                    let set: &mut OptionSet = match (scope, &target) {
                        (Scope::Server, _) => &mut self.server_opts,
                        (Scope::Session, None) => &mut self.session_opts,
                        (Scope::Window, None) => &mut self.window_opts,
                        (Scope::Session, Some(f)) => &mut self.sessions.get_mut(&f.s).expect("session").options,
                        (Scope::Window, Some(f)) => &mut self.windows.get_mut(&need_window(f)?).expect("window").options,
                    };
                    set.set(def.name, v);
                }
            }
        }
        // Options can change window sizes (status), names and the status line.
        let sids: Vec<SessionId> = self.sessions.keys().copied().collect();
        let wids: Vec<WindowId> = self.windows.keys().copied().collect();
        for w in wids {
            self.update_window_name(w, &*host);
        }
        for s in sids {
            self.recalc_session(s, host);
        }
        ok()
    }

    fn cmd_show_options(&mut self, a: &Args, window_flag: bool, ctx: &CmdContext) -> R {
        let global = a.has('g');
        let only = a.value(0);
        let def = match only {
            Some(n) if !n.starts_with('@') => match options::find(n) {
                Ok(d) => Some(d),
                Err(_) if a.has('q') => return ok(),
                Err(e) => return err(e),
            },
            _ => None,
        };
        let scope = match def {
            Some(d) => d.scope,
            None if a.has('s') => Scope::Server,
            None if window_flag => Scope::Window,
            None => Scope::Session,
        };
        let set: &OptionSet = match scope {
            Scope::Server => &self.server_opts,
            Scope::Session if global => &self.session_opts,
            Scope::Window if global => &self.window_opts,
            Scope::Session => &self.sessions[&self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?.s].options,
            Scope::Window => {
                let f = self.find_target(ctx, a.get('t'), Kind::Window, Flags::default())?;
                &self.windows[&need_window(&f)?].options
            }
        };
        let mut out = String::new();
        let mut line = |name: &str, value: String| {
            if a.has('v') {
                out.push_str(&value);
            } else {
                out.push_str(name);
                out.push(' ');
                out.push_str(&escape_arg(&value));
            }
            out.push('\n');
        };
        match (only, def) {
            (Some(n), None) => {
                if let Some(v) = set.user(n) {
                    line(n, v.to_string());
                } else if !a.has('q') {
                    return err(format!("invalid option: {n}"));
                }
            }
            (Some(_), Some(d)) => {
                if let Some(v) = set.get(d.name) {
                    line(d.name, options::value_text(d, v));
                }
            }
            (None, _) => {
                for (n, v) in set.iter() {
                    if let Some(d) = options::DEFS.iter().find(|d| d.name == n) {
                        line(n, options::value_text(d, v));
                    }
                }
                for (n, v) in set.user_iter() {
                    line(n, v.to_string());
                }
            }
        }
        text(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_templates() {
        assert_eq!(expand_prompt_template("rename-window \"%%\"", &["new name"]), "rename-window \"new name\"");
        assert_eq!(expand_prompt_template("select-window -t \":%%\"", &["3"]), "select-window -t \":3\"");
        assert_eq!(expand_prompt_template("%1", &["split-window -h"]), "split-window -h");
        assert_eq!(expand_prompt_template("a %% b %%", &["x", "y"]), "a x b y");
        assert_eq!(expand_prompt_template("x %1 %1", &["v"]), "x v v");
        assert_eq!(expand_prompt_template("x \"%%%\"", &["a\"b$"]), "x \"a\\\"b\\$\"");
        assert_eq!(expand_prompt_template("100%", &["v"]), "100%");
    }
}
