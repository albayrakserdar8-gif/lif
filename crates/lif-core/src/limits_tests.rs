//! Regression tests for limits on hostile input: arithmetic overflow in
//! formats and commands, unbounded format expansion, deep nesting, and
//! repeat counts. Each of these once panicked, hung or exhausted memory in
//! the server, which runs every session on one thread.

use crate::*;
use lif_types::Size;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const NOW: WallClock = WallClock::new(1_794_682_220, 3 * 3600);

/// Records what the model asks of the host.
#[derive(Default)]
struct Rec {
    sent: Vec<String>,
    sent_bytes: usize,
    mode_counts: Vec<u32>,
    captures: std::cell::RefCell<Vec<CaptureRange>>,
    configs: HashMap<String, ConfigFile>,
}

impl Host for Rec {
    fn spawn_pane(&mut self, _spec: &SpawnSpec) -> Result<(), String> {
        Ok(())
    }
    fn kill_pane(&mut self, _pane: PaneId) {}
    fn resize_pane(&mut self, _pane: PaneId, _size: Size) {}
    fn send_to_pane(&mut self, _pane: PaneId, input: PaneInput<'_>) -> Result<(), String> {
        let s = match input {
            PaneInput::Key(k) => format!("key:{}", KeyCode::from_event(k).map(|k| k.to_string()).unwrap_or_default()),
            PaneInput::Text(t) => format!("text:{t}"),
            PaneInput::Bytes(b) => format!("bytes:{b:?}"),
            PaneInput::Paste { data, .. } => format!("paste:{}", data.len()),
            PaneInput::Reset => "reset".into(),
        };
        self.sent_bytes += s.len();
        self.sent.push(s);
        Ok(())
    }
    fn pane_info(&self, _pane: PaneId) -> PaneInfo<'_> {
        PaneInfo::default()
    }
    fn capture_pane(&self, _pane: PaneId, range: CaptureRange) -> String {
        self.captures.borrow_mut().push(range);
        String::new()
    }
    fn attach_client(&mut self, _client: ClientId, _session: SessionId) {}
    fn detach_client(&mut self, _client: ClientId, _reason: DetachReason) {}
    fn hangup_client_parent(&mut self, _client: ClientId) {}
    fn enter_copy_mode(&mut self, _client: Option<ClientId>, _pane: PaneId, _opts: CopyModeOptions) {}
    fn mode_command(&mut self, _c: Option<ClientId>, _p: PaneId, _cmd: &str, _args: &[String], count: u32) -> Result<(), String> {
        self.mode_counts.push(count);
        Ok(())
    }
    fn read_config(&mut self, path: &str) -> Result<ConfigFile, ConfigError> {
        self.configs.get(path).cloned().ok_or_else(|| ConfigError::Missing(format!("{path}: No such file or directory")))
    }
    fn show_message(&mut self, _client: ClientId, _text: &str, _duration_ms: u64) {}
    fn prompt(&mut self, _client: ClientId, _prompt: Prompt) {}
    fn display_panes(&mut self, _client: ClientId, _duration_ms: u64, _template: &str, _keys: bool) {}
    fn bell(&mut self, _client: ClientId) {}
    fn redraw(&mut self, _what: Redraw) {}
    fn shutdown(&mut self) {}
}

/// A model with one detached 80x24 session.
struct M {
    m: Model,
    h: Rec,
    ctx: CmdContext,
}

impl M {
    fn new() -> M {
        let mut m = M { m: Model::new("box.local"), h: Rec::default(), ctx: CmdContext::none(NOW) };
        m.ok("new -d -s a -x 80 -y 24");
        m
    }

    fn run(&mut self, s: &str) -> Result<String, String> {
        let list = parse_string(s).map_err(|e| e.0)?;
        self.m.execute_list(&list, &self.ctx, &mut self.h).map(|o| o.stdout).map_err(|e| e.0)
    }

