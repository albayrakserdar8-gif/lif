//! Model tests. Host calls are recorded by `TestHost`; expectations marked
//! "tmux 3.8" were checked against tmux 3.8 on a private socket
//! (`tmux -L lifcore-ref -f /dev/null`).

use crate::*;
use lif_types::{Key, KeyEvent, Mods, Size};
use std::collections::HashMap;

const NOW: WallClock = WallClock::new(1_794_682_220, 3 * 3600);

#[derive(Default)]
struct TestHost {
    spawned: Vec<SpawnSpec>,
    killed: Vec<PaneId>,
    resized: HashMap<PaneId, Size>,
    sent: Vec<(PaneId, String)>,
    fail_spawn: bool,
    commands: HashMap<PaneId, String>,
    paths: HashMap<PaneId, String>,
    titles: HashMap<PaneId, String>,
    attached: Vec<(ClientId, SessionId)>,
    detached: Vec<(ClientId, DetachReason)>,
    messages: Vec<(ClientId, String)>,
    prompts: Vec<(ClientId, Prompt)>,
    copy_mode: Vec<PaneId>,
    bells: Vec<ClientId>,
    captures: Vec<CaptureRange>,
    shutdown: bool,
    /// Panes in a mode, and the `send-keys -X` calls they got.
    in_mode: Vec<PaneId>,
    mode_commands: Vec<(PaneId, String, Vec<String>, u32)>,
    /// What `read_config` returns, by path.
    configs: HashMap<String, ConfigFile>,
    /// Clients whose parent process got SIGHUP.
    hangups: Vec<ClientId>,
    /// spawn_pane and kill_pane calls in order, for the fuzz test.
    events: Vec<ProcEvent>,
    /// What `command_stdin` returns (taken once).
    stdin: Option<Vec<u8>>,
    /// The files `read_file` and `write_file` see.
    files: HashMap<String, Vec<u8>>,
    /// Data given to `set_clipboard`.
    clipboard: Vec<Vec<u8>>,
    /// Shell commands and timers (id, milliseconds) the model started.
    jobs: Vec<JobSpec>,
    timers: Vec<(u64, u64)>,
    fail_jobs: bool,
    /// Panes whose history was cleared.
    cleared: Vec<PaneId>,
    /// Choosers asked for.
    choosers: Vec<(ClientId, PaneId, ChooseSpec)>,
    /// `pipe-pane` commands, by pane, and every start and stop in order.
    pipes: HashMap<PaneId, String>,
    pipe_calls: Vec<(PaneId, Option<String>)>,
    /// Popups and menus opened, and whether a client has one up.
    popups: Vec<(ClientId, PopupSpec)>,
    menus: Vec<(ClientId, MenuState)>,
    overlay_up: bool,
    overlays_closed: u32,
    /// Panes put into clock mode.
    clocks: Vec<PaneId>,
    history: Vec<String>,
}

impl Host for TestHost {
    fn spawn_pane(&mut self, spec: &SpawnSpec) -> Result<(), String> {
        if self.fail_spawn {
            return Err("fork failed: Resource temporarily unavailable".into());
        }
        self.spawned.push(spec.clone());
        self.events.push(ProcEvent::Spawn { pane: spec.pane, respawn: spec.respawn });
        self.resized.insert(spec.pane, spec.size);
        Ok(())
    }
    fn start_job(&mut self, job: &JobSpec) -> Result<(), String> {
        if self.fail_jobs {
            return Err(format!("{}: fork failed", job.command));
        }
        self.jobs.push(job.clone());
        Ok(())
    }
    fn start_timer(&mut self, id: u64, delay_ms: u64) -> Result<(), String> {
        self.timers.push((id, delay_ms));
        Ok(())
    }
    fn clear_history(&mut self, pane: PaneId) {
        self.cleared.push(pane);
    }
    fn clock_mode(&mut self, pane: PaneId) {
        self.clocks.push(pane);
    }
    fn popup(&mut self, client: ClientId, spec: PopupSpec) -> Result<(), String> {
        self.popups.push((client, spec));
        Ok(())
    }
    fn menu(&mut self, client: ClientId, menu: MenuState) -> Result<(), String> {
        self.menus.push((client, menu));
        Ok(())
    }
    fn has_overlay(&self, _client: ClientId) -> bool {
        self.overlay_up
    }
    fn close_overlay(&mut self, _client: ClientId) {
        self.overlays_closed += 1;
    }
    fn pipe_pane(&mut self, pane: PaneId, command: Option<&str>, _cwd: &str) -> Result<(), String> {
        self.pipe_calls.push((pane, command.map(str::to_string)));
        match command {
            Some(c) => self.pipes.insert(pane, c.to_string()),
            None => self.pipes.remove(&pane),
        };
        Ok(())
    }
    fn prompt_history(&mut self, clear: bool) -> Vec<String> {
        if clear { std::mem::take(&mut self.history) } else { self.history.clone() }
    }
    fn choose(&mut self, client: ClientId, pane: PaneId, spec: ChooseSpec) -> Result<(), String> {
        self.choosers.push((client, pane, spec));
        Ok(())
    }
    fn kill_pane(&mut self, pane: PaneId) {
        self.killed.push(pane);
        self.events.push(ProcEvent::Kill(pane));
    }
    fn resize_pane(&mut self, pane: PaneId, size: Size) {
        self.resized.insert(pane, size);
    }
    fn send_to_pane(&mut self, pane: PaneId, input: PaneInput<'_>) -> Result<(), String> {
        let s = match input {
            PaneInput::Key(k) => format!("key:{}", KeyCode::from_event(k).map(|k| k.to_string()).unwrap_or_default()),
            PaneInput::Text(t) => format!("text:{t}"),
            PaneInput::Bytes(b) => format!("bytes:{b:?}"),
            PaneInput::Paste { data, bracketed } => format!("paste:{}:{bracketed}", String::from_utf8_lossy(data)),
            PaneInput::Reset => "reset".into(),
        };
        self.sent.push((pane, s));
        Ok(())
    }
    fn pane_info(&self, pane: PaneId) -> PaneInfo<'_> {
        PaneInfo {
            pid: 1000u32.wrapping_add(pane.0),
            title: self.titles.get(&pane).map(|s| s.as_str()).unwrap_or(""),
            current_command: self.commands.get(&pane).map(|s| s.as_str()).unwrap_or("zsh"),
            current_path: self.paths.get(&pane).map(|s| s.as_str()).unwrap_or(""),
            history_size: 0,
            history_bytes: 0,
            in_mode: self.in_mode.contains(&pane),
            pipe: self.pipes.contains_key(&pane),
        }
    }
    fn capture_pane(&self, pane: PaneId, range: CaptureRange) -> String {
        format!("captured {pane} {:?}\n", range.start)
    }
    fn attach_client(&mut self, client: ClientId, session: SessionId) {
        self.attached.push((client, session));
    }
    fn detach_client(&mut self, client: ClientId, reason: DetachReason) {
        self.detached.push((client, reason));
    }
    fn enter_copy_mode(&mut self, _client: Option<ClientId>, pane: PaneId, _opts: CopyModeOptions) {
        self.copy_mode.push(pane);
    }
    fn show_message(&mut self, client: ClientId, text: &str, _duration_ms: u64) {
        self.messages.push((client, text.to_string()));
    }
    fn prompt(&mut self, client: ClientId, prompt: Prompt) {
        self.prompts.push((client, prompt));
    }
    fn display_panes(&mut self, client: ClientId, _duration_ms: u64, template: &str, keys: bool) {
        let n = if keys { "" } else { " (no keys)" };
        self.messages.push((client, format!("display-panes {template}{n}")));
    }
    fn hangup_client_parent(&mut self, client: ClientId) {
        self.hangups.push(client);
    }
    fn bell(&mut self, client: ClientId) {
        self.bells.push(client);
    }
    fn redraw(&mut self, _what: Redraw) {}
    fn command_stdin(&mut self) -> Option<Vec<u8>> {
        self.stdin.take()
    }
    fn read_file(&mut self, _client: Option<ClientId>, path: &str) -> Result<Vec<u8>, String> {
        self.files.get(path).cloned().ok_or_else(|| format!("{path}: No such file or directory"))
    }
    fn write_file(&mut self, _client: Option<ClientId>, path: &str, data: &[u8], append: bool) -> Result<(), String> {
        let f = self.files.entry(path.to_string()).or_default();
        if !append {
            f.clear();
        }
        f.extend_from_slice(data);
        Ok(())
    }
    fn set_clipboard(&mut self, data: &[u8]) {
        self.clipboard.push(data.to_vec());
    }
    fn shutdown(&mut self) {
        self.shutdown = true;
    }
    fn mode_command(
        &mut self,
        _client: Option<ClientId>,
        pane: PaneId,
        command: &str,
        args: &[String],
        count: u32,
    ) -> Result<(), String> {
        if !self.in_mode.contains(&pane) {
            return Err("not in a mode".into());
        }
        self.mode_commands.push((pane, command.to_string(), args.to_vec(), count));
        Ok(())
    }
    fn read_config(&mut self, path: &str) -> Result<ConfigFile, ConfigError> {
        self.configs.get(path).cloned().ok_or_else(|| ConfigError::Missing(format!("{path}: No such file or directory")))
    }
}

struct T {
    m: Model,
    h: TestHost,
    ctx: CmdContext,
}

impl T {
    fn new() -> T {
        let mut m = Model::new("box.local");
        m.set_global_option("default-shell", "/bin/zsh").unwrap();
        T { m, h: TestHost::default(), ctx: CmdContext::none(NOW) }
    }

    /// A server with one detached 80x24 session "a" (like
    /// "tmux new -d -s a -x 80 -y 24").
    fn with_session() -> T {
        let mut t = T::new();
        t.ok("new -d -s a -x 80 -y 24");
        t
    }

    fn run(&mut self, s: &str) -> Result<String, String> {
        let list = parse_string(s).map_err(|e| e.0)?;
        let r = self.m.execute_list(&list, &self.ctx, &mut self.h).map(|o| o.stdout).map_err(|e| e.0);
        check_model(&self.m);
        r
    }

