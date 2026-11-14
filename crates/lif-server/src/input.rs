//! What arrives from a client's terminal, and the events panes raise.

use std::time::Instant;

use lif_core::{ClientId, Host, KeyDecision, PaneId};
use lif_input::{InputEvent, TermResponse};
use lif_types::{Key, KeyAction, KeyEvent, MouseAction, MouseButton, MouseEvent, Mods, Pos};
use lif_vt::{VtEngine, VtEvent};

use crate::client::Drag;
use crate::mode::{ModeAction, ModeCtx, new_copy_mode};
use crate::prompt::PromptAction;
use crate::server::Server;

/// Lines one wheel notch scrolls.
const WHEEL_LINES: i32 = 3;

impl Server {
    pub(crate) fn client_input(&mut self, id: ClientId, bytes: &[u8], now: Instant) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let mut events = std::mem::take(&mut a.events);
        a.parser.feed(bytes, &mut events);
        a.input_deadline = a.parser.pending_timeout().map(|t| now + t);
        for ev in events.drain(..) {
            self.input_event(id, ev, now);
        }
        if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.events = events;
        }
    }

    /// The parser's pending-sequence timer fired.
    pub(crate) fn flush_input(&mut self, id: ClientId, now: Instant) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        a.input_deadline = None;
        let mut events = std::mem::take(&mut a.events);
        a.parser.flush_pending(&mut events);
        for ev in events.drain(..) {
            self.input_event(id, ev, now);
        }
        if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.events = events;
        }
    }

    fn input_event(&mut self, id: ClientId, ev: InputEvent, now: Instant) {
        match ev {
            InputEvent::Key(k) => self.key(id, &k, now),
            InputEvent::Mouse(m) => self.mouse(id, &m),
            InputEvent::PasteStart => self.paste_start(id),
            InputEvent::PasteData(d) => self.paste_data(id, &d),
            InputEvent::PasteEnd => self.paste_end(id),
            InputEvent::Focus(f) => {
                let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
                a.focused = f;
                if let Some(p) = a.active_pane {
                    self.focus_pane(p, f);
                }
            }
            InputEvent::Response(r) => self.response(id, r),
            InputEvent::Unknown(_) => {}
        }
    }

    fn active_pane(&self, id: ClientId) -> Option<PaneId> {
        self.rt.clients.get(&id)?.att.as_ref()?.view.as_ref().map(|v| v.active_pane)
    }

    pub(crate) fn focus_pane(&mut self, pane: PaneId, focused: bool) {
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let mut b = Vec::new();
        p.engine.encode_focus(focused, &mut b);
        self.rt.write_pane(pane, &b);
    }

    fn key(&mut self, id: ClientId, key: &KeyEvent, now: Instant) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        if let Some(mut p) = a.prompt.take() {
            match p.key(key, &mut self.rt.prompt_history) {
                PromptAction::Continue => {
                    if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                        a.prompt = Some(p);
                        a.needs_frame = true;
                    }
                }
                PromptAction::Cancel => self.rt.needs_frame(id),
                PromptAction::Run(s) => {
                    self.rt.needs_frame(id);
                    self.run_string(id, &s);
                }
            }
            return;
        }
        let pressed = key.action != KeyAction::Release && !matches!(key.key, Key::Modifier(_));
        if let Some(dp) = a.display_panes.as_ref()
            && pressed
        {
            let pick = match key.key {
                Key::Char(c) if c.is_ascii_digit() => {
                    let d = c.to_string();
                    dp.labels.iter().find(|(_, l)| *l == d).map(|(p, _)| (*p, dp.template.clone()))
                }
                _ => None,
            };
            a.display_panes = None;
            a.needs_frame = true;
            if let Some((p, template)) = pick {
                let cmd = template.replace("%%", &p.to_string());
                self.run_string(id, &cmd);
                return;
            }
        }
        if pressed && let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) && a.message.is_some() {
            a.message = None;
            a.needs_frame = true;
        }
        let now_ms = self.now_ms(now);
        let decision = self.model.handle_key(id, key, now_ms, &mut self.rt);
        let deadline = self.model.client(id).and_then(|c| c.key_state().repeat_deadline());
        if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.key_deadline = deadline.map(|d| self.start + std::time::Duration::from_millis(d));
        }
        match decision {
            KeyDecision::SendToPane => {
                if let Some(p) = self.active_pane(id) {
                    self.key_to_pane(id, p, key);
                }
            }
            KeyDecision::Run(list) => {
                if let Err(e) = self.run_commands(Some(id), &list).map(|o| self.show_output(id, &o.stdout)) {
                    self.rt.show_message(id, &e.to_string(), self.display_time());
                }
            }
            KeyDecision::Swallow => {}
        }
        self.sync_views();
    }

    fn key_to_pane(&mut self, client: ClientId, pane: PaneId, key: &KeyEvent) {
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        if let Some(m) = p.mode.as_mut() {
            let action = m.key(key, &mut ModeCtx { engine: &mut p.engine, rows: p.size.rows });
            self.mode_action(pane, action);
            return;
        }
        let mut b = std::mem::take(&mut self.replies);
        b.clear();
        p.engine.encode_key(key, &mut b);
        self.rt.write_pane(pane, &b);
        self.replies = b;
        self.block_if_full(client, pane);
    }

    /// Stop reading from a client whose pane does not take input.
    fn block_if_full(&mut self, client: ClientId, pane: PaneId) {
        let full = self.rt.panes.get(&pane).is_some_and(|p| p.wq.len() > crate::PANE_WRITE_HIGH);
        if full && let Some(c) = self.rt.clients.get_mut(&client) {
            c.blocked_on = Some(pane);
        }
    }

    pub(crate) fn mode_action(&mut self, pane: PaneId, action: ModeAction) {
        match action {
            ModeAction::Stay => {}
            ModeAction::Exit => {
                if let Some(p) = self.rt.panes.get_mut(&pane) {
                    p.leave_mode();
                }
                self.model.pane_changed(pane, &mut self.rt);
            }
            ModeAction::Copy { data, exit } => {
                self.clipboard_write(pane, b'c', &data, true);
                if exit {
                    self.mode_action(pane, ModeAction::Exit);
                }
            }
        }
        self.rt.mark_all(false);
    }

    fn display_time(&self) -> u64 {
        self.model.option_text("display-time", None, None).and_then(|v| v.parse().ok()).unwrap_or(750)
    }

    fn show_output(&mut self, id: ClientId, out: &str) {
        let text = out.trim_end();
        if !text.is_empty() {
            let d = self.display_time();
            self.rt.show_message(id, text.lines().next_back().unwrap_or(""), d);
        }
    }

    pub(crate) fn run_string(&mut self, id: ClientId, s: &str) {
        let r = lif_core::parse_string(s).and_then(|list| self.run_commands(Some(id), &list));
        match r {
            Ok(o) => self.show_output(id, &o.stdout),
            Err(e) => {
                let d = self.display_time();
                self.rt.show_message(id, &e.to_string(), d);
            }
        }
    }

    // ----- mouse ---------------------------------------------------------

    fn mouse(&mut self, id: ClientId, m: &MouseEvent) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let Some(view) = a.view.as_ref() else { return };
        let (x, y) = (m.cell.x, m.cell.y);
        if let Some(drag) = a.drag {
            match m.action {
                MouseAction::Motion => {
                    let at = if drag.vertical { x } else { y };
                    if at != drag.last {
                        let (flag, n) = match (drag.vertical, at > drag.last) {
                            (true, true) => ("-R", at - drag.last),
                            (true, false) => ("-L", drag.last - at),
                            (false, true) => ("-D", at - drag.last),
                            (false, false) => ("-U", drag.last - at),
                        };
                        a.drag = Some(Drag { last: at, ..drag });
                        let cmd = format!("resize-pane -t {} {flag} {n}", drag.pane);
                        self.run_string(id, &cmd);
                    }
                }
                _ => a.drag = None,
            }
            return;
        }
        let mouse_opt = a.mouse_opt;
        if view.status_row == Some(y) {
            if mouse_opt && m.action == MouseAction::Press && m.button == Some(MouseButton::Left) {
                let sid = view.session;
                let target = self.status_window_at(id, x);
                if let Some(idx) = target {
                    let cmd = format!("select-window -t {}:{idx}", sid);
                    self.run_string(id, &cmd);
                }
            }
            return;
        }
        let hit = view.panes.iter().find(|(_, r)| r.contains(x, y)).map(|(p, r)| (*p, *r));
        let Some((pane, rect)) = hit else {
            if mouse_opt && m.action == MouseAction::Press && m.button == Some(MouseButton::Left) {
                a.drag = border_drag(view, x, y);
            }
            return;
        };
        let active = view.active_pane;
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let modes = p.engine.modes();
        if modes.mouse_tracking && p.mode.is_none() {
            let local = MouseEvent { cell: Pos { x: x - rect.x, y: y - rect.y }, ..*m };
            let mut b = Vec::new();
            p.engine.encode_mouse(&local, &mut b);
            self.rt.write_pane(pane, &b);
            if mouse_opt && m.action == MouseAction::Press && pane != active {
                self.run_string(id, &format!("select-pane -t {pane}"));
            }
            return;
        }
        if !mouse_opt {
            return;
        }
        match (m.action, m.button) {
            (MouseAction::Press, Some(MouseButton::Left)) if pane != active => {
                self.run_string(id, &format!("select-pane -t {pane}"));
            }
            (MouseAction::Press, Some(b @ (MouseButton::WheelUp | MouseButton::WheelDown))) => {
                let up = b == MouseButton::WheelUp;
                if modes.alt_screen && p.mode.is_none() {
                    // A full-screen program without mouse support: arrow
                    // keys, as terminals do in alternate-scroll mode.
                    let k = KeyEvent::new(if up { Key::Up } else { Key::Down }, Mods::NONE);
                    let mut bytes = Vec::new();
                    for _ in 0..WHEEL_LINES {
                        p.engine.encode_key(&k, &mut bytes);
                    }
                    self.rt.write_pane(pane, &bytes);
                    return;
                }
                if p.mode.is_none() {
                    if !up {
                        return;
                    }
                    let opts = lif_core::CopyModeOptions { exit_at_bottom: true, ..Default::default() };
                    p.mode = Some(new_copy_mode(opts, &mut ModeCtx { engine: &mut p.engine, rows: p.size.rows }));
                }
                let lines = if up { -WHEEL_LINES } else { WHEEL_LINES };
                let action = match p.mode.as_mut() {
                    Some(mode) => mode.scroll(lines, &mut ModeCtx { engine: &mut p.engine, rows: p.size.rows }),
                    None => ModeAction::Stay,
                };
                self.mode_action(pane, action);
                self.model.pane_changed(pane, &mut self.rt);
            }
            _ => {}
        }
    }

    /// The window whose entry in the status line covers column `x`.
    fn status_window_at(&self, id: ClientId, x: u16) -> Option<u32> {
        let a = self.rt.clients.get(&id)?.att.as_ref()?;
        let s = self.model.session(a.session)?;
        let text: String = a.status.iter().map(|s| s.text.as_str()).collect();
        let cols: Vec<usize> = {
            let mut v = Vec::with_capacity(text.len());
            let mut col = 0;
            for c in text.chars() {
                v.push(col);
                col += lif_render::char_width(c);
            }
            v
        };
        let chars: Vec<char> = text.chars().collect();
        let mut from = 0;
        for (idx, wid) in s.windows() {
            let w = self.model.window(wid)?;
            let label: Vec<char> = format!("{idx}:{}", w.name()).chars().collect();
            let pos = (from..chars.len().saturating_sub(label.len() - 1)).find(|&i| chars[i..].starts_with(&label))?;
            let start = cols[pos];
            let end = cols.get(pos + label.len()).copied().unwrap_or(usize::MAX);
            if (x as usize) >= start && (x as usize) < end + 1 {
                return Some(idx);
            }
            from = pos + label.len();
        }
        None
    }

    // ----- paste ---------------------------------------------------------

    fn paste_start(&mut self, id: ClientId) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        if a.prompt.is_some() {
            return;
        }
        let Some(pane) = a.view.as_ref().map(|v| v.active_pane) else { return };
        a.paste = Some(pane);
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let bracketed = p.engine.modes().bracketed_paste;
        p.paste_bracketed = Some(bracketed);
        if bracketed {
            self.rt.write_pane(pane, b"\x1b[200~");
        }
    }

    fn paste_data(&mut self, id: ClientId, data: &[u8]) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        if let Some(p) = a.prompt.as_mut() {
            p.insert(&String::from_utf8_lossy(data));
            a.needs_frame = true;
            return;
        }
        let Some(pane) = a.paste else { return };
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let mut b = std::mem::take(&mut self.replies);
        b.clear();
        p.engine.encode_paste_chunk(data, &mut b);
        self.rt.write_pane(pane, &b);
        self.replies = b;
        self.block_if_full(id, pane);
    }

    fn paste_end(&mut self, id: ClientId) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let Some(pane) = a.paste.take() else { return };
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        if p.paste_bracketed.take() == Some(true) {
            self.rt.write_pane(pane, b"\x1b[201~");
        }
    }

    // ----- terminal answers ------------------------------------------------

    fn response(&mut self, id: ClientId, r: TermResponse) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let was_complete = a.probe.is_complete();
        let done = a.probe.apply(&r);
        if let TermResponse::Clipboard { data, .. } = &r
            && let Some((pane, sel)) = a.clip_reads.pop_front()
            && let Some(p) = self.rt.panes.get_mut(&pane)
        {
            let mut b = Vec::new();
            p.engine.clipboard_reply(sel, data, &mut b);
            self.rt.write_pane(pane, &b);
            return;
        }
        if !done {
            return;
        }
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let Some(a) = c.att.as_mut() else { return };
        let mut caps = a.probe.caps();
        let mut out = Vec::new();
        if !was_complete {
            if caps.kitty_keyboard && !a.kitty_pushed {
                lif_input::write_kitty_upgrade(&mut out);
                a.kitty_pushed = true;
                a.parser.set_kitty_keyboard(true);
            }
            let fg = a.probe.foreground().map(|c| (c.r, c.g, c.b));
            let bg = a.probe.background().map(|c| (c.r, c.g, c.b));
            if (fg.is_some() || bg.is_some()) && (fg, bg) != self.rt.default_colors {
                self.rt.default_colors = (fg, bg);
                for p in self.rt.panes.values_mut() {
                    p.engine.set_default_colors(fg, bg, None);
                }
            }
        }
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let Some(a) = c.att.as_mut() else { return };
        caps.kitty_keyboard = a.kitty_pushed;
        if caps != a.renderer.caps() {
            a.renderer.set_caps(caps);
            a.renderer.invalidate();
            a.needs_frame = true;
        }
        let em = crate::client::emergency_bytes(a);
        crate::panic::set(std::os::fd::AsRawFd::as_raw_fd(&c.stream), em);
        if !out.is_empty() {
            c.send_output(&out);
            self.rt.flush_client(id);
        }
    }

    // ----- pane events -----------------------------------------------------

    pub(crate) fn pane_event(&mut self, pane: PaneId, ev: VtEvent) {
        match ev {
            VtEvent::Bell => self.model.pane_bell(pane, &mut self.rt),
            VtEvent::TitleChanged | VtEvent::PwdChanged => self.model.pane_changed(pane, &mut self.rt),
            VtEvent::ClipboardWrite { selection, data } => self.clipboard_write(pane, selection, &data, false),
            VtEvent::ClipboardRead { selection } => self.clipboard_read(pane, selection),
            VtEvent::Notification { title, body } => {
                let text = if title.is_empty() { body } else if body.is_empty() { title } else { format!("{title}: {body}") };
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                let seq = format!("\x1b]9;{text}\x1b\\");
                for c in self.clients_of_pane(pane) {
                    self.rt.send_output(c, seq.as_bytes());
                }
            }
            VtEvent::CommandStarted | VtEvent::CommandFinished { .. } => {}
        }
    }

    /// Attached clients whose session contains `pane`.
    fn clients_of_pane(&self, pane: PaneId) -> Vec<ClientId> {
        let Some(sid) = self.model.pane(pane).and_then(|p| self.model.window(p.window())).map(|w| w.session()) else {
            return Vec::new();
        };
        self.rt.clients.values().filter(|c| c.att.as_ref().is_some_and(|a| a.session == sid)).map(|c| c.id).collect()
    }

    /// OSC 52 from a pane (or a copy from a mode, `own`): forward to the
    /// outer terminals per `set-clipboard` and keep it as a paste buffer.
    fn clipboard_write(&mut self, pane: PaneId, selection: u8, data: &[u8], own: bool) {
        let mode = self.model.option_text("set-clipboard", None, None).unwrap_or_default();
        if mode == "off" && !own {
            return;
        }
        if mode == "on" || own {
            self.model.set_buffer(None, data.to_vec());
        }
        if mode == "off" {
            return;
        }
        let sel = if selection.is_ascii_alphanumeric() { selection as char } else { 'c' };
        let mut seq = format!("\x1b]52;{sel};");
        base64(data, &mut seq);
        seq.push_str("\x1b\\");
        for c in self.clients_of_pane(pane) {
            let ok = self.rt.clients.get(&c).and_then(|c| c.att.as_ref()).is_some_and(|a| a.renderer.caps().osc52_write);
            if ok {
                self.rt.send_output(c, seq.as_bytes());
            }
        }
    }

    fn clipboard_read(&mut self, pane: PaneId, selection: u8) {
        let reader = self.clients_of_pane(pane).into_iter().find(|c| {
            self.rt.clients.get(c).and_then(|c| c.att.as_ref()).is_some_and(|a| a.renderer.caps().osc52_read)
        });
        if let Some(c) = reader {
            if let Some(a) = self.rt.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                a.clip_reads.push_back((pane, selection));
                if a.clip_reads.len() > 16 {
                    a.clip_reads.pop_front();
                }
            }
            let sel = if selection.is_ascii_alphanumeric() { selection as char } else { 'c' };
            self.rt.send_output(c, format!("\x1b]52;{sel};?\x1b\\").as_bytes());
            return;
        }
        let data = self.model.buffers().first().map(|b| b.data.clone()).unwrap_or_default();
        if let Some(p) = self.rt.panes.get_mut(&pane) {
            let mut b = Vec::new();
            p.engine.clipboard_reply(selection, &data, &mut b);
            self.rt.write_pane(pane, &b);
        }
    }
}

fn border_drag(view: &lif_core::ClientView, x: u16, y: u16) -> Option<Drag> {
    let b = view.borders.iter().find(|r| r.contains(x, y))?;
    let vertical = b.cols == 1 && b.rows >= 1 && !(b.rows == 1 && b.cols == 1 && view.panes.iter().any(|(_, r)| r.bottom() == y));
    let pane = if vertical {
        view.panes.iter().find(|(_, r)| r.right() == x && y >= r.y && y < r.bottom())?.0
    } else {
        view.panes.iter().find(|(_, r)| r.bottom() == y && x >= r.x && x < r.right())?.0
    };
    Some(Drag { pane, vertical, last: if vertical { x } else { y } })
}

fn base64(data: &[u8], out: &mut String) {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        let enc = |s: &str| {
            let mut o = String::new();
            super::base64(s.as_bytes(), &mut o);
            o
        };
        assert_eq!(enc(""), "");
        assert_eq!(enc("f"), "Zg==");
        assert_eq!(enc("fo"), "Zm8=");
        assert_eq!(enc("foo"), "Zm9v");
        assert_eq!(enc("foobar"), "Zm9vYmFy");
    }
}