    fn ok(&mut self, s: &str) -> String {
        self.run(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn err(&mut self, s: &str) -> String {
        self.run(s).expect_err(s)
    }

    fn display(&mut self, f: &str) -> String {
        let list = CommandList(vec![parse(&["display-message".into(), "-p".into(), f.into()]).unwrap()]);
        let out = self.m.execute_list(&list, &self.ctx, &mut self.h).unwrap().stdout;
        out.strip_suffix('\n').unwrap_or(&out).to_string()
    }

    fn set_user(&mut self, name: &str, value: &str) {
        let list = CommandList(vec![parse(&["set".into(), "-g".into(), name.into(), value.into()]).unwrap()]);
        self.m.execute_list(&list, &self.ctx, &mut self.h).unwrap();
    }
}

// ----- D1: arithmetic ---------------------------------------------------

#[test]
fn format_arithmetic_saturates_instead_of_panicking() {
    let mut t = M::new();
    assert_eq!(t.display("#{e|/|:-9223372036854775808,-1}"), "9223372036854775807");
    assert_eq!(t.display("#{e|%|:-9223372036854775808,-1}"), "0");
    assert_eq!(t.display("#{e|m|:-9223372036854775808,-1}"), "0");
    assert_eq!(t.display("#{e|+|:9223372036854775807,1}"), "9223372036854775807");
    assert_eq!(t.display("#{e|-|:-9223372036854775808,1}"), "-9223372036854775808");
    assert_eq!(t.display("#{e|*|:-9223372036854775808,-1}"), "9223372036854775807");
    assert_eq!(t.display("#{e|*|:4294967296,-4294967296}"), "-9223372036854775808");
    assert_eq!(t.display("#{e|/|:7,0}#{e|%|:7,0}#{e|/|:-7,2}#{e|%|:-7,2}"), "00-3-1");
}

#[test]
fn width_modifiers_take_any_i64() {
    let mut t = M::new();
    assert_eq!(t.display("#{=-9223372036854775808:session_name}"), "a");
    assert_eq!(t.display("#{=9223372036854775807:session_name}"), "a");
    assert_eq!(t.display("#{=-1:session_name}#{=-0:session_name}|"), "a|");
    let padded = t.display("#{p-9223372036854775808:session_name}");
    // Padding goes first and is cut at the byte limit.
    assert!(padded.starts_with(' ') && padded.len() <= format::MAX_BYTES + 64, "{}", padded.len());
    assert_eq!(t.display("#{p-9223372036854775808:session_name}").len(), padded.len());
}

#[test]
fn time_modifier_takes_any_i64() {
    let mut t = M::new();
    t.set_user("@big", "9223372036854775807");
    t.set_user("@small", "-9223372036854775808");
    assert!(!t.display("#{t:@big}").is_empty());
    assert!(!t.display("#{t:@small}").is_empty());
}

#[test]
fn relative_pane_targets_take_any_i64() {
    let mut t = M::new();
    t.ok("splitw -d");
    t.ok("splitw -d");
    // Three panes, the first active: +N moves N mod 3 forward.
    t.ok("selectp -t +9223372036854775807");
    assert_eq!(t.display("#{pane_index}"), "1");
    t.ok("selectp -t -9223372036854775807");
    assert_eq!(t.display("#{pane_index}"), "0");
    // From the second pane, position + N overflows unless reduced first.
    t.ok("selectp -t +1");
    t.ok("selectp -t +9223372036854775807");
    assert_eq!(t.display("#{pane_index}"), "2");
}

#[test]
fn huge_main_pane_width_is_clamped() {
    let mut t = M::new();
    t.ok("splitw -d");
    t.ok("splitw -d");
    t.ok("setw -g main-pane-width 4294967295");
    t.ok("setw -g main-pane-height 4294967295");
    for layout in ["main-vertical", "main-horizontal", "main-vertical-mirrored", "main-horizontal-mirrored"] {
        t.ok(&format!("selectl {layout}"));
        let w = t.m.windows.values().next().unwrap();
        w.layout.check().unwrap();
        assert_eq!(w.layout.size(), Size::new(80, 24), "{layout}");
    }
}

#[test]
fn delays_are_bounded() {
    let mut t = M::new();
    t.m.client_connect(
        ClientId(1),
        ClientInfo { name: "/dev/ttys001".into(), term: "xterm".into(), size: Size::new(80, 25), cwd: "/".into(), pid: 9, terminal: true },
    );
    t.ctx = CmdContext { client: Some(ClientId(1)), ..CmdContext::none(NOW) };
    t.ok("attach -t a");
    assert_eq!(t.err("display -d 18446744073709551615 hi"), "delay too large: 18446744073709551615");
    assert_eq!(t.err("display -d 99999999999999999999999 hi"), "delay too large: 99999999999999999999999");
    assert_eq!(t.err("display -d x hi"), "delay invalid: x");
    t.ok("display -d 4294967295 hi");
    assert_eq!(t.err("displayp -d 4294967296"), "delay too large: 4294967296");
}

#[test]
fn capture_lines_are_clamped() {
    let mut t = M::new();
    t.ok("capturep -p -S -9223372036854775808 -E 9223372036854775807");
    let r = t.h.captures.borrow()[0];
    assert_eq!(r.start, LinePos::Line(-(i32::MAX as i64)));
    assert_eq!(r.end, LinePos::Line(i32::MAX as i64));
}

#[test]
fn automatic_buffer_names_survive_the_counter_wrapping() {
    let mut t = M::new();
    t.m.next_buffer = u32::MAX - 1;
    t.ok("setb -b buffer0000 named");
    for data in ["a", "b", "c"] {
        let list = CommandList(vec![parse(&["set-buffer".into(), data.into()]).unwrap()]);
        t.m.execute_list(&list, &t.ctx, &mut t.h).unwrap();
    }
    let out = t.ok("lsb -F '#{buffer_name}=#{buffer_sample}'");
    assert_eq!(out, "buffer0001=c\nbuffer4294967295=b\nbuffer4294967294=a\nbuffer0000=named\n");
}

#[test]
fn status_line_with_one_free_column_does_not_underflow() {
    use crate::status::{StatusParts, WindowEntry, render};
    let base = lif_types::Style::default();
    let windows = ["0:a", "1:bbbb", "2:c"]
        .iter()
        .enumerate()
        .map(|(i, t)| WindowEntry { text: t.to_string(), style: base, current: i == 1 })
        .collect();
    // 10 columns, the left part takes 9: one column for a list that is cut
    // on both sides of the current window.
    let p = StatusParts {
        width: 10,
        base,
        left: "123456789".into(),
        left_style: base,
        left_length: 9,
        right: String::new(),
        right_style: base,
        right_length: 0,
        windows,
        separator: " ".into(),
        justify: 0,
    };
    let line = render(&p);
    let w: usize = line.iter().map(|s| crate::status::text_width(&s.text)).sum();
    assert_eq!(w, 10);
}

// ----- D2: bounded format expansion -----------------------------------------

fn quick<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let r = f();
    assert!(start.elapsed() < Duration::from_secs(5), "{what} took {:?}", start.elapsed());
    r
}

#[test]
fn huge_padding_is_bounded() {
    let mut t = M::new();
    let out = quick("padding", || t.display("#{p99999999999:session_name}"));
    assert!(out.starts_with('a'));
    assert!(out.len() <= format::MAX_BYTES + 64, "{}", out.len());
}

#[test]
fn self_doubling_option_is_bounded() {
    let mut t = M::new();
    t.set_user("@x", "#{E:@x}#{E:@x}");
    let out = quick("#{E:@x}", || t.display("#{E:@x}"));
    assert!(out.len() <= format::MAX_BYTES + 64);
    // The same in the status line, which is expanded on every redraw.
    t.ok("set -g status-left '#{E:@x}'");
    let sid = t.m.sessions.keys().copied().next().unwrap();
    quick("status line", || t.m.status_line(sid, 80, NOW, &t.h));
}

#[test]
fn self_referencing_time_option_does_not_recurse_forever() {
    let mut t = M::new();
    t.set_user("@t", "%H#{T:@t}");
    let out = quick("#{T:@t}", || t.display("#{T:@t}"));
    assert!(out.starts_with("21"), "{out}");
}

#[test]
fn substitution_and_glob_are_bounded() {
    let mut t = M::new();
    // Within MAX_BYTES, so the value is fetched whole and the work is in
    // the substitution and the match.
    t.set_user("@a", &"a".repeat(1 << 17));
    let b = "b".repeat(1000);
    let out = quick("substitution", || t.display(&format!("#{{s/a/{b}/:@a}}")));
    assert!(out.len() <= format::MAX_BYTES + 2000, "{}", out.len());
    let pat = "*a".repeat(2000) + "b";
    let out = quick("glob", || t.display(&format!("#{{m:{pat},#{{@a}}}}")));
    assert_eq!(out, "0");
}

#[test]
fn many_expansions_are_cut_off() {
    let mut t = M::new();
    // 200,000 expansions, more than MAX_STEPS: the output stops early.
    let f = "#{session_name}".repeat(200_000);
    let out = quick("many", || t.display(&f));
    assert_eq!(out.len(), format::MAX_STEPS as usize);
}

// ----- D3: nesting -----------------------------------------------------------

#[test]
fn deeply_nested_blocks_are_a_syntax_error() {
    let s = "bind x ".to_string() + &"{ ".repeat(20_000) + &"} ".repeat(20_000);
    let e = parse_string(&s).unwrap_err();
    assert!(e.0.contains("nested"), "{e}");
    // A hundred levels still parse.
    let s = "bind x ".to_string() + &"{ bind y ".repeat(99) + "{ }" + &" }".repeat(99);
    parse_string(&s).unwrap();
}

fn layout_checksum(s: &str) -> u16 {
    let mut c: u16 = 0;
    for b in s.bytes() {
        c = (c >> 1) | ((c & 1) << 15);
        c = c.wrapping_add(b as u16);
    }
    c
}

#[test]
fn deeply_nested_layout_string_is_rejected() {
    let mut t = M::new();
    let body = "80x24,0,0{".repeat(100_000) + "80x24,0,0,1" + &"}".repeat(100_000);
    let s = format!("{:04x},{body}", layout_checksum(&body));
    assert!(t.err(&format!("selectl '{s}'")).starts_with("invalid layout"));
    let body = "4294967295x24,0,0,1";
    let s = format!("{:04x},{body}", layout_checksum(body));
    assert!(t.err(&format!("selectl '{s}'")).starts_with("invalid layout"));
}

#[test]
fn source_file_nesting_is_limited() {
    let mut t = M::new();
    t.h.configs.insert(
        "/self.toml".into(),
        ConfigFile { commands: vec![("source-file /self.toml".into(), "/self.toml:1:1".into())], diagnostics: Vec::new(), ..Default::default() },
    );
    let e = t.err("source-file /self.toml");
    assert!(e.contains("too many nested files"), "{e}");
    // Nesting below the limit still works.
    t.h.configs.insert("/leaf.toml".into(), ConfigFile { commands: vec![("set -g @leaf 1".into(), "/leaf.toml:1:1".into())], diagnostics: Vec::new(), ..Default::default() });
    t.h.configs.insert("/mid.toml".into(), ConfigFile { commands: vec![("source-file /leaf.toml".into(), "/mid.toml:1:1".into())], diagnostics: Vec::new(), ..Default::default() });
    t.ok("source-file /mid.toml");
    assert_eq!(t.display("#{@leaf}"), "1");
}

/// `[startup]` commands are kept only while the server asks for them, and
/// at most [`MAX_STARTUP_COMMANDS`] of them.
#[test]
fn startup_commands_are_bounded_and_only_kept_at_start() {
    let mut t = M::new();
    let many: Vec<(String, String)> = (0..200).map(|i| (format!("set -g @s{i} 1"), format!("/s.toml:{}:1", i + 2))).collect();
    t.h.configs.insert("/s.toml".into(), ConfigFile { startup: many, ..Default::default() });
    // An ordinary source-file keeps none.
    t.ok("source-file /s.toml");
    assert!(t.m.take_startup().is_empty());
    // At start-up they are kept, in order, up to the limit.
    t.m.collect_startup(true);
    t.ok("source-file /s.toml ; source-file /s.toml");
    t.m.collect_startup(false);
    let kept = t.m.take_startup();
    assert_eq!(kept.len(), MAX_STARTUP_COMMANDS);
    assert_eq!(kept[0], ("set -g @s0 1".to_string(), "/s.toml:2:1".to_string()));
    assert_eq!(kept[200].0, "set -g @s0 1");
    assert!(t.m.messages().any(|m| m == "/s.toml: more than 256 start-up commands, the rest are not run"));
    assert!(t.m.take_startup().is_empty());
    // They are handed over, not run by source-file.
    assert_eq!(t.display("#{@s0}"), "");
}

// ----- C6: repeat counts -------------------------------------------------------

#[test]
fn send_keys_repeat_count_is_capped_before_sending() {
    let mut t = M::new();
    assert_eq!(t.err("send -N 5000000 x"), "repeat count too large: 5000000");
    assert_eq!(t.err("send -N 4294967296 x"), "repeat count too large: 4294967296");
    assert_eq!(t.err("send -N 0 x"), "repeat count invalid: 0");
    assert_eq!(t.err("send -N x x"), "repeat count invalid: x");
    assert!(t.h.sent.is_empty());
    // Over the byte limit: nothing at all is sent, not even the reset.
    let e = t.err("send -R -N 100000 -l 'eleven byte'");
    assert!(e.starts_with("too much input: 1100000 bytes"), "{e}");
    let e = t.err("send -N 70000 Enter");
    assert!(e.starts_with("too much input"), "{e}");
    let e = t.err("send -H -N 600000 41 42");
    assert!(e.starts_with("too much input"), "{e}");
    assert!(t.h.sent.is_empty(), "{:?}", &t.h.sent[..t.h.sent.len().min(3)]);
    // No keys: nothing to repeat, nothing sent.
    t.ok("send -N 1000000");
    assert!(t.h.sent.is_empty());
    t.ok("send -N 1000 x Enter");
    assert_eq!(t.h.sent.len(), 2000);
    t.ok("send -N 1000000 -l y");
    assert_eq!(t.h.sent.len(), 1_002_000);
}

#[test]
fn copy_mode_repeat_count_is_capped() {
    let mut t = M::new();
    assert_eq!(t.err("send -X -N 1000001 cursor-down"), "repeat count too large: 1000001");
    t.ok("send -X -N 1000000 cursor-down");
    assert_eq!(t.h.mode_counts, vec![1_000_000]);
}

/// A prompt answer substituted into many slots: past 1 MiB nothing runs
/// (instead of a command of gigabytes, or a cut one doing something else).
#[test]
fn prompt_templates_are_bounded() {
    let template = "display-message '%1'".repeat(100_000);
    let answer = "a".repeat(16 * 1024);
    let t = std::time::Instant::now();
    assert_eq!(crate::expand_prompt_template(&template, &[&answer]), "");
    assert!(t.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(crate::expand_prompt_template("rename-window '%%'", &["x"]), "rename-window 'x'");
}

/// Window sizes past the limits are refused or clamped.
#[test]
fn window_sizes_are_bounded() {
    use lif_types::Size;
    assert_eq!(crate::clamp_size(Size::new(65535, 65535)).cols, crate::MAX_WINDOW_DIM);
    let s = crate::clamp_size(Size::new(65535, 65535));
    assert!(u32::from(s.cols) * u32::from(s.rows) <= crate::MAX_WINDOW_CELLS);
    assert_eq!(crate::clamp_size(Size::new(200, 50)), Size::new(200, 50));
    assert!(crate::options::parse_dim("10000x10000").is_none());
    assert_eq!(crate::options::parse_dim("200x50"), Some((200, 50)));
}