    fn ok(&mut self, s: &str) -> String {
        self.run(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn err(&mut self, s: &str) -> String {
        self.run(s).expect_err(s)
    }

    fn panes(&mut self) -> String {
        self.ok("list-panes -F '#{pane_index} #{pane_id} #{pane_left},#{pane_top} #{pane_width}x#{pane_height} a=#{pane_active}'")
            .lines()
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn attach_client(&mut self, id: u32, cols: u16, rows: u16) -> ClientId {
        let c = ClientId(id);
        self.m.client_connect(
            c,
            ClientInfo { name: format!("/dev/ttys00{id}"), term: "xterm-256color".into(), size: Size::new(cols, rows), cwd: "/home/u".into(), pid: 99, terminal: true },
        );
        c
    }
}

/// Internal consistency of the whole model.
fn check_model(m: &Model) {
    for s in m.sessions() {
        assert!(!s.windows.is_empty(), "session {} without windows", s.id());
        assert!(s.windows.values().any(|w| *w == s.current), "current window not in session");
        for l in &s.last {
            assert!(s.windows.values().any(|w| w == l), "stale last window");
            assert_ne!(*l, s.current);
        }
        for (idx, wid) in &s.windows {
            let w = m.window(*wid).expect("window exists");
            assert_eq!(w.session, s.id);
            assert_eq!(w.index, *idx);
            w.layout.check().unwrap();
            let mut lp = w.layout.panes();
            lp.sort();
            let mut wp = w.panes.clone();
            wp.sort();
            assert_eq!(lp, wp, "layout panes != window panes");
            assert!(w.panes.contains(&w.active));
            for p in &w.panes {
                assert_eq!(m.pane(*p).expect("pane exists").window, w.id);
            }
            for p in &w.last_panes {
                assert!(w.panes.contains(p) && *p != w.active, "stale last pane");
            }
        }
    }
    let nwin: usize = m.sessions().map(|s| s.windows.len()).sum();
    assert_eq!(nwin, m.windows.len(), "orphan windows");
    let npanes: usize = m.windows.values().map(|w| w.panes.len()).sum();
    assert_eq!(npanes, m.panes.len(), "orphan panes");
    for c in m.clients() {
        if let Some(s) = c.session {
            assert!(m.session(s).is_some(), "client attached to a dead session");
            let view = m.client_view(c.id).expect("an attached client has a view");
            let w = m.window(view.window).expect("viewed window exists");
            assert_eq!(w.session, s, "client views a window of another session");
            for (p, _) in &view.panes {
                assert_eq!(m.pane(*p).expect("viewed pane exists").window, w.id);
            }
        }
        if let Some(s) = c.last_session {
            assert!(m.session(s).is_some(), "client's last session is dead");
        }
    }
    // Every window and pane is reachable from exactly one place, every index
    // is in range and every id came from its counter.
    for (wid, w) in &m.windows {
        assert_eq!(*wid, w.id);
        assert!(w.id.0 < m.next_window && w.id.0 != u32::MAX, "window id {} not from the counter", w.id);
        assert!(w.index <= crate::model::MAX_WINDOW_INDEX, "window index {} too big", w.index);
        let s = m.session(w.session).expect("window's session exists");
        assert_eq!(s.windows.get(&w.index), Some(wid), "window {wid} not linked at its index");
    }
    let mut owner: HashMap<PaneId, WindowId> = HashMap::new();
    for w in m.windows.values() {
        for p in &w.panes {
            assert!(owner.insert(*p, w.id).is_none(), "pane {p} in two windows (or twice in one)");
        }
    }
    for (pid, p) in &m.panes {
        assert_eq!(*pid, p.id);
        assert!(p.id.0 < m.next_pane && p.id.0 != u32::MAX, "pane id {} not from the counter", p.id);
        assert_eq!(owner.get(pid), Some(&p.window), "pane {pid} not in the window it points at");
    }
    for s in m.sessions() {
        assert!(s.id.0 < m.next_session && s.id.0 != u32::MAX, "session id {} not from the counter", s.id);
    }
}

#[test]
fn new_session_and_list_sessions() {
    let mut t = T::with_session();
    // tmux 3.8: "a: 1 windows (created Sat Nov 14 21:50:20 2026)".
    assert_eq!(t.ok("ls"), "a: 1 windows (created Sat Nov 14 21:50:20 2026)\n");
    let spec = &t.h.spawned[0];
    assert_eq!(spec.size, Size::new(80, 24));
    assert_eq!(spec.command, SpawnCommand::LoginShell { shell: "/bin/zsh".into() });
    assert_eq!(spec.history_limit, 4000);
    assert_eq!(t.ok("display -p '#{session_id} #{window_id} #{pane_id} #{window_name}'"), "$0 @0 %0 zsh\n");
    // tmux 3.8: unnamed sessions are named after their id, skipping taken
    // names ("x.y:z" took $1; next are "2" and "3").
    t.ok("new -d -s x.y:z");
    t.ok("new -d");
    t.ok("new -d");
    assert_eq!(t.ok("ls -F '#S #{session_id}'"), "2 $2\n3 $3\na $0\nx.y:z $1\n");
    assert_eq!(t.err("new -d -s a"), "duplicate session: a");
    assert_eq!(t.err("new -d -s ''"), "invalid session: ");
    assert_eq!(t.ok("new -d -s p -P"), "p:\n");
    assert_eq!(t.ok("new -d -s q -P -F '#{session_name}:#{window_index}'"), "q:0\n");
    t.ok("new -d -s cmd -n editor vim file.txt");
    let spec = t.h.spawned.last().unwrap();
    assert_eq!(spec.command, SpawnCommand::Argv(vec!["vim".into(), "file.txt".into()]));
    assert_eq!(t.ok("lsw -t cmd -F '#W'"), "editor\n");
    t.ok("new -d -s one 'top -o cpu'");
    assert_eq!(t.h.spawned.last().unwrap().command, SpawnCommand::Shell { shell: "/bin/zsh".into(), command: "top -o cpu".into() });
    assert_eq!(t.ok("lsw -t one -F '#W'"), "top\n");
    assert_eq!(t.ok("has -t a"), "");
    assert_eq!(t.err("has -t nope"), "can't find session: nope");
}

#[test]
fn new_window_indexes_like_tmux() {
    let mut t = T::with_session();
    t.ok("neww -d");
    t.ok("neww -d -t :5");
    t.ok("neww -d");
    t.ok("neww -d");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n2\n3\n5\n");
    // tmux 3.8: "create window failed: index 0 in use".
    assert_eq!(t.err("neww -t :0"), "create window failed: index 0 in use");
    t.ok("selectw -t 5");
    t.ok("selectw -t 2");
    t.ok("kill-window");
    assert_eq!(t.ok("display -p '#I'"), "5\n");
    t.ok("kill-window -t 1");
    // tmux 3.8: "0 -", "3 ", "5 *".
    assert_eq!(t.ok("lsw -F '#I #{window_flags}'"), "0 -\n3 \n5 *\n");
    t.ok("neww -a -t :0");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n3\n5\n");
    t.ok("selectw -t 0");
    t.ok("neww -a");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n2\n3\n5\n");
    t.ok("neww -b -t :0 -d");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n2\n3\n4\n5\n");
    assert_eq!(t.ok("neww -d -P"), "a:6.0\n");
    t.ok("neww -d -n logs");
    t.ok("neww -S -n logs");
    assert_eq!(t.ok("display -p '#W'"), "logs\n");
    assert_eq!(t.ok("lsw -F '#W'").lines().filter(|l| *l == "logs").count(), 1);
    t.ok("neww -k -t :0 -d");
}

#[test]
fn errors_match_tmux() {
    let mut t = T::with_session();
    // All messages verified with tmux 3.8.
    assert_eq!(t.err("next-window"), "no next window");
    assert_eq!(t.err("previous-window"), "no previous window");
    assert_eq!(t.err("last-window"), "no last window");
    assert_eq!(t.err("switch-client -n"), "no current client");
    assert_eq!(t.err("switchc -l"), "no current client");
    assert_eq!(t.err("selectw -t nope"), "can't find window: nope");
    assert_eq!(t.err("selectp -t nope"), "can't find pane: nope");
    assert_eq!(t.err("set -g foo bar"), "invalid option: foo");
    assert_eq!(t.err("killp -t :.9"), "can't find pane: 9");
    assert_eq!(t.err("kill-window -t :9"), "can't find window: 9");
    assert_eq!(t.err("selectw -t :=7"), "can't find window: 7");
    assert_eq!(t.err("swap-pane -t %99"), "can't find pane: %99");
    assert_eq!(t.err("resizep -x abc"), "width invalid");
    assert_eq!(t.err("unbind-key QQ"), "unknown key: QQ");
    assert_eq!(t.err("set -g status-left-length 99999999999"), "value is too large: 99999999999");
    assert_eq!(t.err("set -g mouse maybe"), "bad value: maybe");
    assert_eq!(t.err("set -g base-index -1"), "value is too small: -1");
    assert_eq!(t.err("set -g prefix C-"), "bad key: C-");
    assert_eq!(t.err("set -g status-position up"), "unknown value: up");
    assert_eq!(t.err("lastp"), "no last pane");
    let mut empty = T::new();
    assert_eq!(empty.err("attach"), "no sessions");
    assert_eq!(empty.err("neww"), "no current session");
}

#[test]
fn split_list_panes_and_zoom() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    // tmux 3.8 defaults.
    assert_eq!(
        t.ok("lsp"),
        "0: [40x24] [history 0/4000, 0 bytes] %0\n1: [39x24] [history 0/4000, 0 bytes] %1 (active)\n"
    );
    assert_eq!(t.ok("lsp -s").lines().next().unwrap(), "0.0: [40x24] [history 0/4000, 0 bytes] %0");
    assert_eq!(t.ok("lsp -a").lines().next().unwrap(), "a:0.0: [40x24] [history 0/4000, 0 bytes] %0");
    assert_eq!(t.h.resized[&PaneId(0)], Size::new(40, 24));
    assert_eq!(t.h.spawned[1].size, Size::new(39, 24));
    t.ok("splitw -v -t 0");
    assert_eq!(t.panes(), "0 %0 0,0 40x12 a=0 | 1 %2 0,13 40x11 a=1 | 2 %1 41,0 39x24 a=0");
    let lsw = t.ok("lsw");
    assert!(lsw.starts_with("0: zsh* (3 panes) [80x24] [layout "), "{lsw}");
    assert!(lsw.ends_with("] @0 (active)\n"), "{lsw}");
    t.ok("resizep -Z");
    assert_eq!(t.ok("display -p '#{window_flags} #{window_zoomed_flag}'"), "*Z 1\n");
    assert_eq!(t.h.resized[&PaneId(2)], Size::new(80, 24));
    t.ok("selectp -t 0");
    assert_eq!(t.ok("display -p '#{window_flags}'"), "*\n", "select-pane unzooms");
    assert_eq!(t.h.resized[&PaneId(2)], Size::new(40, 11));
    t.ok("resizep -Z");
    t.ok("selectp -Z -t 1");
    assert_eq!(t.ok("display -p '#{window_flags} #P'"), "*Z 1\n", "-Z keeps the zoom");
    t.ok("resizep -Z");
    assert_eq!(t.err("splitw -l abc"), "size invalid");
    t.ok("splitw -d -p 30 -h");
    assert_eq!(t.ok("display -p '#{window_panes}'"), "4\n");
    assert_eq!(t.ok("splitw -d -P -F '#{pane_id}'"), "%4\n");
}

#[test]
fn split_uses_active_pane_cwd() {
    let mut t = T::with_session();
    t.h.paths.insert(PaneId(0), "/work/project".into());
    t.ok("splitw -t 0");
    assert_eq!(t.h.spawned[1].cwd, "/work/project");
    t.ok("splitw -h -t 0 -c /tmp");
    assert_eq!(t.h.spawned[2].cwd, "/tmp");
    t.ok("splitw -h -t 0 -c sub");
    assert_eq!(t.h.spawned[3].cwd, "/work/project/sub", "relative to the default directory");
    t.ok("splitw -t 0 -c '#{pane_id}'");
    assert_eq!(t.h.spawned[4].cwd, "/work/project/%0", "-c is a format expanded for the target pane");
}

#[test]
fn spawn_failure_rolls_back() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    let before = t.m.window(WindowId(0)).unwrap().layout().clone();
    t.h.fail_spawn = true;
    assert_eq!(t.err("splitw -t 0"), "create pane failed: fork failed: Resource temporarily unavailable");
    assert_eq!(t.m.window(WindowId(0)).unwrap().layout(), &before);
    assert_eq!(t.err("neww"), "create window failed: fork failed: Resource temporarily unavailable");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n");
    assert!(t.err("new -d -s b").starts_with("create window failed"));
    assert_eq!(t.ok("ls -F '#S'"), "a\n");
    t.h.fail_spawn = false;
    t.ok("neww");
    assert_eq!(t.ok("display -p '#{window_id} #{pane_id}'"), "@3 %5\n", "ids are never reused");
}

#[test]
fn swap_and_rotate_match_tmux() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    t.ok("splitw -v");
    t.ok("selectp -t 1");
    t.ok("swapp -D");
    // tmux 3.8: "0 %0 0,0 a=0 1 %2 41,0 a=0 2 %1 41,13 a=1"
    assert_eq!(t.panes(), "0 %0 0,0 40x24 a=0 | 1 %2 41,0 39x12 a=0 | 2 %1 41,13 39x11 a=1");
    t.ok("rotatew");
    // tmux 3.8: "0 %2 0,0 a=0 1 %1 41,0 a=0 2 %0 41,13 a=1"
    assert_eq!(t.panes(), "0 %2 0,0 40x24 a=0 | 1 %1 41,0 39x12 a=0 | 2 %0 41,13 39x11 a=1");
    t.ok("rotatew -D");
    assert_eq!(t.panes(), "0 %0 0,0 40x24 a=0 | 1 %2 41,0 39x12 a=0 | 2 %1 41,13 39x11 a=1");
    t.ok("swapp -U -d");
    assert_eq!(t.panes(), "0 %0 0,0 40x24 a=0 | 1 %1 41,0 39x12 a=0 | 2 %2 41,13 39x11 a=1");
}

#[test]
fn select_pane_directions_wrap_like_tmux() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    t.ok("splitw -v");
    t.ok("selectp -t 1");
    t.ok("swapp -D");
    // Layout: %0 left, %2 top right, %1 bottom right.
    t.ok("selectp -t '{right}'");
    assert_eq!(t.ok("display -p '#D'"), "%2\n");
    t.ok("selectp -R");
    assert_eq!(t.ok("display -p '#D'"), "%0\n");
    t.ok("selectp -L");
    assert_eq!(t.ok("display -p '#D'"), "%2\n");
    t.ok("selectp -U");
    assert_eq!(t.ok("display -p '#D'"), "%1\n");
    t.ok("lastp");
    assert_eq!(t.ok("display -p '#D'"), "%2\n");
    t.ok("selectp -l");
    assert_eq!(t.ok("display -p '#D'"), "%1\n");
    t.ok("selectp -t :.+");
    assert_eq!(t.ok("display -p '#P'"), "0\n");
    t.ok("selectp -t :.-");
    assert_eq!(t.ok("display -p '#P'"), "2\n");
}

#[test]
fn targets() {
    let mut t = T::with_session();
    t.ok("new -d -s work");
    t.ok("neww -d -t work -n editor");
    t.ok("neww -d -t work -n logs");
    t.ok("splitw -d -t work:1");
    let d = |t: &mut T, target: &str| t.ok(&format!("display -p -t '{target}' '#S:#I.#P #D'")).trim().to_string();
    assert_eq!(d(&mut t, "work"), "work:0.0 %1");
    assert_eq!(d(&mut t, "wo"), "work:0.0 %1");
    assert_eq!(d(&mut t, "w*"), "work:0.0 %1");
    assert_eq!(d(&mut t, "$1"), "work:0.0 %1");
    assert_eq!(d(&mut t, "work:1"), "work:1.0 %2");
    assert_eq!(d(&mut t, "work:1.1"), "work:1.1 %4");
    assert_eq!(d(&mut t, "work:editor"), "work:1.0 %2");
    assert_eq!(d(&mut t, "work:ed"), "work:1.0 %2");
    assert_eq!(d(&mut t, "work:=logs"), "work:2.0 %3");
    assert_eq!(d(&mut t, "work:$"), "work:2.0 %3");
    assert_eq!(d(&mut t, "work:^"), "work:0.0 %1");
    assert_eq!(d(&mut t, "work:{end}"), "work:2.0 %3");
    assert_eq!(d(&mut t, "@2"), "work:1.0 %2");
    assert_eq!(d(&mut t, "%4"), "work:1.1 %4");
    assert_eq!(d(&mut t, "@2.1"), "work:1.1 %4");
    assert_eq!(d(&mut t, "work:+"), "work:1.0 %2");
    assert_eq!(d(&mut t, "work:-"), "work:2.0 %3");
    assert_eq!(d(&mut t, "work:+2"), "work:2.0 %3");
    // With no client the current session is the most recently active.
    assert_eq!(d(&mut t, ":1"), "work:1.0 %2");
    assert_eq!(d(&mut t, "editor"), "work:1.0 %2");
    assert_eq!(d(&mut t, "a"), "a:0.0 %0");
    assert_eq!(t.err("display -p -t 'work:=lo' x"), "can't find window: lo");
    t.ok("new -d -s workshop");
    assert_eq!(t.err("has -t wor"), "can't find session: wor");
    assert_eq!(t.err("display -p -t wor x"), "can't find pane: wor");
    assert_eq!(d(&mut t, "=work:"), "work:0.0 %1");
    // A pane target that is a number is a pane index first (tmux).
    t.ok("splitw -d -t work:0");
    t.ok("selectw -t work:0");
    t.ctx.pane = Some(PaneId(1));
    assert_eq!(d(&mut t, "1"), "work:0.1 %6");
    assert_eq!(d(&mut t, "."), "work:0.0 %1");
    assert_eq!(d(&mut t, ".1"), "work:0.1 %6");
    t.ok("selectp -t 1");
    assert_eq!(d(&mut t, "!"), "work:0.0 %1");
    assert_eq!(d(&mut t, "{last}"), "work:0.0 %1");
    assert_eq!(d(&mut t, "{up-of}"), "work:0.0 %1");
    assert_eq!(t.err("display -p -t '{left-of}' x"), "can't find pane: {left-of}");
    assert_eq!(d(&mut t, "{bottom}"), "work:0.1 %6");
    assert_eq!(d(&mut t, "{top}"), "work:0.0 %1");
}

#[test]
fn formats_and_display_message() {
    let mut t = T::with_session();
    // tmux 3.8 default display-message template.
    assert_eq!(t.ok("display -p"), "[a] 0:zsh, current pane 0 - (21:50 14-Nov-26)\n");
    assert_eq!(
        t.ok("display -p '#{session_attached} #{session_windows} [#{host}] [#{host_short}] [#T] #{pane_pid} #{window_panes} [#{pane_current_command}]'"),
        "0 1 [box.local] [box] [box.local] 1000 1 [zsh]\n"
    );
    t.h.titles.insert(PaneId(0), "vim README".into());
    assert_eq!(t.ok("display -p '#T|#{=3:pane_title}|#{?pane_active,act,}|#{pane_width}x#{pane_height}'"), "vim README|vim|act|80x24\n");
    assert_eq!(t.ok("display -p '#{t:session_created}'"), "Sat Nov 14 21:50:20 2026\n");
    assert_eq!(t.ok("display -p '#{base-index} #{status}'"), "0 on\n");
    assert_eq!(t.ok("display -pl '#S'"), "#S\n");
    assert_eq!(t.ok("display -p -F '#I'"), "0\n");
}

#[test]
fn options_set_and_show() {
    let mut t = T::with_session();
    assert_eq!(t.ok("show -g base-index"), "base-index 0\n");
    // tmux 3.8 output for these.
    assert_eq!(t.ok("show -gv status-left"), "[#{session_name}] \n");
    assert_eq!(
        t.ok("show -g status-right"),
        "status-right \"#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\\\"#{=21:pane_title}\\\" %H:%M %d-%b-%y\"\n"
    );
    assert_eq!(t.ok("show -g prefix2"), "prefix2 None\n");
    assert_eq!(t.ok("show -g default-command"), "default-command ''\n");
    t.ok("set -g base-index 1");
    t.ok("neww -d");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n");
    t.ok("set status-left 'S:#S'");
    assert_eq!(t.ok("show status-left"), "status-left \"S:#S\"\n");
    assert_eq!(t.ok("show -g status-left"), "status-left \"[#{session_name}] \"\n");
    t.ok("set -u status-left");
    assert_eq!(t.ok("show status-left"), "");
    t.ok("set -g mouse");
    assert_eq!(t.ok("show -gv mouse"), "off\n", "flag with no value toggles");
    t.ok("setw -g pane-base-index 1");
    assert_eq!(t.ok("display -p '#P'"), "1\n");
    t.ok("set automatic-rename off");
    assert_eq!(t.ok("show -w automatic-rename"), "automatic-rename off\n");
    t.ok("set -g @theme dark");
    assert_eq!(t.ok("show -gv @theme"), "dark\n");
    assert_eq!(t.ok("display -p '#{@theme}'"), "dark\n");
    t.ok("set -ga @theme er");
    assert_eq!(t.ok("show -gv @theme"), "darker\n");
    t.ok("set -gu base-index");
    assert_eq!(t.ok("show -gv base-index"), "0\n");
    t.ok("set -g status-ju centre");
    assert_eq!(t.err("set -g status-l x"), "ambiguous option: status-l");
    assert_eq!(t.ok("set -gq nonsense 1"), "");
    t.ok("set -gF @where '#S'");
    assert_eq!(t.ok("show -gv @where"), "a\n");
    assert!(t.ok("show -g").contains("prefix C-b\n"));
    assert!(t.ok("show -gw").contains("window-status-current-format \"#I:#W#{?window_flags,#{window_flags}, }\"\n"));
    assert!(t.ok("show -s").contains("escape-time 5\n"));
}

