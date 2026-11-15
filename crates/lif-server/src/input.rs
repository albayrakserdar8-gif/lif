//! What arrives from a client's terminal, and the events panes raise.

use std::time::Instant;

use lif_core::{ClientId, Host, KeyDecision, PaneId};
use lif_input::{InputEvent, Pending, TermResponse};
use lif_types::{Key, KeyAction, KeyEvent, MouseAction, MouseButton, MouseEvent, Mods, Pos};
use lif_vt::{VtEngine, VtEvent};

use crate::client::Drag;
use crate::mode::{ModeAction, ModeCtx, new_view_mode};
use crate::prompt::{PromptAction, PromptState};
use crate::server::Server;

/// Lines one wheel notch scrolls in a full-screen program that does not
/// use the mouse (sent as arrow keys).
const WHEEL_LINES: i32 = 3;

/// How often a copy-mode drag resting on a pane's edge scrolls (tmux's
/// rate).
pub(crate) const AUTOSCROLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

impl Server {
    pub(crate) fn client_input(&mut self, id: ClientId, bytes: &[u8], now: Instant) {
        let esc = self.rt.escape_time;
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let mut events = std::mem::take(&mut a.events);
        a.parser.feed(bytes, &mut events);
        // A lone ESC (legacy keyboard only) waits escape-time for the rest
        // of a sequence; 0 decides at once, as tmux does.
        if esc.is_zero() && a.parser.pending() == Pending::Ambiguous {
            a.parser.flush_pending(&mut events);
        }
        a.input_deadline = match a.parser.pending() {
            Pending::None => None,
            Pending::Ambiguous => Some(now + esc),
            Pending::Incomplete => Some(now + lif_input::DEFAULT_INCOMPLETE_TIMEOUT),
        };
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
            InputEvent::Mouse(m) => self.mouse(id, &m, now),
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

    /// A key while a popup or menu is up: it is theirs. Returns false if
    /// there is none.
    fn overlay_key(&mut self, id: ClientId, key: &KeyEvent) -> bool {
        use crate::overlay::Overlay;
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return false };
        let pressed = key.action != KeyAction::Release && !matches!(key.key, Key::Modifier(_));
        match a.overlay.as_mut() {
            None => false,
            Some(Overlay::Menu(m)) => {
                if pressed {
                    let outcome = m.state.key(key);
                    self.menu_outcome(id, outcome);
                }
                true
            }
            Some(Overlay::Popup(p)) => {
                let (pane, exited, any_key) = (p.pane, p.exited, p.any_key);
                if !exited {
                    self.key_to_pane(id, pane, key);
                } else if pressed {
                    // What the program left stays until Escape or C-c
                    // (any key with -k), as in tmux.
                    let ctrl_c = key.key == Key::Char('c') && key.mods.contains(Mods::CTRL);
                    if any_key || key.key == Key::Escape || ctrl_c {
                        self.rt.end_overlay(id, 0);
                        self.after_model();
                    }
                }
                true
            }
        }
    }

