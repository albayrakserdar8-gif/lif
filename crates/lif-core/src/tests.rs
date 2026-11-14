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
}

impl Host for TestHost {
    fn spawn_pane(&mut self, spec: &SpawnSpec) -> Result<(), String> {
        if self.fail_spawn {
            return Err("fork failed: Resource temporarily unavailable".into());
        }
        self.spawned.push(spec.clone());
        self.resized.insert(spec.pane, spec.size);
        Ok(())
    }
    fn kill_pane(&mut self, pane: PaneId) {
        self.killed.push(pane);
    }
    fn resize_pane(&mut self, pane: PaneId, size: Size) {
        self.resized.insert(pane, size);
    }
    fn send_to_pane(&mut self, pane: PaneId, input: PaneInput<'_>) {
        let s = match input {
            PaneInput::Key(k) => format!("key:{}", KeyCode::from_event(k).map(|k| k.to_string()).unwrap_or_default()),
            PaneInput::Text(t) => format!("text:{t}"),
            PaneInput::Bytes(b) => format!("bytes:{b:?}"),
            PaneInput::Paste { data, bracketed } => format!("paste:{}:{bracketed}", String::from_utf8_lossy(data)),
            PaneInput::Reset => "reset".into(),
        };
        self.sent.push((pane, s));
    }
    fn pane_info(&self, pane: PaneId) -> PaneInfo<'_> {
        PaneInfo {
            pid: 1000 + pane.0,
            title: self.titles.get(&pane).map(|s| s.as_str()).unwrap_or(""),
            current_command: self.commands.get(&pane).map(|s| s.as_str()).unwrap_or("zsh"),
            current_path: self.paths.get(&pane).map(|s| s.as_str()).unwrap_or(""),
            history_size: 0,
            history_bytes: 0,
            in_mode: false,
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
    fn display_panes(&mut self, client: ClientId, _duration_ms: u64, template: &str) {
        self.messages.push((client, format!("display-panes {template}")));
    }
    fn bell(&mut self, client: ClientId) {
        self.bells.push(client);
    }
    fn redraw(&mut self, _what: Redraw) {}
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
        }
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

#[test]
fn random_commands_keep_the_model_consistent() {
    let cmds = [
        "splitw", "splitw -h", "splitw -b -l 3", "splitw -d -h -p 70", "killp", "killp -t :.+", "neww", "neww -d", "neww -a",
        "kill-window", "selectp -L", "selectp -R", "selectp -U", "selectp -D", "lastp", "last", "next", "prev",
        "swapp -U", "swapp -D", "rotatew", "rotatew -D", "resizep -Z", "resizep -L 3", "resizep -D 2", "resizep -x 20%",
        "selectl tiled", "selectl main-vertical", "selectl -E", "nextl", "breakp", "breakp -d", "movew -r",
        "swapw -t :+", "new -d", "switchc -n", "selectw -t :^", "selectw -l", "respawnp -k", "selectl -o",
        "set -g renumber-windows on", "set -g renumber-windows off", "neww -k -t :0", "movew -t :9",
    ];
    for seed in 1..40u64 {
        let mut t = T::with_session();
        let c = t.attach_client(1, 100, 40);
        t.ctx = CmdContext::client(c, NOW);
        t.ok("attach");
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        for _ in 0..200 {
            if t.m.sessions().count() == 0 {
                break;
            }
            if rng.next().is_multiple_of(10) {
                let panes: Vec<PaneId> = t.m.panes().map(|p| p.id()).collect();
                let p = panes[(rng.next() % panes.len() as u64) as usize];
                t.m.pane_exited(p, Some(0), &mut t.h);
                check_model(&t.m);
                continue;
            }
            let cmd = rng.pick(&cmds);
            let _ = t.run(cmd);
            if t.m.client(c).and_then(|c| c.session()).is_none() && t.m.sessions().count() > 0 {
                t.ok("attach");
            }
        }
    }
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