#[test]
fn clients_attach_detach_and_window_size() {
    let mut t = T::with_session();
    let c1 = t.attach_client(1, 120, 40);
    t.ctx = CmdContext::client(c1, NOW);
    t.ok("attach -t a");
    assert_eq!(t.h.attached, vec![(c1, SessionId(0))]);
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height} #{session_attached}'"), "120x39 1\n");
    assert_eq!(t.h.resized[&PaneId(0)], Size::new(120, 39));
    // tmux list-clients default.
    assert_eq!(t.ok("lsc"), "/dev/ttys001: a [120x40 xterm-256color] (attached,focused,UTF-8)\n");
    // window-size latest: the most recently active client decides.
    let c2 = t.attach_client(2, 100, 30);
    t.ctx = CmdContext::client(c2, NOW);
    t.ok("attach -t a");
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "100x29\n");
    let k = KeyEvent::new(Key::Char('x'), Mods::NONE);
    assert_eq!(t.m.handle_key(c1, &k, 0, &mut t.h), KeyDecision::SendToPane);
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "120x39\n");
    t.m.client_resized(c1, Size::new(90, 20), &mut t.h);
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "90x19\n");
    t.ok("set -g status off");
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "90x20\n");
    t.ok("set -g status on");
    t.ok("set -g window-size largest");
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "100x29\n");
    t.ok("set -g window-size smallest");
    assert_eq!(t.ok("display -p '#{window_width}x#{window_height}'"), "90x19\n");
    t.ok("set -g window-size latest");
    // attach -d kicks the other client.
    t.ok("attach -d -t a");
    assert_eq!(t.h.detached, vec![(c1, DetachReason::Kicked { session: "a".into() })]);
    t.ok("detach");
    assert_eq!(t.h.detached.last().unwrap(), &(c2, DetachReason::Detached { session: "a".into() }));
    assert_eq!(t.err("detach"), "no current client");
    t.m.client_disconnect(c1, &mut t.h);
    assert!(t.m.client(c1).is_none());
}

#[test]
fn switch_client() {
    let mut t = T::with_session();
    t.ok("new -d -s b");
    t.ok("new -d -s c");
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t a");
    t.ok("switchc -n");
    assert_eq!(t.m.client(c).unwrap().session(), Some(SessionId(1)));
    t.ok("switchc -n");
    t.ok("switchc -n");
    assert_eq!(t.m.client(c).unwrap().session(), Some(SessionId(0)), "wraps by name");
    t.ok("switchc -p");
    assert_eq!(t.m.client(c).unwrap().session(), Some(SessionId(2)));
    t.ok("switchc -l");
    assert_eq!(t.m.client(c).unwrap().session(), Some(SessionId(0)));
    t.ok("switchc -t b");
    assert_eq!(t.ok("display -p '#S #{client_session} #{client_last_session}'"), "b b a\n");
    t.ok("switchc -T prefix");
    assert!(t.m.client(c).unwrap().key_state().in_prefix());
    t.ok("kill-session -t a");
    t.ok("kill-session -t c");
    assert_eq!(t.err("switchc -n"), "can't find next session");
    assert_eq!(t.err("switchc -l"), "can't find last session");
}

#[test]
fn pane_exit_closes_window_then_session() {
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    t.ok("splitw");
    t.ok("neww");
    t.m.pane_exited(PaneId(2), Some(0), &mut t.h);
    check_model(&t.m);
    assert_eq!(t.ok("display -p '#I #D'"), "0 %1\n", "window closed, back to the last window");
    t.m.pane_exited(PaneId(1), Some(0), &mut t.h);
    assert_eq!(t.ok("display -p '#D #{window_panes}'"), "%0 1\n");
    assert!(t.h.killed.is_empty(), "exited panes are not killed");
    t.m.pane_exited(PaneId(0), Some(0), &mut t.h);
    check_model(&t.m);
    assert_eq!(t.h.detached, vec![(c, DetachReason::SessionEnded)]);
    assert!(t.h.shutdown, "exit-empty");
}

#[test]
fn detach_on_destroy_off_switches() {
    let mut t = T::with_session();
    t.ok("new -d -s b");
    t.ok("set -g detach-on-destroy off");
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t a");
    t.ok("kill-session");
    assert_eq!(t.m.client(c).unwrap().session(), Some(SessionId(1)));
    assert!(t.h.detached.is_empty());
    assert!(!t.h.shutdown);
    t.ok("kill-server");
    assert_eq!(t.h.detached, vec![(c, DetachReason::ServerExit)]);
    assert!(t.h.shutdown);
    assert_eq!(t.m.sessions().count(), 0);
}

#[test]
fn remain_on_exit_and_respawn() {
    let mut t = T::with_session();
    t.ok("set -g remain-on-exit on");
    t.m.pane_exited(PaneId(0), Some(1), &mut t.h);
    assert_eq!(t.ok("display -p '#{pane_dead} #{pane_dead_status}'"), "1 1\n");
    t.ok("respawnp");
    assert_eq!(t.ok("display -p '#{pane_dead}'"), "0\n");
    assert!(t.h.spawned.last().unwrap().respawn);
    assert_eq!(t.err("respawnp"), "respawn pane failed: pane a:0.0 still active");
    t.ok("respawnp -k htop");
    assert_eq!(t.h.killed, vec![PaneId(0), PaneId(0)]);
    assert_eq!(t.ok("display -p '#{pane_start_command}'"), "htop\n");
    t.ok("set -g remain-on-exit failed");
    t.m.pane_exited(PaneId(0), Some(0), &mut t.h);
    assert_eq!(t.m.sessions().count(), 0);
}

#[test]
fn kill_pane_picks_last_then_neighbour() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    t.ok("splitw -v");
    t.ok("selectp -t 0");
    t.ok("selectp -t 2");
    t.ok("killp");
    assert_eq!(t.ok("display -p '#D'"), "%0\n", "the last pane becomes active");
    t.ok("splitw");
    t.ok("killp -a");
    assert_eq!(t.ok("display -p '#D #{window_panes}'"), "%3 1\n");
    assert_eq!(t.h.killed, vec![PaneId(2), PaneId(0), PaneId(1)]);
}

#[test]
fn layouts_via_commands() {
    let mut t = T::with_session();
    t.ok("splitw -h");
    t.ok("splitw -v");
    t.ok("selectl even-horizontal");
    // tmux 3.8 (same pane order).
    assert_eq!(t.panes(), "0 %0 0,0 26x24 a=0 | 1 %1 27,0 26x24 a=0 | 2 %2 54,0 26x24 a=1");
    t.ok("selectl tiled");
    assert_eq!(t.panes(), "0 %0 0,0 39x11 a=0 | 1 %1 40,0 40x11 a=0 | 2 %2 0,12 80x12 a=1");
    t.ok("nextl");
    assert_eq!(t.ok("display -p '#{pane_width}'"), "26\n", "after tiled comes even-horizontal");
    t.ok("prevl");
    t.ok("selectl -o");
    assert_eq!(t.ok("display -p '#{pane_width}'"), "26\n");
    let layout = t.ok("display -p '#{window_layout}'").trim().to_string();
    t.ok("selectl main-vertical");
    t.ok(&format!("selectl '{layout}'"));
    assert_eq!(t.ok("display -p '#{window_layout}'").trim(), layout);
    assert_eq!(t.err("selectl foo"), "unknown layout: foo");
    assert_eq!(t.err("selectl 'abcd,1x1,0,0,1'"), "invalid layout: abcd,1x1,0,0,1");
    t.ok("selectl -E");
    t.ok("resizep -t 0 -x 10");
    assert_eq!(t.ok("display -t 0 -p '#{pane_width}'"), "10\n");
    t.ok("resizep -t 0 -R 5");
    assert_eq!(t.ok("display -t 0 -p '#{pane_width}'"), "15\n");
    t.ok("resizep -t 0 -x 50%");
    assert_eq!(t.ok("display -t 0 -p '#{pane_width}'"), "40\n");
    assert_eq!(t.err("resizep -L x"), "adjustment invalid: x");
}

#[test]
fn break_move_swap_windows() {
    let mut t = T::with_session();
    t.ok("splitw");
    t.ok("breakp");
    assert_eq!(t.ok("lsw -F '#I #{window_panes} #{window_active}'"), "0 1 0\n1 1 1\n");
    t.ok("neww -d");
    t.ok("movew -s :2 -t :7");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n7\n");
    assert_eq!(t.err("movew -s :7 -t :0"), "index in use: 0");
    t.ok("movew -r");
    assert_eq!(t.ok("lsw -F '#I'"), "0\n1\n2\n");
    t.ok("selectw -t 0");
    // tmux 3.8: "swapw -t :2" leaves index 0 current (now showing the other
    // window); "swapw -d" makes the destination index current.
    assert_eq!(t.ok("lsw -F '#I #{window_id} #{window_active}'"), "0 @0 1\n1 @1 0\n2 @2 0\n");
    t.ok("swapw -t :2");
    assert_eq!(t.ok("lsw -F '#I #{window_id} #{window_active}'"), "0 @2 1\n1 @1 0\n2 @0 0\n");
    t.ok("swapw -d -s :0 -t :1");
    assert_eq!(t.ok("lsw -F '#I #{window_id} #{window_active}'"), "0 @1 0\n1 @2 1\n2 @0 0\n");
    t.ok("selectw -t 2");
    t.ok("new -d -s b");
    t.ok("movew -s a:2 -t b:5");
    assert_eq!(t.ok("lsw -t b -F '#I #{window_id}'"), "0 @3\n5 @0\n");
    t.ok("set -g renumber-windows on");
    t.ok("kill-window -t b:0");
    assert_eq!(t.ok("lsw -t b -F '#I'"), "0\n");
    t.ok("breakp -s b:0 -t a:9 -n solo");
    assert_eq!(t.ok("lsw -t a -F '#I #W'").trim_end().lines().last().unwrap(), "9 solo");
    assert!(t.m.session(SessionId(1)).is_none(), "b lost its only window");
}

#[test]
fn send_keys_paste_capture() {
    let mut t = T::with_session();
    t.ok("send-keys ls Enter C-c 'echo hi' Space");
    let sent: Vec<&str> = t.h.sent.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(sent, vec!["text:ls", "key:Enter", "key:C-c", "text:echo hi", "key:Space"]);
    t.h.sent.clear();
    t.ok("send -l Enter");
    t.ok("send -N 2 a");
    t.ok("send -H 41 0x42");
    t.ok("send -R");
    let sent: Vec<&str> = t.h.sent.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(sent, vec!["text:Enter", "key:a", "key:a", "bytes:[65, 66]", "reset"]);
    t.h.sent.clear();
    t.ok("send-prefix");
    assert_eq!(t.h.sent[0].1, "key:C-b");
    t.h.sent.clear();
    assert_eq!(t.ok("capturep -p -S -10"), "captured %0 Line(-10)\n");
    assert_eq!(t.h.captures.len(), 0);
    t.ok("capturep -S - -b cap");
    assert_eq!(t.ok("showb -b cap"), "captured %0 HistoryStart\n");
    t.ok("setb 'line1\nline2'");
    assert_eq!(t.ok("lsb"), "buffer0000: 11 bytes: \"line1\\nline2\"\ncap: 25 bytes: \"captured %0 HistoryStart\\n\"\n");
    t.ok("pasteb -p");
    assert_eq!(t.h.sent[0].1, "paste:line1\rline2:true");
    t.ok("pasteb -r -d");
    assert_eq!(t.h.sent[1].1, "paste:line1\nline2:false");
    assert_eq!(t.ok("lsb -F '#{buffer_name}'"), "cap\n");
    t.ok("deleteb");
    assert_eq!(t.err("deleteb"), "no buffer");
    assert_eq!(t.err("showb -b x"), "no buffer x");
    for i in 0..60 {
        t.m.set_buffer(None, format!("{i}").into_bytes());
    }
    assert_eq!(t.m.buffers().len(), 50, "buffer-limit");
}

#[test]
fn keys_bindings_and_prompts() {
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    let key = |n: &str| KeyCode::parse(n).unwrap().to_event();
    assert_eq!(t.m.handle_key(c, &key("C-b"), 0, &mut t.h), KeyDecision::Swallow);
    let KeyDecision::Run(cmds) = t.m.handle_key(c, &key("%"), 0, &mut t.h) else { panic!() };
    t.m.execute_list(&cmds, &t.ctx, &mut t.h).unwrap();
    assert_eq!(t.ok("display -p '#{window_panes}'"), "2\n");
    // tmux 3.8 output.
    assert_eq!(t.ok("list-keys -T prefix C-b"), "bind-key  -T prefix C-b send-prefix\n");
    t.ok("bind -n M-h select-pane -L");
    t.ok("bind -r -T prefix h 'select-pane -L ; display -p hi'");
    assert_eq!(t.ok("lsk -T root"), "bind-key  -T root M-h select-pane -L\n");
    assert_eq!(t.ok("lsk -T prefix h"), "bind-key -r -T prefix h select-pane -L \\; display-message -p hi\n");
    assert!(matches!(t.m.handle_key(c, &key("M-h"), 0, &mut t.h), KeyDecision::Run(_)));
    t.ok("unbind -n M-h");
    assert_eq!(t.err("lsk -T root"), "table root doesn't exist");
    t.ok("set -g prefix C-a");
    assert_eq!(t.m.handle_key(c, &key("C-b"), 0, &mut t.h), KeyDecision::SendToPane);
    assert_eq!(t.m.handle_key(c, &key("C-a"), 0, &mut t.h), KeyDecision::Swallow);
    assert_eq!(t.ok("display -p '#{client_prefix} #{client_key_table}'"), "1 prefix\n");
    let KeyDecision::Run(cmds) = t.m.handle_key(c, &key(","), 0, &mut t.h) else { panic!() };
    t.m.execute_list(&cmds, &t.ctx, &mut t.h).unwrap();
    match &t.h.prompts[0].1 {
        Prompt::Command { prompts, inputs, template, .. } => {
            assert_eq!(prompts, &vec!["(rename-window) ".to_string()]);
            assert_eq!(inputs, &vec!["zsh".to_string()]);
            let cmd = expand_prompt_template(template, &["code"]);
            assert_eq!(cmd, "rename-window \"%%\"".replace("%%", "code"));
            t.ok(&cmd);
            assert_eq!(t.ok("display -p '#W'"), "code\n");
        }
        other => panic!("{other:?}"),
    }
    t.ok("confirm-before -p 'kill-pane #P? (y/n)' kill-pane");
    assert_eq!(
        t.h.prompts[1].1,
        Prompt::Confirm { prompt: "kill-pane 1? (y/n) ".into(), command: "kill-pane".into(), default_yes: false }
    );
    t.ok("command-prompt");
    assert!(matches!(&t.h.prompts[2].1, Prompt::Command { prompts, template, .. } if prompts[0] == ":" && template == "%1"));
    t.ok("display hello");
    assert_eq!(t.h.messages.last().unwrap(), &(c, "hello".to_string()));
    t.ok("display-panes");
    assert_eq!(t.h.messages.last().unwrap().1, "display-panes select-pane -t '%%'");
    t.ok("copy-mode -u");
    assert_eq!(t.h.copy_mode, vec![PaneId(1)]);
}

#[test]
fn automatic_rename_bells_activity() {
    let mut t = T::with_session();
    t.ok("neww");
    t.h.commands.insert(PaneId(1), "vim".into());
    t.m.pane_changed(PaneId(1), &mut t.h);
    assert_eq!(t.ok("display -p '#W'"), "vim\n");
    t.ok("rename-window editor");
    t.h.commands.insert(PaneId(1), "less".into());
    t.m.pane_changed(PaneId(1), &mut t.h);
    assert_eq!(t.ok("display -p '#W #{automatic-rename}'"), "editor off\n");
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    t.m.pane_bell(PaneId(0), &mut t.h);
    assert_eq!(t.ok("lsw -F '#I#{window_flags}'"), "0!-\n1*\n");
    assert_eq!(t.h.bells, vec![c]);
    t.ok("setw -g monitor-activity on");
    t.m.pane_activity(PaneId(0), &mut t.h);
    assert_eq!(t.ok("lsw -F '#I#{window_raw_flags}|#{window_flags}'"), "0#!-|##!-\n1*|*\n");
    t.ok("next-window -a");
    assert_eq!(t.ok("lsw -F '#I#{window_flags}'"), "0*\n1-\n", "selecting clears alerts");
    assert_eq!(t.err("next -a"), "no next window");
}

#[test]
fn status_line_default_look() {
    let mut t = T::with_session();
    t.ok("neww");
    t.ok("splitw");
    t.ok("selectw -t 0");
    let line = t.m.status_line(SessionId(0), 80, NOW, &t.h);
    let text: String = line.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(text.chars().count(), 80);
    // tmux 3.8 default: "[a] 0:zsh* 1:zsh-" ... "\"box.local\" 21:50 14-Nov-26".
    assert!(text.starts_with("[a] 0:zsh* 1:zsh- "), "{text}");
    assert!(text.ends_with("\"box.local\" 21:50 14-Nov-26"), "{text}");
    let current = line.iter().find(|s| s.text.contains("0:zsh*")).unwrap();
    assert_eq!(current.style.underline, lif_types::Underline::Single, "window-status-current-style underscore");
    assert_eq!(current.style.bg, lif_types::Color::Indexed(2));
    t.ok("set -g status-right ''");
    t.ok("set -g status-left '#[fg=red]#S#[default]|'");
    let line = t.m.status_line(SessionId(0), 30, NOW, &t.h);
    assert_eq!(line[0].text, "a");
    assert_eq!(line[0].style.fg, lif_types::Color::Indexed(1));
}