    /// Carry out what a key or click on `id`'s menu led to.
    fn menu_outcome(&mut self, id: ClientId, outcome: lif_core::MenuOutcome) {
        use crate::overlay::Overlay;
        use lif_core::MenuOutcome;
        match outcome {
            MenuOutcome::None => {}
            MenuOutcome::Redraw => {
                if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut())
                    && let Some(Overlay::Menu(m)) = a.overlay.as_mut()
                {
                    let s = &m.state;
                    m.frame = crate::overlay::menu_frame(s, lif_core::parse_style(&s.style), crate::host::menu_selected(s), crate::host::menu_border(s));
                    a.needs_frame = true;
                }
            }
            MenuOutcome::Close | MenuOutcome::Run(_) => {
                let pane = match self.rt.clients.get(&id).and_then(|c| c.att.as_ref()).and_then(|a| a.overlay.as_ref()) {
                    Some(Overlay::Menu(m)) => m.state.pane,
                    _ => None,
                };
                self.rt.end_overlay(id, 0);
                if let MenuOutcome::Run(command) = outcome
                    && !command.trim().is_empty()
                {
                    // The item's command, for the pane the menu was
                    // opened for.
                    let r = lif_core::parse_string(&command).and_then(|list| self.run_commands_in(Some(id), pane, &list));
                    match r {
                        Ok(o) => self.show_output(id, &o.stdout),
                        Err(e) => {
                            let d = self.display_time();
                            self.rt.show_message(id, &e.to_string(), d);
                        }
                    }
                }
                self.sync_views();
            }
        }
    }

    /// The mouse while a popup or menu is up. Returns false if there is
    /// none.
    fn overlay_mouse(&mut self, id: ClientId, m: &MouseEvent) -> bool {
        use crate::overlay::Overlay;
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return false };
        match a.overlay.as_mut() {
            None => false,
            Some(Overlay::Menu(menu)) => {
                let click = m.action == MouseAction::Press && m.button == Some(MouseButton::Left);
                let moved = m.action == MouseAction::Motion;
                if click || moved {
                    let outcome = menu.state.mouse(m.cell.x, m.cell.y, click);
                    self.menu_outcome(id, outcome);
                }
                true
            }
            Some(Overlay::Popup(p)) => {
                let (pane, r) = (p.pane, p.inner);
                let inside = m.cell.x >= r.x && m.cell.x < r.right() && m.cell.y >= r.y && m.cell.y < r.bottom();
                // Its program gets the mouse if it asked for it; clicks
                // beside the popup do nothing.
                if inside
                    && let Some(pr) = self.rt.panes.get_mut(&pane)
                    && pr.engine.modes().mouse_tracking
                {
                    let local = MouseEvent { cell: Pos { x: m.cell.x - r.x, y: m.cell.y - r.y }, ..*m };
                    let mut b = Vec::new();
                    pr.engine.encode_mouse(&local, &mut b);
                    self.rt.write_input(pane, &b);
                }
                true
            }
        }
    }

    fn key(&mut self, id: ClientId, key: &KeyEvent, now: Instant) {
        if self.overlay_key(id, key) {
            return;
        }
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
                PromptAction::CopyChanged(pane, text) => {
                    if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                        a.prompt = Some(p);
                        a.needs_frame = true;
                    }
                    let act = self.with_mode(pane, |m, ctx| m.prompt_changed(&text, ctx));
                    self.mode_action(Some(id), pane, act);
                }
                PromptAction::CopyAgain(pane, forward) => {
                    if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                        a.prompt = Some(p);
                        a.needs_frame = true;
                    }
                    let act = self.with_mode(pane, |m, ctx| m.prompt_again(forward, ctx));
                    self.mode_action(Some(id), pane, act);
                }
                PromptAction::CopyAccept(pane, text) => {
                    self.rt.needs_frame(id);
                    let act = self.with_mode(pane, |m, ctx| m.prompt_accept(&text, ctx));
                    self.mode_action(Some(id), pane, act);
                }
                PromptAction::CopyCancel(pane) => {
                    self.rt.needs_frame(id);
                    let act = self.with_mode(pane, |m, ctx| m.prompt_cancel(ctx));
                    self.mode_action(Some(id), pane, act);
                }
            }
            return;
        }
        let pressed = key.action != KeyAction::Release && !matches!(key.key, Key::Modifier(_));
        if let Some(dp) = a.display_panes.as_ref()
            && pressed
            && dp.keys
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
        if self.rt.panes.get(&pane).is_some_and(|p| p.mode.is_some()) {
            self.key_to_mode(client, pane, key);
            return;
        }
        // select-pane -d: nothing typed reaches the program.
        if self.model.pane_input_off(pane) {
            return;
        }
        // synchronize-panes: the window's other panes get the key too,
        // each encoded for its own program. A full input queue there
        // drops it without a word; the pane typed into reports its own.
        let mut others = std::mem::take(&mut self.scratch_panes);
        others.clear();
        self.model.synchronized_panes(pane, &mut others);
        for &o in &others {
            let Some(p) = self.rt.panes.get_mut(&o).filter(|p| p.mode.is_none()) else { continue };
            let mut b = std::mem::take(&mut self.replies);
            b.clear();
            p.engine.encode_key(key, &mut b);
            self.rt.write_input(o, &b);
            self.replies = b;
        }
        self.scratch_panes = others;
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let mut b = std::mem::take(&mut self.replies);
        b.clear();
        p.engine.encode_key(key, &mut b);
        if !self.rt.write_input(pane, &b) {
            self.input_dropped(client, pane, b.len());
        }
        self.replies = b;
    }

    /// Typed or pasted input did not fit the pane's input queue (its
    /// program is not reading): it is dropped, as a terminal with a full
    /// input queue drops it, and the user is told. The client is never
    /// stopped: its other keys keep working.
    fn input_dropped(&mut self, client: ClientId, pane: PaneId, bytes: usize) {
        log!("pane {pane}: input queue full, {bytes} bytes from client {client} dropped");
        let d = self.display_time();
        let text = format!("pane {pane} is not reading its input: input dropped");
        self.rt.show_message(client, &text, d);
    }

    /// A key for a pane in a mode: through the copy-mode key table (the
    /// model's `copy-mode` or `copy-mode-vi` table, per `mode-keys`).
    /// Bindings copy mode understands go to the mode; others run as
    /// commands, as tmux runs them.
    fn key_to_mode(&mut self, client: ClientId, pane: PaneId, key: &KeyEvent) {
        if key.action == KeyAction::Release || matches!(key.key, Key::Modifier(_)) {
            return;
        }
        let vi = self.sync_mode_config(pane);
        let Some(p) = self.rt.panes.get(&pane) else { return };
        let Some(m) = p.mode.as_deref() else { return };
        let (pending, view) = (m.pending(), m.is_view());
        if view && !pending && (matches!(key.key, Key::Escape | Key::Char('q')) && key.mods.is_empty()) {
            self.mode_action(Some(client), pane, ModeAction::Exit);
            return;
        }
        if !pending {
            let table = if vi { lif_core::bindings::COPY_MODE_VI_TABLE } else { lif_core::bindings::COPY_MODE_TABLE };
            let run = self
                .model
                .bindings()
                .lookup(table, key)
                .filter(|b| lif_copy::Action::from_commands(&b.commands).is_none())
                .map(|b| b.commands.clone());
            if let Some(list) = run {
                if let Err(e) = self.run_commands(Some(client), &list).map(|o| self.show_output(client, &o.stdout)) {
                    let d = self.display_time();
                    self.rt.show_message(client, &e.to_string(), d);
                }
                return;
            }
        }
        let act = self.with_mode(pane, |m, ctx| m.key(key, ctx));
        if act == ModeAction::Binding {
            self.run_mode_binding(Some(client), pane, key);
            return;
        }
        self.mode_action(Some(client), pane, act);
    }

    /// Run the commands `key` is bound to in `pane`'s copy-mode table, with
    /// `pane` as the current pane. Output and errors go to `client`'s
    /// status line (or the log, for `send-keys` from no client).
    pub(crate) fn run_mode_binding(&mut self, client: Option<ClientId>, pane: PaneId, key: &KeyEvent) {
        let vi = self.sync_mode_config(pane);
        let table = if vi { lif_core::bindings::COPY_MODE_VI_TABLE } else { lif_core::bindings::COPY_MODE_TABLE };
        let Some(list) = self.model.bindings().lookup(table, key).map(|b| b.commands.clone()) else { return };
        let r = self.run_commands_in(client, Some(pane), &list);
        match (client, r) {
            (Some(c), Ok(o)) => self.show_output(c, &o.stdout),
            (Some(c), Err(e)) => {
                let d = self.display_time();
                self.rt.show_message(c, &e.to_string(), d);
            }
            (None, Ok(_)) => {}
            (None, Err(e)) => log!("copy-mode binding in pane {pane}: {e}"),
        }
    }

    /// Run `f` on `pane`'s mode; `ModeAction::None` without one.
    pub(crate) fn with_mode(
        &mut self,
        pane: PaneId,
        f: impl FnOnce(&mut dyn crate::mode::PaneMode, &mut ModeCtx<'_>) -> ModeAction,
    ) -> ModeAction {
        self.rt.panes.get_mut(&pane).and_then(|p| p.with_mode(f)).unwrap_or(ModeAction::None)
    }

    /// Keep copy mode's settings (key table, mode-keys, word separators,
    /// wrap-search) in step with the options of `pane`'s window and give
    /// them to its mode when they changed. Returns whether keys are vi.
    pub(crate) fn sync_mode_config(&mut self, pane: PaneId) -> bool {
        let w = self.model.pane(pane).map(|p| p.window());
        let s = w.and_then(|w| self.model.window(w)).map(|w| w.session());
        let opt = |n: &str| self.model.option_text(n, s, w).unwrap_or_default();
        let vi = opt("mode-keys") == "vi";
        let drag_end = match opt("mouse-drag-end").as_str() {
            "copy-pipe-no-clear" => lif_copy::DragEnd::CopyAndStay,
            "none" => lif_copy::DragEnd::Keep,
            _ => lif_copy::DragEnd::CopyAndCancel,
        };
        let key = (self.model.bindings().generation(), vi, opt("word-separators"), opt("wrap-search") != "off", drag_end);
        if self.rt.copy_cfg.key != key {
            let keys = if vi { lif_copy::ModeKeys::Vi } else { lif_copy::ModeKeys::Emacs };
            let mut cfg = lif_copy::Config::new(keys);
            cfg.table = std::rc::Rc::new(lif_copy::KeyTable::from_bindings(self.model.bindings(), keys.table_name()));
            cfg.word_separators.clone_from(&key.2);
            cfg.wrap_search = key.3;
            cfg.mouse.drag_end = drag_end;
            self.rt.copy_cfg = crate::host::CopyConfig { key, cfg };
        }
        let gen_key = &self.rt.copy_cfg.key;
        if let Some(p) = self.rt.panes.get_mut(&pane)
            && p.mode.is_some()
            && p.mode_cfg.as_ref() != Some(gen_key)
        {
            p.mode_cfg = Some(gen_key.clone());
            let cfg = &self.rt.copy_cfg.cfg;
            p.with_mode(|m, ctx| m.set_config(cfg, ctx));
        }
        vi
    }

    /// Set up the choosers commands asked for, and give the open ones a
    /// list that matches the model again. Called after every model call;
    /// costs two empty checks when no chooser is open.
    pub(crate) fn sync_choosers(&mut self) {
        if !self.rt.pending_choose.is_empty() {
            for (client, pane, spec) in std::mem::take(&mut self.rt.pending_choose) {
                let now = crate::sys::wall_clock();
                let items = self.model.choose_items(&spec, Some(client), now, &self.rt);
                let start = self.model.choose_start(spec.kind, client);
                let zoomed = spec.zoomed;
                let state = lif_core::ChooseState::new(spec, items, start.as_deref());
                let Some(p) = self.rt.panes.get_mut(&pane) else { continue };
                if state.is_empty() {
                    // Nothing to choose from (no buffers): tmux opens an
                    // empty mode; lif does not open one.
                    self.model.chooser_closed(pane, zoomed, &mut self.rt);
                    continue;
                }
                p.leave_mode();
                p.mode = Some(Box::new(crate::choose::ChoosePane::new(state, client, p.engine.size())));
                if !self.rt.chooser_panes.contains(&pane) {
                    self.rt.chooser_panes.push(pane);
                }
                self.model.pane_changed(pane, &mut self.rt);
                self.rt.views_stale = true;
                self.rt.mark_all(true);
            }
        }
        if self.rt.chooser_panes.is_empty() {
            return;
        }
        let panes = std::mem::take(&mut self.rt.chooser_panes);
        for pane in &panes {
            self.rebuild_chooser(*pane);
        }
        self.rt.chooser_panes = panes;
        let rt = &mut self.rt;
        rt.chooser_panes.retain(|p| rt.panes.get_mut(p).is_some_and(|p| p.mode.as_deref_mut().is_some_and(|m| m.chooser().is_some())));
    }

    /// A new list for the chooser in `pane`, if it has one; a chooser
    /// with nothing left to list ends.
    fn rebuild_chooser(&mut self, pane: PaneId) {
        let Some(c) = self.rt.panes.get_mut(&pane).and_then(|p| p.mode.as_deref_mut()?.chooser()) else { return };
        let (spec, client) = (c.state.spec().clone(), c.client);
        let items = self.model.choose_items(&spec, Some(client), crate::sys::wall_clock(), &self.rt);
        let Some(c) = self.rt.panes.get_mut(&pane).and_then(|p| p.mode.as_deref_mut()?.chooser()) else { return };
        c.state.set_items(items);
        c.mark_dirty();
        if c.state.is_empty() {
            self.mode_action(None, pane, ModeAction::Exit);
        }
    }

    /// Draw the choosers whose state or size changed, before a frame.
    pub(crate) fn refresh_choosers(&mut self) {
        if self.rt.chooser_panes.is_empty() {
            return;
        }
        for i in 0..self.rt.chooser_panes.len() {
            let pane = self.rt.chooser_panes[i];
            let Some(p) = self.rt.panes.get_mut(&pane) else { continue };
            let size = p.engine.size();
            if !p.mode.as_deref_mut().and_then(|m| m.chooser()).is_some_and(|c| c.needs_draw(size)) {
                continue;
            }
            let selected = lif_core::parse_style(&self.model.pane_option_text(pane, "mode-style").unwrap_or_default());
            // The mode comes out of its pane while it draws: the preview
            // reads the other panes.
            let Some(mut mode) = self.rt.panes.get_mut(&pane).and_then(|p| p.mode.take()) else { continue };
            if let Some(c) = mode.chooser() {
                c.draw(size, &self.rt.panes, selected);
            }
            if let Some(p) = self.rt.panes.get_mut(&pane) {
                p.mode = Some(mode);
            }
            self.rt.mark_all(false);
        }
    }

    /// Carry out what a mode asked for. `client` is who caused it, if
    /// anyone: prompts and messages go there (or to every client showing
    /// the pane).
    pub(crate) fn mode_action(&mut self, client: Option<ClientId>, pane: PaneId, action: ModeAction) {
        match action {
            ModeAction::None | ModeAction::Redraw => {}
            ModeAction::Exit => {
                let mut zoomed = None;
                if let Some(p) = self.rt.panes.get_mut(&pane) {
                    zoomed = p.mode.as_deref_mut().and_then(|m| m.chooser()).map(|c| c.state.spec().zoomed);
                    p.leave_mode();
                }
                if let Some(z) = zoomed {
                    self.model.chooser_closed(pane, z, &mut self.rt);
                    self.rt.views_stale = true;
                    self.rt.mark_all(true);
                }
                for c in self.rt.clients.values_mut() {
                    if let Some(a) = c.att.as_mut() {
                        if a.prompt.as_ref().and_then(|p| p.copy_pane()) == Some(pane) {
                            a.prompt = None;
                        }
                        if a.mode_drag.is_some_and(|(p, _)| p == pane) {
                            a.mode_drag = None;
                        }
                    }
                }
                self.model.pane_changed(pane, &mut self.rt);
            }
            ModeAction::Copy { data, append, exit, pipe } => {
                let data = if append && let Some(top) = self.model.buffers().first() {
                    let (name, mut all) = (top.name.clone(), top.data.clone());
                    all.extend_from_slice(&data);
                    self.model.set_buffer(Some(&name), all.clone());
                    all
                } else {
                    self.model.set_buffer(None, data.clone());
                    data
                };
                self.clipboard_write(b'c', &data, true);
                let command = match pipe {
                    Some(c) if c.is_empty() => self.model.option_text("copy-command", None, None).unwrap_or_default(),
                    Some(c) => c,
                    None => String::new(),
                };
                if !command.trim().is_empty() {
                    let cwd = self.rt.panes.get(&pane).map(|p| p.current_path.clone()).unwrap_or_default();
                    if let Err(e) = self.rt.copy_pipe(&command, &cwd, data) {
                        log!("{e}");
                        let d = self.display_time();
                        for c in self.mode_clients(client, pane) {
                            self.rt.show_message(c, &e, d);
                        }
                    }
                }
                if exit {
                    self.mode_action(client, pane, ModeAction::Exit);
                }
            }
            // Only a key knows its binding: key_to_mode and send-keys run it.
            ModeAction::Binding => {}
            ModeAction::Prompt { label, initial, incremental } => {
                for c in self.mode_clients(client, pane) {
                    if let Some(a) = self.rt.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                        a.prompt = Some(PromptState::copy(pane, label, initial.clone(), incremental));
                        a.needs_frame = true;
                    }
                }
            }
            ModeAction::Message(text) => {
                let d = self.display_time();
                for c in self.mode_clients(client, pane) {
                    self.rt.show_message(c, &text, d);
                }
            }
            ModeAction::Searching => {}
            ModeAction::Run { commands, exit } => {
                // A chooser's choices are its client's commands, whoever
                // pressed the key.
                let owner = self.rt.panes.get_mut(&pane).and_then(|p| p.mode.as_deref_mut()?.chooser().map(|c| c.client));
                if exit {
                    self.mode_action(client, pane, ModeAction::Exit);
                }
                let who = owner.or(client).filter(|c| self.rt.clients.get(c).is_some_and(|c| c.att.is_some()));
                if let Some(c) = who {
                    for command in &commands {
                        self.run_string(c, command);
                    }
                }
                return;
            }
            ModeAction::Ask { label, single_key } => {
                for c in self.mode_clients(client, pane) {
                    if let Some(a) = self.rt.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                        a.prompt = Some(PromptState::ask(pane, &label, single_key));
                        a.needs_frame = true;
                    }
                }
            }
            ModeAction::Rebuild => self.rebuild_chooser(pane),
        }
        if let Some(p) = self.rt.panes.get_mut(&pane) {
            let (searching, scroll) = p.mode.as_deref().map_or((false, false), |m| (m.searching(), m.autoscroll_active()));
            if searching && !p.searching {
                p.search_client = client;
            }
            p.searching = searching;
            if !scroll {
                p.autoscroll_at = None;
            } else if p.autoscroll_at.is_none() {
                p.autoscroll_at = Some(std::time::Instant::now() + AUTOSCROLL_INTERVAL);
            }
        }
        self.rt.mark_all(false);
    }

    /// Who sees a mode's prompts and messages: the client that caused them,
    /// else every client attached to the pane's session.
    fn mode_clients(&self, client: Option<ClientId>, pane: PaneId) -> Vec<ClientId> {
        match client {
            Some(c) if self.rt.clients.get(&c).is_some_and(|c| c.att.is_some()) => vec![c],
            _ => self.clients_of_pane(pane),
        }
    }

    pub(crate) fn display_time(&self) -> u64 {
        self.model.option_text("display-time", None, None).and_then(|v| v.parse().ok()).unwrap_or(750)
    }

    /// Output of a command a client ran from a key or the prompt: one line
    /// is shown as a message; more open view mode in the active pane, a
    /// read-only pager (tmux's view mode) left with q or Escape.
    pub(crate) fn show_output(&mut self, id: ClientId, out: &str) {
        let text = out.trim_end();
        if text.is_empty() {
            return;
        }
        if !text.contains('\n') {
            let d = self.display_time();
            self.rt.show_message(id, text, d);
            return;
        }
        let Some(pane) = self.active_pane(id) else { return };
        self.sync_mode_config(pane);
        let cfg = self.rt.copy_cfg.cfg.clone();
        let key = self.rt.copy_cfg.key.clone();
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        p.leave_mode();
        p.mode = Some(new_view_mode(text.to_string(), &cfg, &mut ModeCtx { engine: &mut p.engine }));
        p.mode_cfg = Some(key);
        self.model.pane_changed(pane, &mut self.rt);
        self.rt.mark_all(false);
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

    fn mouse(&mut self, id: ClientId, m: &MouseEvent, now: std::time::Instant) {
        if self.overlay_mouse(id, m) {
            return;
        }
        let now_ms = self.now_ms(now);
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        let (x, y) = (m.cell.x, m.cell.y);
        if let Some((pane, rect)) = a.mode_drag {
            // The pane in a mode that took the press gets the whole drag,
            // clamped to its edges (where it scrolls).
            if m.action == MouseAction::Release {
                a.mode_drag = None;
            }
            let cell = Pos {
                x: x.clamp(rect.x, rect.right().saturating_sub(1)) - rect.x,
                y: y.clamp(rect.y, rect.bottom().saturating_sub(1)) - rect.y,
            };
            let local = MouseEvent { cell, ..*m };
            self.sync_mode_config(pane);
            let act = self.with_mode(pane, |md, ctx| md.mouse(&local, now_ms, ctx));
            self.mode_action(Some(id), pane, act);
            return;
        }
        let Some(view) = a.view.as_ref() else { return };
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
            self.rt.write_input(pane, &b);
            if mouse_opt && m.action == MouseAction::Press && pane != active {
                self.run_string(id, &format!("select-pane -t {pane}"));
            }
            return;
        }
        if !mouse_opt {
            return;
        }
        let local = MouseEvent { cell: Pos { x: x - rect.x, y: y - rect.y }, ..*m };
        let wheel = matches!(m.button, Some(MouseButton::WheelUp | MouseButton::WheelDown));
        if p.mode.is_some() {
            if m.action == MouseAction::Press && pane != active && !wheel {
                self.run_string(id, &format!("select-pane -t {pane}"));
            }
            if m.action == MouseAction::Press
                && m.button == Some(MouseButton::Left)
                && let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut())
            {
                a.mode_drag = Some((pane, rect));
            }
            self.sync_mode_config(pane);
            let act = self.with_mode(pane, |md, ctx| md.mouse(&local, now_ms, ctx));
            self.mode_action(Some(id), pane, act);
            return;
        }
        let click_ms = self.rt.copy_cfg.cfg.mouse.click_ms;
        let press = match self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            Some(a) if m.action == MouseAction::Press && m.button == Some(MouseButton::Left) => {
                let n = match a.clicks {
                    Some((p, at, t, n)) if p == pane && at == local.cell && now_ms.saturating_sub(t) <= click_ms => n % 3 + 1,
                    _ => 1,
                };
                a.clicks = Some((pane, local.cell, now_ms, n));
                if n > 1 {
                    a.mouse_press = None;
                    if pane != active {
                        self.run_string(id, &format!("select-pane -t {pane}"));
                    }
                    self.click_select(id, pane, &local, n, now_ms);
                    return;
                }
                a.mouse_press = Some((pane, rect, local));
                None
            }
            Some(a) => a.mouse_press.take(),
            None => None,
        };
        match (m.action, m.button) {
            (MouseAction::Press, Some(MouseButton::Left)) if pane != active => {
                self.run_string(id, &format!("select-pane -t {pane}"));
            }
            (MouseAction::Motion, Some(MouseButton::Left)) if !modes.alt_screen => {
                // tmux's MouseDrag1Pane: a drag starts copy mode, selecting
                // from where the button went down.
                let Some((pp, prect, pev)) = press.filter(|(p, _, _)| *p == pane) else { return };
                self.sync_mode_config(pp);
                let opts = lif_core::CopyModeOptions { mouse_drag: true, ..Default::default() };
                let Some(p) = self.rt.panes.get_mut(&pp) else { return };
                crate::host::enter_copy(p, opts, &self.rt.copy_cfg.cfg);
                p.mode_cfg = Some(self.rt.copy_cfg.key.clone());
                self.model.pane_changed(pp, &mut self.rt);
                if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    a.mode_drag = Some((pp, prect));
                }
                let act = self.with_mode(pp, |md, ctx| {
                    md.mouse(&pev, now_ms, ctx);
                    md.mouse(&local, now_ms, ctx)
                });
                self.mode_action(Some(id), pp, act);
            }
            (MouseAction::Press, Some(b @ (MouseButton::WheelUp | MouseButton::WheelDown))) => {
                let up = b == MouseButton::WheelUp;
                if modes.alt_screen {
                    // A full-screen program without mouse support: arrow
                    // keys, as terminals do in alternate-scroll mode.
                    let k = KeyEvent::new(if up { Key::Up } else { Key::Down }, Mods::NONE);
                    let mut bytes = Vec::new();
                    for _ in 0..WHEEL_LINES {
                        p.engine.encode_key(&k, &mut bytes);
                    }
                    self.rt.write_input(pane, &bytes);
                    return;
                }
                if !up {
                    return;
                }
                // tmux's default WheelUpPane: copy-mode -e, then scroll.
                self.sync_mode_config(pane);
                let opts = lif_core::CopyModeOptions { exit_at_bottom: true, ..Default::default() };
                let Some(p) = self.rt.panes.get_mut(&pane) else { return };
                crate::host::enter_copy(p, opts, &self.rt.copy_cfg.cfg);
                p.mode_cfg = Some(self.rt.copy_cfg.key.clone());
                self.model.pane_changed(pane, &mut self.rt);
                let act = self.with_mode(pane, |md, ctx| md.mouse(&local, now_ms, ctx));
                self.mode_action(Some(id), pane, act);
            }
            _ => {}
        }
    }

    /// tmux's DoubleClick1Pane and TripleClick1Pane on a pane not in a mode
    /// whose program does not take the mouse: copy mode (`-H`), select the
    /// word or line under the pointer and copy it per `mouse-drag-end`
    /// (tmux's `copy-pipe-and-cancel`). tmux shows the selection for 0.3 s
    /// before it copies and leaves; lif copies at once.
    fn click_select(&mut self, id: ClientId, pane: PaneId, press: &MouseEvent, clicks: u8, now_ms: u64) {
        self.sync_mode_config(pane);
        let opts = lif_core::CopyModeOptions { hide_position: true, ..Default::default() };
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        crate::host::enter_copy(p, opts, &self.rt.copy_cfg.cfg);
        p.mode_cfg = Some(self.rt.copy_cfg.key.clone());
        self.model.pane_changed(pane, &mut self.rt);
        // The mode counts the clicks itself: give it all of them.
        let mut act = self.with_mode(pane, |md, ctx| {
            let mut last = ModeAction::None;
            for _ in 0..clicks {
                last = md.mouse(press, now_ms, ctx);
            }
            last
        });
        // Nothing to copy (a blank line): tmux's copy-pipe-and-cancel still
        // leaves.
        if !matches!(act, ModeAction::Copy { .. }) && self.rt.copy_cfg.cfg.mouse.drag_end == lif_copy::DragEnd::CopyAndCancel {
            act = ModeAction::Exit;
        }
        self.mode_action(Some(id), pane, act);
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
        // A paste goes to a popup's program; a menu takes none.
        let over = match a.overlay.as_ref() {
            Some(crate::overlay::Overlay::Popup(p)) => Some((!p.exited).then_some(p.pane)),
            Some(crate::overlay::Overlay::Menu(_)) => Some(None),
            None => None,
        };
        let pane = match over {
            Some(None) => {
                a.paste = None;
                return;
            }
            Some(Some(p)) => p,
            None => match a.view.as_ref().map(|v| v.active_pane) {
                Some(p) => p,
                None => return,
            },
        };
        if self.model.pane_input_off(pane) {
            a.paste = None;
            return;
        }
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        a.paste = Some(pane);
        a.paste_dropping = false;
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let bracketed = p.engine.modes().bracketed_paste;
        // A start that did not fit is not ended either.
        let started = !bracketed || self.rt.write_input(pane, b"\x1b[200~");
        if let Some(p) = self.rt.panes.get_mut(&pane) {
            p.paste_bracketed = Some(bracketed && started);
        }
        if !started {
            if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.paste_dropping = true;
            }
            self.input_dropped(id, pane, 6);
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
        if a.paste_dropping {
            return;
        }
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        let mut b = std::mem::take(&mut self.replies);
        b.clear();
        p.engine.encode_paste_chunk(data, &mut b);
        let queued = self.rt.write_input(pane, &b);
        // synchronize-panes: each other pane gets this part as a paste of
        // its own (bracketed if its program asked for that).
        let mut others = std::mem::take(&mut self.scratch_panes);
        others.clear();
        self.model.synchronized_panes(pane, &mut others);
        for &o in &others {
            let Some(p) = self.rt.panes.get_mut(&o).filter(|p| p.mode.is_none()) else { continue };
            b.clear();
            p.engine.encode_paste(data, &mut b);
            self.rt.write_input(o, &b);
        }
        self.scratch_panes = others;
        self.replies = b;
        if !queued {
            if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.paste_dropping = true;
            }
            self.input_dropped(id, pane, data.len());
        }
    }

    fn paste_end(&mut self, id: ClientId) {
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        a.paste_dropping = false;
        let Some(pane) = a.paste.take() else { return };
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        if p.paste_bracketed.take() == Some(true) {
            self.rt.write_pane_closing(pane, b"\x1b[201~");
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
            // Usually right after the first frame of an attach: the repaint
            // with the real capabilities is not output to coalesce, so it
            // goes out at once instead of a pacing interval later.
            a.last_frame = None;
        }
        c.arm_panic();
        if !out.is_empty() {
            c.send_output(&out);
            self.rt.flush_client(id);
        }
    }

    // ----- pane events -----------------------------------------------------

    pub(crate) fn pane_event(&mut self, pane: PaneId, ev: VtEvent) {
        match ev {
            VtEvent::Bell => self.model.pane_bell(pane, &mut self.rt),
            VtEvent::TitleChanged => {
                self.model.pane_title_reset(pane);
                self.model.pane_changed(pane, &mut self.rt);
            }
            VtEvent::PwdChanged => {
                if let Some(p) = self.rt.panes.get_mut(&pane) {
                    p.osc7_path = crate::pane::file_url_path(p.engine.pwd());
                }
                self.model.pane_changed(pane, &mut self.rt);
            }
            VtEvent::ClipboardWrite { selection, data } => self.clipboard_write(selection, &data, false),
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

    /// OSC 52 from a pane, or a copy from a mode (`own`, already stored
    /// as a paste buffer): forward it to the outer terminals unless
    /// `set-clipboard` is off, and keep a pane's as a paste buffer when it
    /// is on.
    fn clipboard_write(&mut self, selection: u8, data: &[u8], own: bool) {
        let mode = self.model.option_text("set-clipboard", None, None).unwrap_or_default();
        if mode == "off" && !own {
            return;
        }
        if mode == "on" && !own {
            self.model.set_buffer(None, data.to_vec());
        }
        if mode == "off" {
            return;
        }
        let sel = if selection.is_ascii_alphanumeric() { selection as char } else { 'c' };
        let mut seq = format!("\x1b]52;{sel};");
        base64(data, &mut seq);
        seq.push_str("\x1b\\");
        // Every attached terminal gets it, whether or not it was recognized
        // as supporting OSC 52: one that does not ignores the sequence, and
        // detection misses many that do (a plain xterm-256color TERM).
        let attached: Vec<ClientId> = self.rt.clients.values().filter(|c| c.att.is_some()).map(|c| c.id).collect();
        for c in attached {
            self.rt.send_output(c, seq.as_bytes());
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

pub(crate) fn base64(data: &[u8], out: &mut String) {
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