#[test]
fn client_view_geometry() {
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    t.ok("splitw -h");
    let v = t.m.client_view(c).unwrap();
    assert_eq!(v.status_row, Some(24));
    assert_eq!(v.panes, vec![(PaneId(0), lif_types::Rect::new(0, 0, 40, 24)), (PaneId(1), lif_types::Rect::new(41, 0, 39, 24))]);
    assert_eq!(v.borders, vec![lif_types::Rect::new(40, 0, 1, 24)]);
    t.ok("set -g status-position top");
    let v = t.m.client_view(c).unwrap();
    assert_eq!(v.status_row, Some(0));
    assert_eq!(v.panes[0].1, lif_types::Rect::new(0, 1, 40, 24));
}

/// Rough timings of the paths the server hits often. Run with
/// `cargo test --release -p lif-core -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_hot_paths() {
    use std::time::Instant;
    let mut t = T::with_session();
    let c = t.attach_client(1, 200, 50);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    for _ in 0..4 {
        t.ok("neww");
    }
    for _ in 0..7 {
        t.ok("splitw");
        t.ok("selectl tiled");
    }
    let n = 200_000u32;
    let key = KeyEvent::new(Key::Char('a'), Mods::NONE);
    let start = Instant::now();
    for i in 0..n {
        let d = t.m.handle_key(c, &key, i as u64, &mut t.h);
        assert_eq!(d, KeyDecision::SendToPane);
    }
    eprintln!("handle_key (plain key): {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
    let n = 20_000u32;
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(t.m.client_view(c));
    }
    eprintln!("client_view (8 panes): {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(t.m.status_line(SessionId(0), 200, NOW, &t.h));
    }
    eprintln!("status_line (5 windows, default formats): {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(parse_string("split-window -h -c '#{pane_current_path}' -l 30%").unwrap());
    }
    eprintln!("parse_string: {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
    let layout = t.m.window(t.m.session(SessionId(0)).unwrap().current_window()).unwrap().layout().clone();
    let start = Instant::now();
    for i in 0..n {
        std::hint::black_box(layout.compute(Size::new(150 + (i % 50) as u16, 40)));
    }
    eprintln!("layout compute with re-fit (8 panes): {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
    let list = parse_string("split-window -h ; kill-pane").unwrap();
    let start = Instant::now();
    for _ in 0..n {
        t.m.execute_list(&list, &t.ctx, &mut t.h).unwrap();
    }
    eprintln!("execute split-window -h ; kill-pane: {:.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[(self.next() % v.len() as u64) as usize]
    }
}

/// What the fuzz test's host saw happen to pane processes, in order.
#[derive(Clone, Copy, Debug)]
enum ProcEvent {
    Spawn { pane: PaneId, respawn: bool },
    Kill(PaneId),
}

/// Replays [`TestHost::events`] to check the model never leaks a process,
/// never starts two for one pane and never reuses a pane id.
#[derive(Default)]
struct Procs {
    /// Panes with a running process.
    live: std::collections::HashSet<PaneId>,
    /// Panes whose process exited (reported with `pane_exited`).
    exited: std::collections::HashSet<PaneId>,
    /// Every pane id ever spawned.
    seen: std::collections::HashSet<PaneId>,
    /// Events of `TestHost::events` already replayed.
    done: usize,
}

impl Procs {
    fn exited(&mut self, p: PaneId) {
        assert!(self.live.remove(&p), "{p} exited without a process");
        self.exited.insert(p);
    }

    fn check(&mut self, h: &TestHost, m: &Model, step: &str) {
        for ev in &h.events[self.done..] {
            match *ev {
                ProcEvent::Spawn { pane, respawn } => {
                    assert!(!self.live.contains(&pane), "{step}: spawned {pane}, which still has a process");
                    if respawn {
                        assert!(self.seen.contains(&pane), "{step}: respawned {pane}, which never ran");
                    } else {
                        assert!(self.seen.insert(pane), "{step}: pane id {pane} reused");
                    }
                    self.exited.remove(&pane);
                    self.live.insert(pane);
                }
                ProcEvent::Kill(pane) => {
                    // Killing a pane whose process is gone (exited, or a
                    // failed respawn) is harmless; any other kill is not.
                    assert!(self.live.remove(&pane) || self.exited.contains(&pane), "{step}: killed {pane}, which never ran");
                    self.exited.insert(pane);
                }
            }
        }
        self.done = h.events.len();
        for p in &self.live {
            assert!(m.pane(*p).is_some(), "{step}: leaked the process of {p}");
        }
        for p in m.panes() {
            assert!(p.is_dead() || self.live.contains(&p.id()), "{step}: {} has no process", p.id());
        }
    }
}

#[test]
fn random_commands_keep_the_model_consistent() {
    // Window indices at the edges: 0, base-index values, the maximum and
    // its neighbours, and values past it up to u32::MAX.
    const IDX: [&str; 9] = ["0", "1", "2", "2147483646", "2147483647", "2147483648", "4294967294", "4294967295", "+1"];
    const SESSIONS: [&str; 5] = ["a", "b", "c", "1", "2"];
    let plain = [
        "splitw", "splitw -h", "splitw -b -l 3", "splitw -d -h -p 70", "killp", "killp -t :.+", "killp -a", "neww", "neww -d", "neww -a",
        "neww -b", "kill-window", "selectp -L", "selectp -R", "selectp -U", "selectp -D", "lastp", "last", "next", "prev",
        "swapp -U", "swapp -D", "rotatew", "rotatew -D", "resizep -Z", "resizep -L 3", "resizep -D 2", "resizep -x 20%",
        "selectl tiled", "selectl main-vertical", "selectl -E", "nextl", "breakp", "breakp -d", "breakp -a", "breakp -b -d", "movew -r",
        "swapw -t :+", "new -d", "new -d -x 200 -y 60", "new -d -x 20 -y 5", "switchc -n", "switchc -p", "switchc -l", "selectw -t :^",
        "selectw -l", "respawnp -k", "respawnp", "selectl -o", "joinp", "joinp -s :+", "detach", "detach -a", "attach", "attach -d",
        "kill-session", "kill-session -a", "set -g renumber-windows on", "set -g renumber-windows off", "neww -k -t :0", "movew -t :9",
        "set -g base-index 0", "set -g base-index 1", "set -g base-index 2147483646", "set -g base-index 2147483647", "set base-index 2",
        "set -g remain-on-exit on", "set -g remain-on-exit off", "set -g detach-on-destroy off", "set -g detach-on-destroy on",
        "set -g detach-on-destroy no-detached", "set -g detach-on-destroy previous", "set -g detach-on-destroy next",
        "set -g window-size largest", "set -g window-size smallest", "set -g window-size latest",
    ];
    let indexed = [
        "neww -d -t :{i}", "neww -t :{i}", "neww -a -t :{i}", "neww -b -d -t :{i}", "neww -k -t :{i}", "neww -t {s}:{i}",
        "movew -t :{i}", "movew -a -t :{i}", "movew -b -t :{i}", "movew -k -t :{i}", "movew -s :{i} -t {s}:", "movew -t {s}:{i}",
        "movew -d -a -s {s}: -t :{i}", "swapw -t :{i}", "swapw -s :{i} -t {s}:", "swapw -d -s {s}: -t :{i}", "selectw -t :{i}",
        "selectw -t {s}:{i}", "breakp -t :{i}", "breakp -a -t :{i}", "breakp -d -s :{i} -t {s}:{i}", "breakp -b -t {s}:{i}",
        "killw -t :{i}", "joinp -t :{i}", "respawnp -k -t :{i}", "kill-session -t {s}", "switchc -t {s}", "switchc -t {s}:{i}",
        "attach -t {s}", "new -d -s {s} -x 30 -y 8", "new -d -s {s} -x 120 -y 50", "neww -t {s}:", "killp -t {s}:{i}.0",
    ];
    // LIF_FUZZ_SEEDS=5000 cargo test ... runs a deeper search; the default
    // keeps the suite fast.
    let seeds: u64 = std::env::var("LIF_FUZZ_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    for seed in 1..=seeds {
        let mut t = T::with_session();
        let mut procs = Procs::default();
        procs.check(&t.h, &t.m, "start");
        let sizes = [(100, 40), (60, 20), (200, 70)];
        let mut connected = Vec::new();
        for (n, (cols, rows)) in sizes.iter().enumerate() {
            connected.push(t.attach_client(n as u32 + 1, *cols, *rows));
        }
        t.ctx = CmdContext::client(connected[0], NOW);
        t.ok("attach");
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        for step in 0..250 {
            let roll = rng.next() % 100;
            let what;
            if t.m.sessions().count() == 0 {
                t.ctx = CmdContext::none(NOW);
                t.h.fail_spawn = false;
                what = "new -d -s a".to_string();
                t.ok(&what);
            } else if roll < 6 {
                let live: Vec<PaneId> = t.m.panes().filter(|p| !p.is_dead()).map(|p| p.id()).collect();
                if live.is_empty() {
                    continue;
                }
                let p = live[(rng.next() % live.len() as u64) as usize];
                what = format!("pane_exited {p}");
                procs.exited(p);
                let status = if rng.next().is_multiple_of(2) { Some(0) } else { Some(1) };
                t.m.pane_exited(p, status, &mut t.h);
            } else if roll < 9 {
                // A client connects, disconnects or resizes.
                let id = (rng.next() % 4) as u32 + 1;
                let c = ClientId(id);
                if t.m.client(c).is_some() && rng.next().is_multiple_of(2) {
                    what = format!("disconnect {c}");
                    t.m.client_disconnect(c, &mut t.h);
                } else if t.m.client(c).is_some() {
                    let size = Size::new((rng.next() % 250) as u16 + 1, (rng.next() % 80) as u16 + 1);
                    what = format!("resize {c} {size:?}");
                    t.m.client_resized(c, size, &mut t.h);
                } else {
                    what = format!("connect {c}");
                    t.attach_client(id, 80 + id as u16 * 10, 24 + id as u16 * 5);
                }
            } else if roll < 11 {
                t.h.fail_spawn = !t.h.fail_spawn;
                what = format!("fail_spawn {}", t.h.fail_spawn);
            } else {
                // Run as a random connected client, or with no client.
                let clients: Vec<ClientId> = t.m.clients().map(|c| c.id()).collect();
                let pick = rng.next() as usize % (clients.len() + 1);
                t.ctx = match clients.get(pick) {
                    Some(c) => CmdContext::client(*c, NOW),
                    None => CmdContext::none(NOW),
                };
                what = if roll < 55 {
                    rng.pick(&plain).to_string()
                } else {
                    rng.pick(&indexed).replace("{i}", rng.pick(&IDX)).replace("{s}", rng.pick(&SESSIONS))
                };
                let _ = t.run(&what);
            }
            let step = format!("seed {seed} step {step}: {what}");
            check_model(&t.m);
            procs.check(&t.h, &t.m, &step);
        }
    }
}

/// Window indices stop at base-index's maximum (tmux's INT_MAX). They used
/// to wrap at u32::MAX: `neww -a` after window 4294967295 overwrote window 0
/// in the session's index map, leaking its pane and process.
#[test]
fn window_indices_stop_at_the_maximum() {
    let mut t = T::with_session();
    let too_big = |i: &str| format!("window index too big: {i} (the largest is 2147483647)");
    // The reported sequence: refused before any process starts.
    assert_eq!(t.err("neww -t a:4294967295"), format!("create window failed: {}", too_big("4294967295")));
    assert_eq!(t.err("neww -t a:2147483648"), format!("create window failed: {}", too_big("2147483648")));
    assert_eq!(t.h.spawned.len(), 1);
    t.ok("neww -a");
    t.ok("breakp");
    assert_eq!(t.ok("lsw -F '#{window_index}'"), "0\n2\n");
    // The largest index works; nothing fits after it.
    t.ok("neww -t a:2147483647");
    assert_eq!(t.err("neww -a"), "create window failed: no free window index");
    assert_eq!(t.err("neww -b"), "create window failed: no free window index");
    assert_eq!(t.err("neww -a -t :+1"), format!("create window failed: {}", too_big("2147483648")));
    t.ok("neww -d -t :2147483646");
    assert_eq!(t.err("neww -b -t :2147483646"), "create window failed: no free window index");
    assert_eq!(t.h.spawned.len(), 4, "a refused new-window must not start a process");
    t.ok("splitw");
    assert_eq!(t.err("breakp -a"), "no free window index");
    assert_eq!(t.err("breakp -t :4294967295"), too_big("4294967295"));
    assert_eq!(t.err("movew -s :0 -a -t :2147483646"), "no free window index");
    assert_eq!(t.err("movew -s :0 -t :4294967295"), too_big("4294967295"));
    assert_eq!(t.err("movew -s :0 -b -t :4294967295"), too_big("4294967295"));
    assert!(t.err("selectw -t :4294967295").contains("can't find window"));
    // Like tmux, -b shifts before the window leaves its place, so its own
    // index does not count as room.
    assert_eq!(t.err("movew -s :2147483647 -b -t :2147483646"), "no free window index");
    assert_eq!(t.ok("lsw -F '#{window_index}'"), "0\n2\n2147483646\n2147483647\n");
    // -b makes room by shifting up into the gap below the maximum.
    t.ok("neww -d -b -t :0");
    assert_eq!(t.ok("lsw -F '#{window_index}'"), "0\n1\n2\n2147483646\n2147483647\n");
    assert_eq!(t.err("neww -d -a -t :2147483645"), "create window failed: no free window index");
    // swap-window between extreme indices keeps them.
    t.ok("swapw -s :0 -t :2147483647");
    // With base-index at its maximum the only free index is taken.
    t.ok("set -g base-index 2147483647");
    assert_eq!(t.err("neww"), "create window failed: no free window index");
    assert_eq!(t.err("breakp"), "no free window index");
    // renumber-windows keeps every index at or below the maximum.
    t.ok("movew -r");
    assert_eq!(t.ok("lsw -F '#{window_index}'"), "2147483643\n2147483644\n2147483645\n2147483646\n2147483647\n");
    t.ok("set -g base-index 0");
    t.ok("movew -r");
    assert_eq!(t.ok("lsw -F '#{window_index}'"), "0\n1\n2\n3\n4\n");
    // A new session at the maximum base-index.
    t.ok("set -g base-index 2147483647");
    t.ok("new -d -s top");
    assert_eq!(t.ok("lsw -t top -F '#{window_index}'"), "2147483647\n");
    assert_eq!(t.err("neww -t top:"), "create window failed: no free window index");
}

/// Ids come from counters that are never reused; at the end of the range
/// a command fails instead of wrapping onto a live id (or the u32::MAX
/// placeholder the model uses for "no window yet").
#[test]
fn exhausted_ids_fail_cleanly() {
    let mut t = T::with_session();
    t.ok("splitw");
    let spawned = t.h.spawned.len();
    t.m.next_window = u32::MAX;
    // break-pane must fail before it takes the pane out of its window.
    assert_eq!(t.err("breakp"), "no free window id");
    assert_eq!(t.m.windows.len(), 1);
    assert_eq!(t.err("neww"), "create window failed: no free window id");
    t.m.next_session = u32::MAX;
    assert_eq!(t.err("new -d"), "no free session id");
    assert_eq!(t.err("new -d -s named"), "no free session id");
    t.m.next_pane = u32::MAX - 1;
    t.m.next_window = u32::MAX - 1;
    t.ok("neww");
    assert_eq!(t.err("splitw"), "no free pane id");
    assert_eq!(t.err("neww"), "create window failed: no free pane id");
    assert_eq!(t.h.spawned.len(), spawned + 1, "no process for a refused command");
    assert_eq!(t.m.sessions().count(), 1);
}

#[test]
fn copy_mode_tables_and_send_keys_x() {
    let mut t = T::with_session();
    // The default tables are tmux's (list-keys -T copy-mode-vi, tmux 3.8).
    let vi = t.ok("list-keys -T copy-mode-vi");
    for line in [
        "bind-key -T copy-mode-vi v                 send-keys -X rectangle-toggle",
        "bind-key -T copy-mode-vi Enter             send-keys -X copy-pipe-and-cancel",
        "bind-key -T copy-mode-vi /                 command-prompt -P -T search -p \"(search down)\" { send-keys -X search-forward -- \"%%\" }",
    ] {
        let want: Vec<&str> = line.split_whitespace().collect();
        assert!(vi.lines().any(|l| l.split_whitespace().collect::<Vec<_>>() == want), "{line}\n{vi}");
    }
    assert!(t.ok("list-keys -T copy-mode").contains("C-s"));
    // Rebinding and unbinding go through the model like any table.
    t.ok("bind -T copy-mode-vi y send-keys -X copy-selection-and-cancel");
    assert!(t.ok("list-keys -T copy-mode-vi y").contains("copy-selection-and-cancel"));
    t.ok("unbind -T copy-mode-vi y");
    assert!(t.ok("list-keys -T copy-mode-vi").lines().all(|l| !l.contains(" y ")));
    // send-keys -X: tmux's error outside a mode, and the call inside one.
    assert_eq!(t.err("send-keys -X cursor-up"), "not in a mode");
    let p = PaneId(0);
    t.h.in_mode.push(p);
    t.ok("send-keys -X -N 3 cursor-up");
    t.ok("send-keys -X search-backward -- foo");
    t.ok("send -t %0 -X begin-selection");
    assert_eq!(
        t.h.mode_commands,
        vec![
            (p, "cursor-up".into(), vec![], 3),
            (p, "search-backward".into(), vec!["foo".into()], 1),
            (p, "begin-selection".into(), vec![], 1),
        ]
    );
    // The flags tmux's default prompts use parse.
    assert!(parse_string("command-prompt -Pi -I \"#{pane_search_string}\" -T search -p x { send -X a }").is_ok());
    // mode-keys, word-separators and the styles are options.
    t.ok("set -g mode-keys vi");
    assert_eq!(t.ok("show -gv mode-keys"), "vi\n");
    assert_eq!(t.ok("show -gv mode-style"), "bg=yellow,fg=black\n");
    t.ok("set -g pane-active-border-style fg=red,bold");
    assert_eq!(t.ok("show -gv pane-active-border-style"), "fg=red,bold\n");
    t.ok("set -g word-separators ' -'");
    assert_eq!(t.ok("show -gv word-separators"), " -\n");
    assert_eq!(parse_style("fg=red,bold").fg, lif_types::Color::Indexed(1));
}

#[test]
fn source_file_runs_commands_and_reports_problems() {
    let mut t = T::with_session();
    t.h.configs.insert(
        "/c.toml".into(),
        ConfigFile {
            commands: vec![
                ("set -g base-index 1".into(), "/c.toml:1:1".into()),
                ("set -g no-such-option 1".into(), "/c.toml:2:1".into()),
                ("set -g status-left x".into(), "/c.toml:3:1".into()),
            ],
            diagnostics: vec![
                (false, "/c.toml:4:1: warning: something odd".into()),
                (true, "/c.toml:5:1: error: something wrong".into()),
            ],
        },
    );
    let e = t.err("source-file /c.toml");
    assert_eq!(e, "/c.toml:5:1: error: something wrong\n/c.toml:2:1: invalid option: no-such-option");
    // Everything that could run did.
    assert_eq!(t.ok("show -gv base-index"), "1\n");
    assert_eq!(t.ok("show -gv status-left"), "x\n");
    let log = t.ok("show-messages");
    assert!(log.contains("warning: something odd") && log.contains("invalid option"), "{log}");
    // Missing files: an error, unless -q.
    assert_eq!(t.err("source-file /nope.toml"), "/nope.toml: No such file or directory");
    t.ok("source-file -q /nope.toml");
    // -n only checks.
    t.ok("set -g base-index 0");
    let _ = t.run("source-file -n /c.toml");
    assert_eq!(t.ok("show -gv base-index"), "0\n");
}

#[test]
fn server_formats() {
    let mut t = T::with_session();
    t.m.set_server_info(4242, "/tmp/lif-501/default");
    assert_eq!(t.ok("display -p '#{pid} #{socket_path}'"), "4242 /tmp/lif-501/default\n");
    assert_eq!(t.ok("display -p '#{version}'"), format!("{}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn set_option_o_refuses_options_already_set() {
    // tmux 3.8: global tables hold every option, so -go always refuses;
    // a session or window table refuses only what it set itself.
    let mut t = T::with_session();
    assert_eq!(t.err("set -go status-right x"), "already set: status-right");
    assert_eq!(t.err("set -wgo mode-keys vi"), "already set: mode-keys");
    t.ok("set -o status-right x");
    assert_eq!(t.err("set -o status-right y"), "already set: status-right");
    assert_eq!(t.ok("show -v status-right"), "x\n");
    t.ok("set -go @u 1");
    assert_eq!(t.err("set -go @u 2"), "already set: @u");
}

#[test]
fn flags_that_change_behavior_work_or_fail() {
    let mut t = T::with_session();
    t.ok("new -d -s b");
    let c1 = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c1, NOW);
    t.ok("attach -t a");
    // detach -P: SIGHUP to the client's parent after detaching it.
    t.ok("detach -P");
    assert_eq!(t.h.detached.last().unwrap(), &(c1, DetachReason::Detached { session: "a".into() }));
    assert_eq!(t.h.hangups, vec![c1]);
    // attach -x: like -d, and the kicked client's parent gets SIGHUP.
    t.ok("attach -t a");
    let c2 = t.attach_client(2, 80, 25);
    t.ctx = CmdContext::client(c2, NOW);
    t.ok("attach -x -t a");
    assert_eq!(t.h.detached.last().unwrap(), &(c1, DetachReason::Kicked { session: "a".into() }));
    assert_eq!(t.h.hangups, vec![c1, c1]);
    // new-session -A -X is attach -x; -D is attach -d (no SIGHUP).
    t.ctx = CmdContext::client(c1, NOW);
    t.ok("new -A -X -s a");
    assert_eq!(t.h.detached.last().unwrap(), &(c2, DetachReason::Kicked { session: "a".into() }));
    assert_eq!(t.h.hangups, vec![c1, c1, c2]);
    t.ctx = CmdContext::client(c2, NOW);
    t.ok("new -A -D -s a");
    assert_eq!(t.h.hangups.len(), 3);
    // -E would skip update-environment, which lif does not have.
    assert_eq!(t.err("new -d -E -s e"), "new-session -E is not supported: lif has no update-environment option");
    // display-panes -N: numbers that ignore keys; -b is not a tmux 3.8 flag.
    t.ok("display-panes -N");
    assert_eq!(t.h.messages.last().unwrap(), &(c2, "display-panes select-pane -t '%%' (no keys)".to_string()));
    assert_eq!(t.err("display-panes -b"), "command display-panes: unknown flag -b");
    // command-prompt -i (incremental) only exists for copy-mode searches.
    assert!(t.err("command-prompt -i").contains("not supported"));
}

#[test]
fn switch_client_z_keeps_zoom() {
    // tmux 3.8: switching to another pane of a zoomed window unzooms it;
    // with -Z the window stays zoomed, on that pane.
    let mut t = T::with_session();
    t.ok("split-window -t a");
    t.ok("new -d -s b");
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t b");
    t.ok("resize-pane -Z -t a:0.1");
    assert_eq!(t.ok("display -p -t a:0 '#{window_zoomed_flag} #{pane_index}'"), "1 1\n");
    t.ok("switch-client -t a:0.0");
    assert_eq!(t.ok("display -p -t a:0 '#{window_zoomed_flag} #{pane_index}'"), "0 0\n");
    t.ok("resize-pane -Z -t a:0.1");
    t.ok("switch-client -Z -t a:0.0");
    assert_eq!(t.ok("display -p -t a:0 '#{window_zoomed_flag} #{pane_index}'"), "1 0\n");
}

#[test]
fn list_commands_format() {
    let mut t = T::new();
    assert_eq!(
        t.ok("list-commands -F '#{command_list_name}|#{command_list_alias}' new-session"),
        "new-session|new\n"
    );
    assert_eq!(
        t.ok("lscm -F '#{command_list_usage}' new-session"),
        "[-AdDEPX] [-c value] [-e value] [-F value] [-n value] [-s value] [-x value] [-y value] [argument ...]\n"
    );
    assert_eq!(t.ok("lscm -F '#{command_list_usage}' kill-server"), "\n");
    let all = t.ok("lscm -F '#{command_list_name}'");
    assert_eq!(all.lines().count(), t.ok("lscm").lines().count());
}


// ----- what Claude Code's tmux backend runs (agent teams) ---------------------

#[test]
fn pane_options_override_the_window() {
    // tmux 3.8: any window option can be set per pane; show -p lists only
    // the pane's own values.
    let mut t = T::with_session();
    t.ok("splitw -h");
    t.ok("set -p -t %1 remain-on-exit failed");
    t.ok("set -p -t %1 pane-border-style fg=red");
    t.ok("set -p -t %1 @who lead");
    assert_eq!(t.ok("show -p -t %1"), "pane-border-style fg=red\nremain-on-exit failed\n@who lead\n");
    assert_eq!(t.ok("show -p -t %0"), "");
    assert_eq!(t.ok("show -pv -t %1 remain-on-exit"), "failed\n");
    assert_eq!(t.err("set -p -t %1 status off"), "not a pane option: status");
    assert_eq!(t.err("set -p -t %1 nonsuch x"), "invalid option: nonsuch");
    assert_eq!(t.m.pane_option_text(PaneId(1), "pane-border-style").as_deref(), Some("fg=red"));
    assert_eq!(t.m.pane_option_text(PaneId(0), "pane-border-style").as_deref(), Some("default"));
    // remain-on-exit read per pane: %1 stays when it fails, %0 goes.
    t.m.pane_exited(PaneId(1), Some(1), &mut t.h);
    assert_eq!(t.ok("display -p -t %1 '#{pane_dead}'"), "1\n");
    t.ok("set -p -u -t %1 remain-on-exit");
    assert_eq!(t.ok("show -p -t %1"), "pane-border-style fg=red\n@who lead\n");
}

#[test]
fn select_pane_title_until_the_program_sets_one() {
    let mut t = T::with_session();
    t.ok("splitw");
    t.ok("selectp -t %0");
    t.ok("selectp -t %1 -T researcher");
    assert_eq!(t.ok("display -p '#D'"), "%0\n", "-T selects nothing");
    assert_eq!(t.ok("display -p -t %1 '#{pane_title}'"), "researcher\n");
    t.h.titles.insert(PaneId(1), "vim".into());
    assert_eq!(t.ok("display -p -t %1 '#T'"), "researcher\n");
    t.m.pane_title_reset(PaneId(1));
    assert_eq!(t.ok("display -p -t %1 '#T'"), "vim\n");
}

#[test]
fn border_status_takes_a_row_and_labels_panes() {
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 25);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach");
    t.ok("splitw -h");
    assert_eq!(t.m.client_view(c).unwrap().window_size, Size::new(80, 24));
    t.ok("set -w pane-border-status top");
    let v = t.m.client_view(c).unwrap();
    assert_eq!(v.border_status, BorderStatus::Top);
    assert_eq!(v.window_size, Size::new(80, 23));
    assert!(v.panes.iter().all(|(_, r)| r.y == 1 && r.rows == 23), "{:?}", v.panes);
    t.ok("selectp -t %0 -T lead");
    t.ok("set -p -t %0 pane-border-format '#[fg=red,bold] #{pane_title} #[default]'");
    let line = t.m.pane_border_line(PaneId(0), 80, &t.h, NOW);
    let text: String = line.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(text, " lead ");
    assert!(line[0].style.attrs.contains(lif_types::Attrs::BOLD));
    let line = t.m.pane_border_line(PaneId(1), 3, &t.h, NOW);
    let text: String = line.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(text, "1 \"", "the default format, cut to the width");
    t.ok("set -w pane-border-status off");
    assert_eq!(t.m.client_view(c).unwrap().window_size, Size::new(80, 24));
}

#[test]
fn load_and_save_buffers() {
    let mut t = T::with_session();
    t.h.stdin = Some(b"from stdin".to_vec());
    t.ok("load-buffer -w -");
    assert_eq!(t.ok("show-buffer"), "from stdin\n");
    assert_eq!(t.h.clipboard, vec![b"from stdin".to_vec()]);
    t.h.files.insert("/tmp/x".into(), b"from file".to_vec());
    t.ok("loadb -b named /tmp/x");
    assert_eq!(t.ok("showb -b named"), "from file\n");
    assert_eq!(t.err("loadb /nope"), "/nope: No such file or directory");
    t.ok("saveb -b named /tmp/y");
    t.ok("saveb -a -b named /tmp/y");
    assert_eq!(t.h.files["/tmp/y"], b"from filefrom file");
    assert_eq!(t.ok("saveb -"), "from file", "the newest buffer");
    t.ok("set -s set-clipboard off");
    t.h.stdin = Some(b"quiet".to_vec());
    t.ok("loadb -w -");
    assert_eq!(t.h.clipboard.len(), 1, "set-clipboard off keeps the clipboard");
}

#[test]
fn set_titles_string() {
    let mut t = T::with_session();
    assert_eq!(t.m.titles_string(SessionId(0), &t.h, NOW), None);
    t.ok("set -g set-titles on");
    t.h.titles.insert(PaneId(0), "vim x".into());
    assert_eq!(t.m.titles_string(SessionId(0), &t.h, NOW).as_deref(), Some("a:0:zsh - \"vim x\""));
}

// ----- commands that wait, hooks, environment (tmux 3.8 checked) ---------

impl T {
    /// Runs `s`, which must park.
    fn park(&mut self, s: &str) {
        let list = parse_string(s).unwrap();
        let o = self.m.execute_list(&list, &self.ctx, &mut self.h).unwrap_or_else(|e| panic!("{s}: {e}"));
        assert!(o.pending && o.stdout.is_empty(), "{s} did not park: {o:?}");
    }

    /// The newest job ends with `status` and `output`.
    fn job(&mut self, status: i32, output: &str) {
        let id = self.h.jobs.last().expect("a job").id;
        self.m.job_done(id, JobStatus::Exited(status), output.as_bytes(), NOW, &mut self.h);
        check_model(&self.m);
    }

    fn finished(&mut self) -> Finished {
        let mut f = self.m.take_finished();
        assert_eq!(f.len(), 1, "{f:?}");
        f.remove(0)
    }
}

#[test]
fn run_shell_parks_the_list_and_carries_on() {
    let mut t = T::with_session();
    t.park("run-shell 'echo #{session_name}' ; display -p after");
    assert_eq!(t.h.jobs.len(), 1);
    assert_eq!(t.h.jobs[0].command, "echo a", "the command is a format");
    assert_eq!((t.h.jobs[0].session, t.h.jobs[0].pane, t.h.jobs[0].stderr), (Some(SessionId(0)), Some(PaneId(0)), false));
    assert_eq!(t.m.parked(), 1);
    assert!(!t.m.has_finished());
    t.job(0, "a\n");
    assert_eq!(t.finished(), Finished { client: None, waited: true, stdout: "a\nafter\n".into(), status: 0, error: None });
    assert_eq!(t.m.parked(), 0);
    // tmux 3.8: a failing command prints this, the list goes on, and the
    // client exits with the command's status.
    t.park("run -E 'exit 3' ; display -p after");
    assert!(t.h.jobs[1].stderr);
    t.job(3, "partial");
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.status), ("partial\n'exit 3' returned 3\nafter\n", 3));
    t.park("run 'kill -9 $$'");
    let id = t.h.jobs.last().unwrap().id;
    t.m.job_done(id, JobStatus::Signaled(9), b"", NOW, &mut t.h);
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.status), ("'kill -9 $$' terminated by signal 9\n", 137));
    // An error after the wait ends the list; what ran before is kept.
    t.park("run 'echo x' ; kill-pane -t %99 ; display -p never");
    t.job(0, "x\n");
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.error.map(|e| e.0).as_deref()), ("x\n", Some("can't find pane: %99")));
    // Nothing to run is not an error; a job that cannot start is.
    assert_eq!(t.ok("run-shell"), "");
    t.h.fail_jobs = true;
    assert_eq!(t.err("run-shell 'true'"), "true: fork failed");
    assert_eq!(t.m.parked(), 0);
    assert_eq!(t.err("run-shell -t %9 true"), "can't find pane: %9");
}

#[test]
fn run_shell_background_delay_and_commands() {
    let mut t = T::with_session();
    // -b: the list does not wait.
    assert_eq!(t.ok("run-shell -b 'echo bg' ; display -p now"), "now\n");
    assert_eq!(t.m.parked(), 1);
    t.job(0, "bg\n");
    assert_eq!(t.finished(), Finished { client: None, waited: false, stdout: "bg\n".into(), status: 0, error: None });
    // -C runs commands, in place.
    assert_eq!(t.ok("run-shell -C 'display -p one' ; display -p two"), "one\ntwo\n");
    assert_eq!(t.err("run-shell -C 'nosuch'"), "unknown command: nosuch");
    // -d waits first (seconds, with a fraction).
    t.park("run-shell -d 0.25 'echo late'");
    assert_eq!(t.h.timers.len(), 1);
    assert_eq!(t.h.timers[0].1, 250);
    let jobs = t.h.jobs.len();
    t.m.job_done(t.h.timers[0].0, JobStatus::Exited(0), b"", NOW, &mut t.h);
    assert_eq!(t.h.jobs.len(), jobs + 1, "the command starts when the delay is over");
    assert!(!t.m.has_finished());
    t.job(0, "late\n");
    assert_eq!(t.finished().stdout, "late\n");
    t.park("run-shell -d 1 -C 'display -p later' ; display -p last");
    t.m.job_done(t.h.timers[1].0, JobStatus::Exited(0), b"", NOW, &mut t.h);
    assert_eq!(t.finished().stdout, "later\nlast\n");
    t.park("run-shell -d 2");
    assert_eq!(t.h.timers[2].1, 2000);
    assert_eq!(t.err("run-shell -d x"), "invalid delay time: x");
    assert_eq!(t.err("run-shell -d -1"), "invalid delay time: -1");
    assert_eq!(t.err("run-shell -d 1e300"), "invalid delay time: 1e300");
}

#[test]
fn if_shell_runs_a_branch_in_place() {
    let mut t = T::with_session();
    assert_eq!(t.ok("if-shell -F '#{session_name}' 'display -p yes' 'display -p no' ; display -p end"), "yes\nend\n");
    assert_eq!(t.ok("if -F '0' 'display -p yes' 'display -p no'"), "no\n");
    assert_eq!(t.ok("if -F '' 'display -p yes'"), "");
    assert_eq!(t.ok("if -F 1 { display -p a ; display -p b }"), "a\nb\n");
    assert!(t.h.jobs.is_empty(), "-F runs nothing");
    t.park("if-shell 'test -n #{session_name}' 'display -p yes' 'display -p no' ; display -p end");
    assert_eq!(t.h.jobs[0].command, "test -n a");
    t.job(0, "ignored output");
    assert_eq!(t.finished().stdout, "yes\nend\n");
    t.park("if-shell false 'display -p yes' 'display -p no'");
    t.job(1, "");
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.status), ("no\n", 0), "the condition's status is not the client's");
    t.park("if-shell false 'display -p yes'");
    t.job(1, "");
    assert_eq!(t.finished().stdout, "");
    // Only the branch taken is parsed (tmux 3.8).
    t.park("if-shell true 'nosuchcmd' 'alsonot'");
    t.job(0, "");
    assert_eq!(t.finished().error.map(|e| e.0).as_deref(), Some("unknown command: nosuchcmd"));
    // A branch can wait itself.
    t.park("if-shell true 'run-shell \"echo inner\"' ; display -p end");
    t.job(0, "");
    assert!(!t.m.has_finished());
    t.job(0, "inner\n");
    assert_eq!(t.finished().stdout, "inner\nend\n");
    assert_eq!(t.ok("if-shell -b true 'display -p yes' ; display -p first"), "first\n");
    t.job(0, "");
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.waited), ("yes\n", false));
    assert_eq!(t.err("if-shell true"), "command if-shell: too few arguments (need at least 2)");
}

#[test]
fn wait_for_channels_and_locks() {
    let mut t = T::with_session();
    let (a, b) = (t.attach_client(1, 80, 24), t.attach_client(2, 80, 24));
    t.ctx = CmdContext::client(a, NOW);
    t.park("wait-for ch ; display -p woke-a");
    t.ctx = CmdContext::client(b, NOW);
    t.park("wait-for ch ; display -p woke-b");
    t.ctx = CmdContext::none(NOW);
    assert_eq!(t.ok("wait-for -S other"), "", "another channel wakes nobody");
    assert!(!t.m.has_finished());
    assert_eq!(t.ok("wait-for -S ch ; display -p signalled"), "signalled\n");
    let f = t.m.take_finished();
    assert_eq!(f.iter().map(|f| (f.client, f.stdout.as_str())).collect::<Vec<_>>(), [(Some(a), "woke-a\n"), (Some(b), "woke-b\n")]);
    // Signalled ahead of the waiter (tmux 3.8 keeps that).
    assert_eq!(t.ok("wait-for other ; display -p at-once"), "at-once\n");
    t.park("wait-for other");
    // Locks: one holder, the others queue in order.
    assert_eq!(t.err("wait-for -U lk"), "channel lk not locked");
    assert_eq!(t.ok("wait-for -L lk"), "");
    t.ctx = CmdContext::client(a, NOW);
    t.park("wait-for -L lk ; display -p a-has-it");
    t.ctx = CmdContext::client(b, NOW);
    t.park("wait-for -L lk ; display -p b-has-it");
    t.ctx = CmdContext::none(NOW);
    t.ok("wait-for -U lk");
    assert_eq!(t.finished().stdout, "a-has-it\n");
    t.ok("wait-for -U lk");
    assert_eq!(t.finished().stdout, "b-has-it\n");
    t.ok("wait-for -U lk");
    assert_eq!(t.err("wait-for -U lk"), "channel lk not locked");
    // A client that goes away stops waiting.
    t.ctx = CmdContext::client(a, NOW);
    t.park("wait-for gone");
    let parked = t.m.parked();
    t.m.client_disconnect(a, &mut t.h);
    assert_eq!(t.m.parked(), parked - 1);
    t.ctx = CmdContext::none(NOW);
    assert_eq!(t.err("wait-for"), "command wait-for: too few arguments (need at least 1)");
}

#[test]
fn waiting_inside_source_file_does_not_hold_it_up() {
    let mut t = T::with_session();
    let file = ConfigFile {
        commands: vec![("run-shell 'echo slow' ; set -g @after-run 1".into(), "f:1:1".into()), ("set -g @next 1".into(), "f:2:1".into())],
        diagnostics: Vec::new(),
    };
    t.h.configs.insert("/f".into(), file);
    assert_eq!(t.ok("source-file /f ; display -p '#{@next}#{@after-run}'"), "1\n", "the file's other lines ran, the list goes on");
    t.job(0, "slow\n");
    assert_eq!(t.ok("display -p '#{@after-run}'"), "1\n", "the rest of the line ran when the command was done");
    let f = t.finished();
    assert!(!f.waited);
}

#[test]
fn parked_lists_are_bounded() {
    let mut t = T::with_session();
    for _ in 0..1024 {
        t.park("wait-for never");
    }
    assert_eq!(t.err("wait-for never"), "too many waiting commands (1024)");
    assert_eq!(t.err("run-shell true"), "too many waiting commands (1024)");
    assert!(t.h.jobs.is_empty(), "nothing is started that could not be waited for");
    t.ok("wait-for -S never");
    assert_eq!(t.m.take_finished().len(), 1024);
    assert_eq!(t.m.parked(), 0);
}

#[test]
fn format_shell_commands_show_their_last_output() {
    let mut t = T::with_session();
    let at = |secs: i64| WallClock::new(NOW.unix_secs + secs, NOW.utc_offset_secs);
    let show = |t: &mut T, secs: i64| t.m.expand_format("[#(uptime #{session_name})]", Some(SessionId(0)), None, None, None, at(secs), &t.h);
    let uses = t.m.format_job_uses();
    assert_eq!(show(&mut t, 0), "[]", "nothing until it has run once");
    assert_ne!(t.m.format_job_uses(), uses);
    assert!(t.h.jobs.is_empty(), "expanding starts nothing itself");
    t.m.start_format_jobs(at(0), &mut t.h);
    assert_eq!(t.h.jobs.len(), 1);
    assert_eq!((t.h.jobs[0].command.as_str(), t.h.jobs[0].session), ("uptime a", Some(SessionId(0))));
    assert_eq!(show(&mut t, 1), "[]");
    t.m.start_format_jobs(at(1), &mut t.h);
    assert_eq!(t.h.jobs.len(), 1, "not again while it runs");
    t.job(0, "first line\nload 1.5\n\n");
    assert!(!t.m.has_finished(), "a format's command is nobody's command");
    assert_eq!(show(&mut t, 2), "[load 1.5]", "the last line that is not empty");
    t.m.start_format_jobs(at(2), &mut t.h);
    assert_eq!(t.h.jobs.len(), 1, "not before status-interval (15 s) has passed");
    assert_eq!(show(&mut t, 15), "[load 1.5]");
    t.m.start_format_jobs(at(15), &mut t.h);
    assert_eq!(t.h.jobs.len(), 2);
    t.job(0, "load 2.0\n");
    assert_eq!(show(&mut t, 16), "[load 2.0]");
    // More commands than are remembered: the one used longest ago goes.
    for i in 0..64 {
        t.m.expand_format(&format!("#(echo {i})"), None, None, None, None, at(20), &t.h);
    }
    assert_eq!(show(&mut t, 21), "[]", "forgotten, so empty again");
}

#[test]
fn hooks_run_after_commands_and_events() {
    let mut t = T::with_session();
    assert_eq!(t.err("set-hook -g bogus-hook 'display x'"), "invalid option: bogus-hook");
    assert_eq!(t.err("set-hook -g after-new-window"), "empty value");
    assert_eq!(t.err("show-hooks -g nosuch"), "invalid option: nosuch");
    assert_eq!(t.ok("show-hooks"), "");
    let all = t.ok("show-hooks -g");
    assert_eq!(all.lines().count(), 62, "tmux 3.8 has 62 session hooks");
    assert!(all.starts_with("after-bind-key\nafter-capture-pane\n") && all.ends_with("window-linked\nwindow-unlinked\n"));
    assert_eq!(t.ok("show-hooks -gw").lines().count(), 27);
    // tmux 3.8 prints the command as it parsed it; the hook's output joins
    // the command's.
    t.ok("set-hook -g after-new-window 'display -p \"hooked #{hook} #{hook_session_name} #{hook_window} #{hook_pane} #{pane_id}\"'");
    assert_eq!(t.ok("show-hooks -g after-new-window"), "after-new-window[0] display-message -p \"hooked #{hook} #{hook_session_name} #{hook_window} #{hook_pane} #{pane_id}\"\n");
    assert_eq!(t.ok("new-window -d"), "hooked after-new-window a @1 %1 %1\n", "about the new pane, which is its current pane");
    t.ok("set-hook -ga after-new-window 'display -p second'");
    assert_eq!(t.ok("new-window -d ; display -p cmd"), "cmd\nhooked after-new-window a @2 %2 %2\nsecond\n", "hooks run when the list is done");
    t.ok("set-hook -gu 'after-new-window[0]'");
    assert_eq!(t.ok("show-hooks -g after-new-window"), "after-new-window[1] display-message -p second\n");
    // A session's own setting replaces the global one.
    t.ok("set-hook after-new-window 'display -p mine'");
    assert_eq!(t.ok("show-hooks"), "after-new-window[0] display-message -p mine\n");
    assert_eq!(t.ok("new-window -d"), "mine\n");
    t.ok("set-hook -u after-new-window ; set-hook -gu after-new-window");
    assert_eq!(t.ok("new-window -d"), "");
    // Commands of a hook fire no hooks.
    t.ok("set-hook -g after-new-window 'new-window -d'");
    let before = t.ok("list-windows").lines().count();
    t.ok("new-window -d");
    assert_eq!(t.ok("list-windows").lines().count(), before + 2);
    t.ok("set-hook -gu after-new-window");
    // Events.
    t.ok("set-hook -g session-renamed 'display -p \"renamed to #{hook_session_name}\"'");
    assert_eq!(t.ok("rename-session b"), "renamed to b\n");
    t.ok("set-hook -g session-closed 'set -gF @closed \"#{hook_session_name}\"'");
    t.ok("new -d -s doomed ; kill-session -t doomed");
    assert_eq!(t.ok("display -p '#{@closed}'"), "doomed\n");
    t.ok("set-hook -g window-renamed 'set -gF @w \"#{hook_window_name}\"' ; rename-window -t b:0 ed");
    assert_eq!(t.ok("display -p '#{@w}'"), "ed\n");
    // Outside commands: the server runs them.
    t.ok("set-hook -g pane-exited 'set -gF @exited \"#{hook_pane}\"' ; split-window -d -t b:0");
    let p = t.m.window(WindowId(0)).unwrap().panes()[1];
    t.m.pane_exited(p, Some(0), &mut t.h);
    assert!(t.m.has_hooks());
    t.m.run_hooks(NOW, &mut t.h);
    assert!(!t.m.has_hooks());
    assert_eq!(t.ok("display -p '#{@exited}'"), format!("{p}\n"));
    // On a pane, for window hooks; -R runs one now.
    t.ok("set-hook -p -t b:0.0 pane-exited 'display -p pane-own'");
    assert_eq!(t.ok("show-hooks -p -t b:0.0"), "pane-exited[0] display-message -p pane-own\n");
    assert_eq!(t.ok("show-hooks -w -t b:0"), "");
    assert_eq!(t.ok("set-hook -R -t b:0.0 pane-exited"), "pane-own\n");
    // A list that waits takes along what its hooks printed so far.
    t.ok("set-hook -g after-rename-window 'display -p renamed-hook'");
    t.park("rename-window -t b:0 w1 ; run-shell 'echo job' ; display -p end");
    t.job(0, "job\n");
    assert_eq!(t.finished().stdout, "renamed-hook\njob\nend\n");
    // A hook that waits finishes on its own, reporting once.
    t.ok("set-hook -g after-rename-window 'run-shell \"echo a\" ; run-shell \"echo b\"'");
    assert_eq!(t.ok("rename-window -t b:0 w2"), "");
    t.job(0, "a\n");
    let f = t.finished();
    assert_eq!((f.stdout.as_str(), f.waited), ("a\n", false), "what it has so far, for whoever can show it");
    t.job(0, "b\n");
    assert_eq!(t.finished().stdout, "b\n");
    // A hook that fails is logged, not an error of the command.
    t.ok("set-hook -g after-rename-window 'kill-pane -t %99'");
    assert_eq!(t.ok("rename-window -t b:0 again"), "");
    assert!(t.m.messages().any(|m| m == "can't find pane: %99"));
}

#[test]
fn environment_commands() {
    let mut t = T::with_session();
    assert_eq!(t.err("setenv '' x"), "empty variable name");
    assert_eq!(t.err("setenv A=B x"), "variable name contains =");
    assert_eq!(t.err("setenv NOVAL"), "no value specified");
    assert_eq!(t.err("setenv -u X y"), "can't specify a value with -u");
    assert_eq!(t.err("setenv -r X y"), "can't specify a value with -r");
    assert_eq!(t.err("showenv NOPE"), "unknown variable: NOPE");
    t.ok("setenv FOO bar ; setenv -g GFOO 'g \"x\" $y' ; setenv -F FF '#{session_name}' ; setenv -h HID 1 ; setenv -r GONE");
    assert_eq!(t.ok("showenv"), "FF=a\nFOO=bar\n-GONE\n");
    assert_eq!(t.ok("showenv -h"), "HID=1\n");
    assert_eq!(t.ok("showenv -g"), "GFOO=g \"x\" $y\n");
    assert_eq!(t.ok("showenv -gs"), "GFOO=\"g \\\"x\\\" \\$y\"; export GFOO;\n");
    assert_eq!(t.ok("showenv -s GONE"), "unset GONE;\n");
    assert_eq!(t.ok("showenv FOO"), "FOO=bar\n");
    // New panes and shell commands get them: global, session, then -e.
    t.ok("new-window -d -e FOO=flag -e EXTRA=1 -e ignored");
    let env = &t.h.spawned.last().unwrap().env;
    let want: Vec<(String, Option<String>)> = [("GFOO", Some("g \"x\" $y")), ("FF", Some("a")), ("FOO", Some("bar")), ("GONE", None), ("FOO", Some("flag")), ("EXTRA", Some("1"))]
        .iter()
        .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
        .collect();
    assert_eq!(*env, want, "hidden ones stay out; later entries win");
    t.ok("split-window -d -e S=1");
    assert_eq!(t.h.spawned.last().unwrap().env.last(), Some(&("S".to_string(), Some("1".to_string()))));
    t.park("run-shell true");
    assert_eq!(t.h.jobs[0].env.len(), 4);
    t.ok("setenv -u FOO ; setenv -gu GFOO");
    assert_eq!(t.ok("showenv"), "FF=a\n-GONE\n");
    assert_eq!(t.ok("showenv -g"), "");
    // Another session has its own.
    t.ok("new -d -s other");
    assert_eq!(t.ok("showenv -t other"), "");
}

#[test]
fn join_pane_moves_a_pane_between_windows() {
    // Sizes and order as tmux 3.8 gives them for the same commands.
    let mut t = T::with_session();
    t.ok("new-window -d ; split-window -d -t a:0");
    assert_eq!(t.err("join-pane -s a:1.0 -t a:1.0"), "source and target panes must be different");
    t.ok("join-pane -h -s a:0.1 -t a:1.0");
    assert_eq!(
        t.ok("list-panes -a -F '#{window_index}.#{pane_index} #{pane_id} #{pane_width}x#{pane_height} #{pane_active}'"),
        "0.0 %0 80x24 1\n1.0 %1 40x24 0\n1.1 %2 39x24 1\n"
    );
    assert_eq!(t.ok("display -p '#{window_index}'"), "1\n", "the target's window becomes current");
    // The last pane of a window: the window goes.
    t.ok("join-pane -d -s a:0.0 -t a:1.0");
    assert_eq!(
        t.ok("list-panes -a -F '#{window_index}.#{pane_index} #{pane_id} #{pane_width}x#{pane_height} #{pane_active}'"),
        "1.0 %1 40x12 0\n1.1 %0 40x11 0\n1.2 %2 39x24 1\n"
    );
    assert!(t.h.killed.is_empty(), "nothing was killed");
    // Inside one window, before the target.
    t.ok("join-pane -b -s a:1.2 -t a:1.0");
    assert_eq!(t.ok("list-panes -t a:1 -F '#{pane_id}'"), "%2\n%1\n%0\n");
    // A pane that does not fit leaves everything as it was.
    t.ok("new -d -s tiny -x 80 -y 2");
    let before = t.ok("list-panes -a -F '#{pane_id} #{window_id} #{pane_width}x#{pane_height}'");
    assert_eq!(t.err("join-pane -s a:1.0 -t tiny:0.0"), t.err("split-window -t tiny:0.0"), "what split-window says there");
    assert_eq!(t.ok("list-panes -a -F '#{pane_id} #{window_id} #{pane_width}x#{pane_height}'"), before);
    // The last window of a session: the session goes.
    t.ok("new -d -s one");
    t.ok("join-pane -s one:0.0 -t a:1.0");
    assert_eq!(t.err("has-session -t one"), "can't find session: one");
}

#[test]
fn synchronize_panes_and_clear_history() {
    let mut t = T::with_session();
    t.ok("split-window -d ; split-window -d");
    t.ok("send-keys -t a:0.0 -l solo");
    assert_eq!(t.h.sent.len(), 1);
    assert_eq!(t.ok("display -p '#{pane_synchronized}'"), "0\n");
    t.ok("set -w synchronize-panes on");
    assert_eq!(t.ok("display -p '#{pane_synchronized}'"), "1\n");
    t.h.sent.clear();
    t.h.in_mode.push(PaneId(2));
    t.ok("send-keys -t a:0.0 -l all Enter");
    let got: Vec<(u32, &str)> = t.h.sent.iter().map(|(p, s)| (p.0, s.as_str())).collect();
    assert_eq!(got, [(0, "text:all"), (1, "text:all"), (0, "text:Enter"), (1, "text:Enter")], "not the pane in a mode");
    // One pane can stay out (set -p).
    t.h.in_mode.clear();
    t.h.sent.clear();
    t.ok("set -p -t %1 synchronize-panes off ; send-keys -t a:0.0 -l x");
    assert_eq!(t.h.sent.iter().map(|(p, _)| p.0).collect::<Vec<_>>(), [0, 2]);
    t.ok("clear-history -t %1");
    assert_eq!(t.h.cleared, [PaneId(1)]);
    assert_eq!(t.err("clear-history -t nosuch"), "can't find pane: nosuch");
}

#[test]
fn new_window_from_inside_a_pane_takes_a_free_index() {
    // A client that is not attached but runs in a pane (LIF_PANE): its
    // current window is the pane's, which is no reason to reuse its index.
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 24);
    t.ctx = CmdContext { client: Some(c), pane: Some(PaneId(0)), now: NOW };
    assert_eq!(t.ok("new-window -d -P"), "a:1.0\n");
    assert_eq!(t.err("new-window -d -t a:0"), "create window failed: index 0 in use");
}

// ----- chooser modes (layout and keys read off tmux 3.8's screen) ---------

fn key(name: &str) -> KeyEvent {
    KeyCode::parse(name).unwrap_or_else(|| panic!("key {name}")).to_event()
}

impl T {
    /// Runs a chooser command for an attached client and opens its mode.
    fn chooser(&mut self, cmd: &str) -> ChooseState {
        self.ok(cmd);
        let (client, _, spec) = self.h.choosers.pop().expect("a chooser");
        let items = self.m.choose_items(&spec, Some(client), NOW, &self.h);
        let start = self.m.choose_start(spec.kind, client);
        ChooseState::new(spec, items, start.as_deref())
    }

    fn rebuild(&mut self, s: &mut ChooseState) {
        let items = self.m.choose_items(s.spec(), None, NOW, &self.h);
        s.set_items(items);
    }
}

fn lines(s: &mut ChooseState) -> Vec<String> {
    s.screen(80, 24).lines.into_iter().map(|l| format!("{}{}", if l.selected { ">" } else { " " }, l.text)).collect()
}

fn tree_setup() -> (T, ClientId) {
    let mut t = T::with_session();
    t.ok("new-window -d -n second ; split-window -d -t a:0 ; new -d -s b");
    let c = t.attach_client(1, 80, 24);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t a");
    t.h.titles.insert(PaneId(0), "vim".into());
    (t, c)
}

#[test]
fn choose_tree_lists_sessions_windows_and_panes() {
    let (mut t, c) = tree_setup();
    assert_eq!(t.err("choose-tree -O bogus"), "invalid sort order: bogus");
    let mut s = t.chooser("choose-tree");
    assert_eq!(
        lines(&mut s),
        [
            " (0) - a: 2 windows (attached)",
            " (1) ├─→ - 0: zsh*",
            ">(2) │   ├─→ 0: zsh*: \"vim\"",
            " (3) │   └─→ 1: zsh: \"box.local\"",
            " (4) └─→ - 1: second: \"box.local\"",
            " (5)     └─→ 0: zsh*: \"box.local\"",
            " (6) - b: 1 windows",
            " (7) └─→ - 0: zsh*: \"box.local\"",
            " (8)     └─→ 0: zsh*: \"box.local\"",
        ],
        "everything open, the client's pane selected"
    );
    // -s and -w start collapsed, on the nearest line that is shown.
    let mut s = t.chooser("choose-tree -s");
    assert_eq!(lines(&mut s), [">(0) + a: 2 windows (attached)", " (1) + b: 1 windows"]);
    let mut s = t.chooser("choose-tree -w");
    assert_eq!(
        lines(&mut s),
        [
            " (0) - a: 2 windows (attached)",
            ">(1) ├─→ + 0: zsh*",
            " (2) └─→ + 1: second: \"box.local\"",
            " (3) - b: 1 windows",
            " (4) └─→ + 0: zsh*: \"box.local\""
        ]
    );
    // The screen: half the pane for a short list, a box with a preview.
    let scr = s.screen(80, 23);
    assert_eq!((scr.list_rows, scr.title.as_deref()), (11, Some("0 (sort: index) (view: preview)")));
    assert_eq!(scr.preview, Preview::Panes(vec![(PaneId(0), "0:vim".into()), (PaneId(2), "1:box.local".into())]));
    assert_eq!(s.screen(80, 11).title, None, "no preview in a small pane");
    // Moving, expanding, collapsing.
    assert_eq!(s.key(&key("Down")), ChooseOutcome::Redraw);
    assert_eq!(s.key(&key("Right")), ChooseOutcome::Redraw);
    assert_eq!(lines(&mut s)[2..4], [">(2) └─→ - 1: second: \"box.local\"", " (3)     └─→ 0: zsh*: \"box.local\""]);
    s.key(&key("Down"));
    s.key(&key("Left"));
    assert_eq!(lines(&mut s)[2], ">(2) └─→ - 1: second: \"box.local\"", "left on a leaf goes to its parent");
    s.key(&key("M--"));
    assert_eq!(lines(&mut s), [">(0) + a: 2 windows (attached)", " (1) + b: 1 windows"]);
    s.key(&key("Up"));
    assert_eq!(lines(&mut s)[1], ">(1) + b: 1 windows", "one step up from the top wraps");
    // Choosing: Enter, or the shortcut on the line.
    assert_eq!(s.key(&key("Enter")), ChooseOutcome::Choose("switch-client -t '=b:'".into()));
    assert_eq!(s.key(&key("0")), ChooseOutcome::Choose("switch-client -t '=a:'".into()));
    s.key(&key("M-+"));
    assert_eq!(s.key(&key("3")), ChooseOutcome::Choose("switch-client -t '=a:0.%2'".into()));
    assert_eq!(s.key(&key("q")), ChooseOutcome::Exit);
    // A template of the caller's.
    let mut s = t.chooser("choose-tree -s 'kill-session -t \"%%\"'");
    assert_eq!(s.key(&key("Enter")), ChooseOutcome::Choose("kill-session -t \"=a:\"".into()));
    let _ = c;
}

#[test]
fn choose_tree_tags_kills_searches_and_filters() {
    let (mut t, _) = tree_setup();
    let mut s = t.chooser("choose-tree -w");
    // x asks, y kills, the list follows the model.
    assert_eq!(s.key(&key("x")), ChooseOutcome::Prompt { label: "Kill window 0?".into() });
    assert!(s.confirming());
    assert_eq!(s.prompt_accept("n"), ChooseOutcome::Redraw);
    s.key(&key("x"));
    let ChooseOutcome::Run(cmds) = s.prompt_accept("y") else { panic!("not run") };
    assert_eq!(cmds, ["kill-window -t @0"]);
    t.ok(&cmds[0]);
    t.rebuild(&mut s);
    assert_eq!(lines(&mut s), [" (0) - a: 1 windows (attached)", ">(1) └─→ + 1: second*: \"box.local\"", " (2) - b: 1 windows", " (3) └─→ + 0: zsh*: \"box.local\""]);
    // Tags: t tags and moves on, X kills the tagged, : runs a command for each.
    s.key(&key("t"));
    assert_eq!(lines(&mut s)[1..3], [" (1) └─→ + 1*: second*: \"box.local\"", ">(2) - b: 1 windows"]);
    s.key(&key("t"));
    assert_eq!(s.key(&key(":")), ChooseOutcome::Prompt { label: "(2 tagged)".into() });
    assert_eq!(s.prompt_accept("display -p '%%'"), ChooseOutcome::Run(vec!["display -p '=a:1.'".into(), "display -p '=b:'".into()]));
    assert_eq!(s.key(&key("X")), ChooseOutcome::Prompt { label: "Kill 2 tagged?".into() });
    assert_eq!(s.prompt_cancel(), ChooseOutcome::Redraw);
    s.key(&key("T"));
    assert_eq!(s.key(&key(":")), ChooseOutcome::Prompt { label: "(current)".into() });
    s.prompt_cancel();
    // Search: the next line that contains it, opening what hides it.
    assert_eq!(s.key(&key("C-s")), ChooseOutcome::Prompt { label: "(search)".into() });
    assert_eq!(s.prompt_accept("SECOND"), ChooseOutcome::Redraw);
    assert_eq!(lines(&mut s)[1], ">(1) └─→ + 1: second*: \"box.local\"");
    assert_eq!(s.key(&key("n")), ChooseOutcome::Redraw, "the only match again");
    // Sort order and reverse ask for a new list.
    assert_eq!(s.key(&key("O")), ChooseOutcome::Rebuild);
    assert_eq!(s.spec().sort, 1);
    assert_eq!(s.key(&key("r")), ChooseOutcome::Rebuild);
    t.rebuild(&mut s);
    assert_eq!(lines(&mut s)[0], " (0) - b: 1 windows", "by name, reversed");
    assert_eq!(s.screen(80, 23).title.as_deref(), Some("1 (sort: name, reversed) (view: preview)"));
    // A filter; one that leaves nothing is ignored.
    assert_eq!(s.key(&key("f")), ChooseOutcome::Prompt { label: "(filter)".into() });
    assert_eq!(s.prompt_accept("#{==:#{session_name},a}"), ChooseOutcome::Rebuild);
    t.rebuild(&mut s);
    assert_eq!(lines(&mut s), [" (0) - a: 1 windows (attached)", ">(1) └─→ + 1: second*: \"box.local\""]);
    s.key(&key("f"));
    s.prompt_accept("0");
    t.rebuild(&mut s);
    assert_eq!(lines(&mut s).len(), 4);
    assert_eq!(s.key(&key("c")), ChooseOutcome::Rebuild);
    // The mouse: a click selects, a double click chooses, the wheel moves.
    assert_eq!(s.mouse(Some(0), false, 0), ChooseOutcome::Redraw);
    assert!(matches!(s.mouse(Some(0), true, 0), ChooseOutcome::Choose(_)));
    assert_eq!(s.mouse(None, false, 1), ChooseOutcome::Redraw);
    assert_eq!(s.mouse(Some(20), false, 0), ChooseOutcome::None, "below the list");
    // Everything gone: nothing left to show.
    t.ok("kill-session -t b ; kill-session -t a");
    t.rebuild(&mut s);
    assert!(s.is_empty());
}

#[test]
fn choose_buffer_client_and_find_window() {
    let (mut t, c) = tree_setup();
    t.ok("set-buffer -b named 'hello world' ; set-buffer 'second buffer'");
    let mut s = t.chooser("choose-buffer");
    assert_eq!(lines(&mut s), [">(0) buffer0000: 21:50: second buffer", " (1) named: 21:50: hello world"]);
    let scr = s.screen(80, 23);
    assert_eq!(scr.title.as_deref(), Some("buffer0000 (sort: creation)"));
    assert_eq!(scr.preview, Preview::Text("second buffer".into()));
    assert_eq!(s.key(&key("Enter")), ChooseOutcome::Choose("paste-buffer -p -b 'buffer0000'".into()));
    s.key(&key("Down"));
    assert_eq!(s.key(&key("d")), ChooseOutcome::Run(vec!["delete-buffer -b 'named'".into()]));
    assert_eq!(s.key(&key("O")), ChooseOutcome::Rebuild);
    t.rebuild(&mut s);
    assert_eq!(lines(&mut s)[0], " (0) buffer0000: 21:50: second buffer", "by name");
    // Clients: the attached ones.
    let mut s = t.chooser("choose-client");
    assert_eq!(lines(&mut s), [">(0) /dev/ttys001: 21:50: session a"]);
    assert_eq!(s.key(&key("Enter")), ChooseOutcome::Choose("detach-client -t '/dev/ttys001'".into()));
    assert_eq!(s.key(&key("x")), ChooseOutcome::Run(vec!["detach-client -P -t '/dev/ttys001'".into()]));
    assert_eq!(s.key(&key("z")), ChooseOutcome::Run(vec!["suspend-client -t '/dev/ttys001'".into()]));
    // find-window: the tree with the windows that match (name, title,
    // visible text); no match shows everything, as tmux does.
    let mut s = t.chooser("find-window sec");
    assert_eq!(lines(&mut s), [">(0) - a: 2 windows (attached)", " (1) └─→ - 1: second: \"box.local\"", " (2)     └─→ 0: zsh*: \"box.local\""]);
    s.key(&key("Down"));
    assert_eq!(s.key(&key("Enter")), ChooseOutcome::Choose("switch-client -t '=a:1.'".into()));
    let mut s = t.chooser("find-window -T vim");
    assert_eq!(lines(&mut s)[1], " (1) └─→ - 0: zsh*");
    let mut s = t.chooser("find-window -N vim");
    assert_eq!(lines(&mut s).len(), 9, "no window is named vim");
    let mut s = t.chooser("find-window -i -C CAPTURED");
    assert_eq!(lines(&mut s).len(), 9, "every pane's text matches");
    assert_eq!(t.err("find-window -r x"), "find-window -r is not supported: lif matches with shell patterns");
    // -Z zooms the pane for the mode, and only then is there a zoom to undo.
    t.ok("choose-tree -Z -t a:0.0");
    let (_, pane, spec) = t.h.choosers.pop().unwrap();
    assert!(spec.zoomed);
    assert_eq!(t.ok("display -p -t a:0 '#{window_zoomed_flag}'"), "1\n");
    t.m.chooser_closed(pane, spec.zoomed, &mut t.h);
    assert_eq!(t.ok("display -p -t a:0 '#{window_zoomed_flag}'"), "0\n");
    t.ok("choose-tree -Z -t a:1.0");
    assert!(!t.h.choosers.pop().unwrap().2.zoomed, "one pane: nothing to zoom");
    // Without a client there is nobody to show it to.
    t.m.client_disconnect(c, &mut t.h);
    t.ctx = CmdContext::none(NOW);
    assert_eq!(t.err("choose-tree"), "no clients");
}

// ----- the smaller commands of 0.3 (tmux 3.8 checked) ---------------------

#[test]
fn split_window_full_size() {
    // Sizes as tmux 3.8 gives them for the same commands (in a window one
    // row higher: this session has no client, so no status line).
    let mut t = T::with_session();
    t.ok("split-window -d -h ; split-window -d -f");
    let fmt = "list-panes -F '#{pane_index} #{pane_left},#{pane_top} #{pane_width}x#{pane_height}'";
    assert_eq!(t.ok(fmt), "0 0,0 40x12\n1 41,0 39x12\n2 0,13 80x11\n");
    t.ok("split-window -d -f -h -b -l 10");
    assert_eq!(t.ok(fmt), "0 0,0 10x24\n1 11,0 34x12\n2 46,0 34x12\n3 11,13 69x11\n");
    // A root split the same way takes the pane as one more child.
    let mut t = T::with_session();
    t.ok("split-window -d -h ; split-window -d -h -f -l 20");
    assert_eq!(t.ok(fmt), "0 0,0 29x24\n1 30,0 29x24\n2 60,0 20x24\n");
    t.ok("new -d -s tiny -x 80 -y 2");
    assert_eq!(t.err("split-window -f -t tiny"), t.err("split-window -t tiny"));
}

#[test]
fn resize_and_respawn_window() {
    let mut t = T::with_session();
    t.ok("new-window -d");
    t.ok("resize-window -t a:1 -x 100 -y 30");
    assert_eq!(t.ok("display -p -t a:1 '#{window_width}x#{window_height}'"), "100x30\n");
    assert_eq!(t.ok("show -w -t a:1 window-size"), "window-size manual\n");
    assert_eq!(t.ok("show -w -t a:0 window-size"), "", "only that window");
    t.ok("resize-window -t a:1 -L 5 ; resize-window -t a:1 -D");
    assert_eq!(t.ok("display -p -t a:1 '#{window_width}x#{window_height}'"), "95x31\n");
    t.ok("resize-window -t a:1 -R 3 ; resize-window -t a:1 -U 11");
    assert_eq!(t.ok("display -p -t a:1 '#{window_width}x#{window_height}'"), "98x20\n");
    assert_eq!(t.err("resize-window -t a:1 -x 5000000"), "width too large");
    assert_eq!(t.err("resize-window -t a:1 -y 0"), "height too small");
    assert_eq!(t.err("resize-window -t a:1 -U x"), "adjustment invalid");
    // A client's size does not move a manual window; -A takes the largest client's.
    let c = t.attach_client(1, 120, 40);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t a");
    assert_eq!(t.ok("display -p -t a:1 '#{window_width}x#{window_height}'"), "98x20\n");
    t.ok("resize-window -t a:1 -A");
    assert_eq!(t.ok("display -p -t a:1 '#{window_width}x#{window_height}'"), "120x39\n");
    // respawn-window: one pane again, running the new command.
    t.ok("split-window -d -t a:1 ; split-window -d -t a:1");
    assert_eq!(t.err("respawn-window -t a:1"), "respawn window failed: window a:1 still active");
    let spawned = t.h.spawned.len();
    t.ok("respawn-window -k -c /tmp -e A=1 -t a:1 'echo again'");
    assert_eq!(t.ok("list-panes -t a:1 -F '#{pane_id} #{pane_start_command} #{pane_start_path}'"), "%1 echo again /tmp\n");
    assert_eq!(t.h.spawned.len(), spawned + 1);
    let spec = t.h.spawned.last().unwrap();
    assert!(spec.respawn && spec.env.last() == Some(&("A".to_string(), Some("1".to_string()))));
    assert_eq!(t.h.killed.len(), 3, "two panes closed, the first one's process replaced");
}

#[test]
fn pipe_pane_input_off_clock_and_prompt_history() {
    let mut t = T::with_session();
    t.ok("pipe-pane 'cat > #{session_name}.log'");
    assert_eq!(t.h.pipes[&PaneId(0)], "cat > a.log", "the command is a format");
    assert_eq!(t.ok("display -p '#{pane_pipe}'"), "1\n");
    // -o toggles: with a pipe open it only closes it.
    t.ok("pipe-pane -o 'cat > other'");
    assert_eq!(t.ok("display -p '#{pane_pipe}'"), "0\n");
    t.ok("pipe-pane -o 'cat > other'");
    assert_eq!(t.h.pipes[&PaneId(0)], "cat > other");
    // A new command replaces the old one; no command stops it.
    t.h.pipe_calls.clear();
    t.ok("pipe-pane 'cat > third'");
    assert_eq!(t.h.pipe_calls, [(PaneId(0), None), (PaneId(0), Some("cat > third".into()))]);
    t.ok("pipe-pane");
    assert!(t.h.pipes.is_empty());
    assert!(t.err("pipe-pane -I x").starts_with("pipe-pane -I is not supported"));
    // select-pane -d and -e.
    assert_eq!(t.ok("display -p '#{pane_input_off}'"), "0\n");
    t.ok("split-window -d ; select-pane -d -t %1");
    assert!(t.m.pane_input_off(PaneId(1)) && !t.m.pane_input_off(PaneId(0)));
    assert_eq!(t.ok("display -p -t %1 '#{pane_input_off} #{pane_active}'"), "1 0\n", "nothing is selected");
    t.ok("select-pane -e -t %1");
    assert!(!t.m.pane_input_off(PaneId(1)));
    // clock-mode and the prompt history go to the host.
    t.ok("clock-mode -t %1");
    assert_eq!(t.h.clocks, [PaneId(1)]);
    assert_eq!(t.ok("show-prompt-history"), "History for command:\n\n\nHistory for search:\n\n\n");
    t.h.history = vec!["display hi".into(), "ls".into()];
    assert_eq!(t.ok("show-prompt-history -T command"), "History for command:\n\n1: display hi\n2: ls\n\n");
    assert_eq!(t.ok("showphist -T search"), "History for search:\n\n\n");
    assert_eq!(t.err("show-prompt-history -T bogus"), "invalid type: bogus");
    t.ok("clear-prompt-history -T search");
    assert_eq!(t.h.history.len(), 2, "another type's history stays");
    t.ok("clearphist");
    assert!(t.h.history.is_empty());
    // The default bindings have tmux's notes.
    assert_eq!(t.ok("list-keys -1N c"), "C-b c Create a new window\n");
    assert_eq!(t.ok("list-keys -1N t"), "C-b t Show a clock\n");
}

// ----- popups and menus (geometry and statuses read off tmux 3.8) ---------

fn overlay_setup() -> (T, ClientId) {
    let mut t = T::with_session();
    let c = t.attach_client(1, 80, 24);
    t.ctx = CmdContext::client(c, NOW);
    t.ok("attach -t a");
    (t, c)
}

#[test]
fn display_popup_places_and_waits() {
    let (mut t, c) = overlay_setup();
    let rect = |t: &mut T, cmd: &str| {
        t.park(cmd);
        let r = t.h.popups.last().unwrap().1.rect;
        (r.x, r.y, r.cols, r.rows)
    };
    // Half the screen, tmux's centre (one up and left of the middle).
    assert_eq!(rect(&mut t, "display-popup"), (19, 5, 40, 12));
    assert_eq!(rect(&mut t, "display-popup -w 20 -h 6"), (29, 8, 20, 6));
    assert_eq!(rect(&mut t, "display-popup -w 21 -h 7"), (29, 7, 21, 7));
    // -y is the row below the popup; R the right edge, S the status line.
    assert_eq!(rect(&mut t, "display-popup -w 30 -h 8 -x 5 -y 12"), (5, 4, 30, 8));
    assert_eq!(rect(&mut t, "display-popup -w 50% -h 50% -x R -y S"), (40, 11, 40, 12));
    assert_eq!(rect(&mut t, "display-popup -w 20 -h 5 -x 0 -y 0"), (0, 0, 20, 5), "moved to fit");
    assert_eq!(rect(&mut t, "display-popup -w 20 -h 5 -x 70 -y 40"), (60, 19, 20, 5));
    assert_eq!(rect(&mut t, "display-popup -w 20 -h 5 -x P -y P"), (0, 18, 20, 5), "by the pane");
    assert_eq!(t.err("display-popup -w 0"), "width too small");
    assert_eq!(t.err("display-popup -w 500 -h 500"), "height too large");
    assert_eq!(t.err("display-popup -w 500"), "width too large");
    assert_eq!(t.err("display-popup -x left"), "invalid position: left");
    assert_eq!(t.err("display-popup -b wavy"), "invalid border lines: wavy");
    assert_eq!(t.err("display-popup -t nosuch"), "can't find pane: nosuch");
    // What the host gets.
    t.h.popups.clear();
    t.ok("setenv -g FROM global");
    t.park("display-popup -E -b rounded -T ' #{session_name} ' -d /tmp -e A=1 -s bg=blue -S fg=red 'make test' ; display -p after");
    let (client, p) = t.h.popups.pop().unwrap();
    assert_eq!(client, c);
    assert_eq!((p.border, p.title.as_str(), p.close, p.any_key), (BorderLines::Rounded, " a ", PopupClose::OnExit, false));
    assert_eq!((p.cwd.as_str(), p.style.as_str(), p.border_style.as_str()), ("/tmp", "bg=blue", "fg=red"));
    assert_eq!(p.command, SpawnCommand::Shell { shell: "/bin/zsh".into(), command: "make test".into() });
    assert_eq!(p.env, [("FROM".to_string(), Some("global".to_string())), ("A".to_string(), Some("1".to_string()))]);
    assert_eq!((p.session, p.pane), (SessionId(0), PaneId(0)));
    // The list waits for the popup; its program's status is the client's.
    assert!(!t.m.has_finished());
    t.m.job_done(p.id, JobStatus::Exited(3), b"", NOW, &mut t.h);
    let f = t.m.take_finished().pop().unwrap();
    assert_eq!((f.stdout.as_str(), f.status, f.waited), ("after\n", 3, true));
    t.park("display-popup -EE -B -k");
    let p = &t.h.popups.last().unwrap().1;
    assert_eq!((p.border, p.close, p.any_key), (BorderLines::None, PopupClose::OnSuccess, true));
    assert_eq!(p.command, SpawnCommand::LoginShell { shell: "/bin/zsh".into() });
    // One at a time; -C closes.
    t.h.overlay_up = true;
    let n = t.h.popups.len();
    assert_eq!(t.ok("display-popup 'second'"), "");
    assert_eq!(t.h.popups.len(), n, "a second popup is not opened");
    t.ok("display-popup -C");
    assert_eq!(t.h.overlays_closed, 1);
    // Nobody to show it to.
    t.h.overlay_up = false;
    t.m.client_disconnect(c, &mut t.h);
    t.ctx = CmdContext::none(NOW);
    assert_eq!(t.err("display-popup"), "no clients");
}

#[test]
fn display_menu_items_keys_and_mouse() {
    let (mut t, c) = overlay_setup();
    assert_eq!(t.err("display-menu a"), "not enough arguments");
    assert_eq!(t.err("display-menu a b"), "not enough arguments");
    assert_eq!(t.err("display-menu a NotAKey c"), "unknown key: NotAKey");
    t.ok("display-menu -T '#[align=centre]Title' First f 'display -p first' 'Second item' s 'display -p second' '' '-Disabled' d 'display d' '#{?pane_dead,Hidden,}' h '' Last '' 'display -p last'");
    let (client, mut m) = t.h.menus.pop().unwrap();
    assert_eq!(client, c);
    // tmux's layout: the title, names and keys in columns, a separator.
    assert_eq!(m.title, "Title");
    assert_eq!(
        m.lines(),
        [Some(" First       (f) ".into()), Some(" Second item (s) ".into()), None, Some(" Disabled        ".into()), Some(" Last            ".into())]
    );
    assert_eq!((m.rect.x, m.rect.y, m.rect.cols, m.rect.rows), (30, 7, 19, 7), "centred as tmux centres it");
    assert_eq!(m.selected, Some(0), "opened by a command: the first item");
    // Down skips the separator and the disabled item, and wraps.
    assert_eq!(m.key(&key("Down")), MenuOutcome::Redraw);
    assert_eq!(m.key(&key("Down")), MenuOutcome::Redraw);
    assert_eq!(m.selected, Some(4));
    assert_eq!(m.key(&key("Down")), MenuOutcome::Redraw);
    assert_eq!(m.selected, Some(0));
    assert_eq!(m.key(&key("Up")), MenuOutcome::Redraw);
    assert_eq!(m.selected, Some(4));
    assert_eq!(m.key(&key("Enter")), MenuOutcome::Run("display -p last".into()));
    // An item's key chooses it; a disabled item's does nothing.
    assert_eq!(m.key(&key("s")), MenuOutcome::Run("display -p second".into()));
    assert_eq!(m.key(&key("d")), MenuOutcome::None);
    assert_eq!(m.key(&key("x")), MenuOutcome::None);
    assert_eq!(m.key(&key("q")), MenuOutcome::Close);
    assert_eq!(m.key(&key("Escape")), MenuOutcome::Close);
    // The mouse: over an item selects, a click chooses, a click outside closes.
    assert_eq!(m.mouse(32, 9, false), MenuOutcome::Redraw);
    assert_eq!(m.selected, Some(1));
    assert_eq!(m.mouse(32, 9, false), MenuOutcome::None);
    assert_eq!(m.mouse(32, 10, true), MenuOutcome::None, "the separator");
    assert_eq!(m.mouse(32, 8, true), MenuOutcome::Run("display -p first".into()));
    assert_eq!(m.mouse(2, 2, false), MenuOutcome::None);
    assert_eq!(m.mouse(2, 2, true), MenuOutcome::Close);
    // Positions and the starting choice.
    t.ok("display-menu -x 3 -y 10 -C 1 A a '' B b ''");
    let m = t.h.menus.pop().unwrap().1;
    assert_eq!((m.rect.x, m.rect.y, m.rect.cols, m.rect.rows, m.selected), (3, 6, 9, 4, Some(1)));
    t.ok("display-menu -x R -y S -C - Only o ''");
    let m = t.h.menus.pop().unwrap().1;
    assert_eq!((m.rect.x, m.rect.y, m.selected), (68, 20, None));
    // Separators never lead, trail or double; nothing to show opens nothing.
    t.ok("display-menu '' A a '' '' '' B b '' ''");
    assert_eq!(t.h.menus.pop().unwrap().1.lines().len(), 3);
    t.ok("display-menu '#{?pane_dead,x,}' a ''");
    assert!(t.h.menus.is_empty());
    // The default menus open.
    let list = t.m.bindings().lookup(bindings::PREFIX_TABLE, &key("<")).unwrap().commands.clone();
    t.m.execute_list(&list, &t.ctx, &mut t.h).unwrap();
    let m = t.h.menus.pop().unwrap().1;
    assert_eq!(m.title, "0:zsh");
    assert_eq!(m.lines()[0].as_deref(), Some(" Swap Left      "), "one window: nothing to swap with, so no key");
    assert!(matches!(&m.items[0], MenuItem::Entry { enabled: false, .. }));
    assert_eq!(m.lines()[3].as_deref(), Some(" Kill       (X) "));
}
