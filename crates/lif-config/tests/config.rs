//! lif-config end to end: TOML text -> command strings -> a real
//! lif_core::Model.

use lif_config::{Config, Diagnostic, Entry, Severity, example, migrate_tmux_conf, resolve_path};
use lif_core::bindings::{PREFIX_TABLE, ROOT_TABLE};
use lif_core::{
    CaptureRange, ClientId, CmdContext, CopyModeOptions, DetachReason, Host, KeyCode, Model, PaneId, PaneInfo, PaneInput,
    Prompt, Redraw, SessionId, SpawnSpec, WallClock, parse_string,
};
use lif_types::Size;
use std::ffi::OsStr;
use std::path::PathBuf;

// ---------------------------------------------------------------- helpers --

/// A host that accepts everything (the commands of a config file never
/// reach it).
#[derive(Default)]
struct TestHost {
    calls: usize,
}

impl Host for TestHost {
    fn spawn_pane(&mut self, _spec: &SpawnSpec) -> Result<(), String> {
        self.calls += 1;
        Ok(())
    }
    fn kill_pane(&mut self, _pane: PaneId) {
        self.calls += 1;
    }
    fn resize_pane(&mut self, _pane: PaneId, _size: Size) {}
    fn send_to_pane(&mut self, _pane: PaneId, _input: PaneInput<'_>) -> Result<(), String> {
        self.calls += 1;
        Ok(())
    }
    fn pane_info(&self, _pane: PaneId) -> PaneInfo<'_> {
        PaneInfo::default()
    }
    fn capture_pane(&self, _pane: PaneId, _range: CaptureRange) -> String {
        String::new()
    }
    fn attach_client(&mut self, _client: ClientId, _session: SessionId) {}
    fn detach_client(&mut self, _client: ClientId, _reason: DetachReason) {}
    fn enter_copy_mode(&mut self, _client: Option<ClientId>, _pane: PaneId, _opts: CopyModeOptions) {}
    fn show_message(&mut self, _client: ClientId, _text: &str, _duration_ms: u64) {}
    fn prompt(&mut self, _client: ClientId, _prompt: Prompt) {}
    fn display_panes(&mut self, _client: ClientId, _duration_ms: u64, _template: &str, _keys: bool) {}
    fn hangup_client_parent(&mut self, _client: ClientId) {}
    fn bell(&mut self, _client: ClientId) {}
    fn redraw(&mut self, _what: Redraw) {}
    fn shutdown(&mut self) {}
}

/// Commands of a file that must load without diagnostics.
fn cmds(toml: &str) -> Vec<String> {
    let c = Config::parse(toml);
    assert!(c.diagnostics.is_empty(), "unexpected diagnostics for\n{toml}\n{}", c.report());
    c.command_strings()
}

/// Diagnostics rendered as "line:col: severity: message".
fn diags(toml: &str) -> Vec<String> {
    Config::parse(toml).diagnostics.iter().map(|d| d.to_string()).collect()
}

/// The one diagnostic of a file.
fn diag(toml: &str) -> String {
    let d = diags(toml);
    assert_eq!(d.len(), 1, "expected one diagnostic for\n{toml}\ngot {d:#?}");
    d[0].clone()
}

fn model_with(toml: &str) -> Model {
    let mut m = Model::new("host");
    let c = Config::parse(toml);
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    let mut host = TestHost::default();
    let errs = c.apply(&mut m, &mut host);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(host.calls, 0, "config commands never touch panes");
    m
}

fn opt(m: &Model, name: &str) -> String {
    m.option_text(name, None, None).unwrap_or_else(|| panic!("no option {name}"))
}

fn key(name: &str) -> KeyCode {
    KeyCode::parse(name).unwrap()
}

/// (repeat, note, commands) of a binding.
fn bound(m: &Model, table: &str, k: &str) -> Option<(bool, Option<String>, String)> {
    m.bindings().lookup_code(table, &key(k)).map(|b| (b.repeat, b.note.clone(), b.commands.to_string()))
}

// ------------------------------------------------------- TOML conformance --

#[test]
fn toml_forms_mean_the_same() {
    let expected = vec![
        "set-option -g base-index 1".to_string(),
        "set-option -g status-left \"[#S] \"".to_string(),
        "set-option -g status-position top".to_string(),
        "bind-key -T root M-h \"select-pane -L\"".to_string(),
    ];
    let forms = [
        // Header tables.
        "base-index = 1\n[status]\nleft = \"[#S] \"\nposition = \"top\"\n[keys.root]\nM-h = \"select-pane -L\"\n",
        // Dotted keys at the top level.
        "base-index = 1\nstatus.left = \"[#S] \"\nstatus.position = \"top\"\nkeys.root.M-h = \"select-pane -L\"\n",
        // Inline tables, literal strings, quoted keys, comments everywhere.
        "# comment\nbase-index = 1 # trailing\nstatus = { left = '[#S] ', position = 'top' } # c\n\n[keys]\n# c\nroot = { \"M-h\" = 'select-pane -L' }\n",
        // Dotted keys inside a header, hex integer, CRLF line endings.
        "base-index = 0x1\r\n[keys]\r\nroot.'M-h' = \"select-pane -L\"\r\n[status]\r\nleft = \"[#S] \"\r\nposition = \"top\"\r\n",
    ];
    for f in forms {
        assert_eq!(cmds(f), expected, "{f}");
    }
}

#[test]
fn strings_numbers_and_arrays() {
    // Basic string escapes.
    assert_eq!(cmds(r#"status.right = "a\tb\\c\"dé\U0001F600""#), ["set-option -g status-right \"a\\tb\\\\c\\\"dé😀\""]);
    // Multi-line basic string: the leading newline is dropped, a trailing
    // backslash joins lines.
    assert_eq!(cmds("status.left = \"\"\"\n#S \\\n   :\"\"\""), ["set-option -g status-left \"#S :\""]);
    // Multi-line literal string keeps backslashes and newlines.
    assert_eq!(cmds("status.left = '''\nx\\y\nz'''"), ["set-option -g status-left x\\\\y\\nz"]);
    // Integers: underscores, octal, binary, plus sign.
    assert_eq!(cmds("history-limit = 1_000_000"), ["set-option -g history-limit 1000000"]);
    assert_eq!(cmds("history-limit = 0o17"), ["set-option -g history-limit 15"]);
    assert_eq!(cmds("history-limit = 0b101"), ["set-option -g history-limit 5"]);
    assert_eq!(cmds("history-limit = +7"), ["set-option -g history-limit 7"]);
    // Arrays across lines with comments and a trailing comma.
    assert_eq!(
        cmds("[keys]\nr = [\n  \"select-layout tiled\", # first\n  \"display-message hi\",\n]\n"),
        ["bind-key -T prefix r \"select-layout tiled ; display-message hi\""]
    );
}

#[test]
fn syntax_errors_have_positions_and_do_not_lose_the_rest() {
    let c = Config::parse("mouse = false\nbase-index = \"1\nrepeat-time = 100\n");
    assert_eq!(c.diagnostics.len(), 1, "{}", c.report());
    let d = &c.diagnostics[0];
    assert_eq!((d.severity, d.line, d.column), (Severity::Error, 2, 16));
    assert!(d.message.starts_with("invalid TOML: invalid basic string"), "{}", d.message);
    assert_eq!(c.command_strings(), ["set-option -g mouse off", "set-option -g repeat-time 100"]);

    for (src, line, col, what) in [
        ("mouse = tru\n", 1, 9, "invalid TOML"),
        ("\n\nmouse true\n", 3, 7, "invalid TOML"),
        ("mouse = true\nmouse = false\n", 2, 1, "duplicate key"),
        ("[keys\nx = false\n", 1, 6, "invalid TOML"),
        ("base-index = 1 2\n", 1, 14, "invalid TOML"),
    ] {
        let c = Config::parse(src);
        let d = c.diagnostics.first().unwrap_or_else(|| panic!("no diagnostic for {src:?}"));
        assert_eq!((d.line, d.column), (line, col), "{src:?}: {d}");
        assert!(d.message.contains(what), "{src:?}: {d}");
    }
}

#[test]
fn unsupported_toml_features_are_errors_not_guesses() {
    assert_eq!(diag("display-time = 1979-05-27"), "1:16: error: `display-time`: expected an integer, found datetime");
    assert_eq!(diag("display-time = 7.5"), "1:16: error: `display-time`: expected an integer, found float");
    assert_eq!(diag("display-time = inf"), "1:16: error: `display-time`: expected an integer, found float");
    assert_eq!(diag("[[keys]]\nx = false"), "1:3: error: `keys` must be a table of key bindings ([keys]), found array");
    assert_eq!(
        diag("[[theme.status]]\nfg = \"red\""),
        "1:1: error: `theme.status`: expected a style table such as { fg = \"black\", bg = \"green\" } or a tmux style string, found array"
    );
    assert_eq!(diag("history-limit = 99999999999999999999"), "1:17: error: `history-limit`: integer out of range");
}

// ------------------------------------------------- options -> commands --

#[test]
fn every_option_form_gives_the_exact_command() {
    let cases: &[(&str, &[&str])] = &[
        ("mouse = false", &["set-option -g mouse off"]),
        ("automatic-rename = true", &["set-option -g automatic-rename on"]),
        ("renumber-windows = true", &["set-option -g renumber-windows on"]),
        ("base-index = 1", &["set-option -g base-index 1"]),
        ("pane-base-index = 1", &["set-option -g pane-base-index 1"]),
        ("repeat-time = 600", &["set-option -g repeat-time 600"]),
        ("display-time = 2000", &["set-option -g display-time 2000"]),
        ("history-limit = 50000", &["set-option -g history-limit 50000"]),
        ("default-shell = \"/bin/zsh\"", &["set-option -g default-shell /bin/zsh"]),
        ("default-command = \"exec fish -l\"", &["set-option -g default-command \"exec fish -l\""]),
        ("default-command = \"\"", &["set-option -g default-command ''"]),
        ("scrollback = 1048576", &["set-option -g scrollback-bytes 1048576"]),
        ("scrollback = \"32MB\"", &["set-option -g scrollback-bytes 33554432"]),
        ("scrollback = \"512 KiB\"", &["set-option -g scrollback-bytes 524288"]),
        ("scrollback = \"1G\"", &["set-option -g scrollback-bytes 1073741824"]),
        ("mode-keys = \"vi\"", &["set-option -g mode-keys vi"]),
        ("set-clipboard = \"on\"", &["set-option -s set-clipboard on"]),
        ("set-clipboard = false", &["set-option -s set-clipboard off"]),
        ("escape-time = 0", &["set-option -s escape-time 0"]),
        ("exit-empty = false", &["set-option -s exit-empty off"]),
        ("buffer-limit = 10", &["set-option -s buffer-limit 10"]),
        ("visual-bell = true", &["set-option -g visual-bell on"]),
        ("visual-bell = \"both\"", &["set-option -g visual-bell both"]),
        ("main-pane-width = \"60%\"", &["set-option -g main-pane-width \"60%\""]),
        ("main-pane-height = 30", &["set-option -g main-pane-height 30"]),
        ("default-size = \"120x40\"", &["set-option -g default-size 120x40"]),
        ("prefix2 = \"C-s\"", &["set-option -g prefix2 C-s"]),
        ("prefix2 = \"none\"", &["set-option -g prefix2 None"]),
        ("status = false", &["set-option -g status off"]),
        ("[status]\nenabled = true", &["set-option -g status on"]),
        ("[status]\nposition = \"top\"", &["set-option -g status-position top"]),
        ("[status]\njustify = \"centre\"", &["set-option -g status-justify centre"]),
        ("[status]\ninterval = 5", &["set-option -g status-interval 5"]),
        ("[status]\nleft = \"#S \"\nleft-length = 20", &["set-option -g status-left \"#S \"", "set-option -g status-left-length 20"]),
        ("[status]\nright = \"%H:%M\"\nright-length = 8", &["set-option -g status-right \"%H:%M\"", "set-option -g status-right-length 8"]),
        ("[status]\nwindow-format = \"#I #W\"", &["set-option -g window-status-format \"#I #W\""]),
        ("[status]\ncurrent-window-format = \"*#I\"", &["set-option -g window-status-current-format \"*#I\""]),
        ("[status]\nseparator = \"|\"", &["set-option -g window-status-separator |"]),
    ];
    for (src, want) in cases {
        assert_eq!(cmds(src), *want, "{src}");
    }
}

#[test]
fn prefix_alone_does_the_whole_job() {
    assert_eq!(
        cmds("prefix = \"C-a\""),
        ["set-option -g prefix C-a", "unbind-key -T prefix C-b", "bind-key -T prefix C-a send-prefix"]
    );
    // Normalized key names; C-b itself needs no unbinding.
    assert_eq!(cmds("prefix = \"^b\""), ["set-option -g prefix C-b", "bind-key -T prefix C-b send-prefix"]);
    assert_eq!(
        cmds("prefix = \"M-Space\""),
        ["set-option -g prefix M-Space", "unbind-key -T prefix C-b", "bind-key -T prefix M-Space send-prefix"]
    );
    assert_eq!(cmds("prefix = false"), ["set-option -g prefix None"]);
    // Bindings come after the prefix recipe, so an explicit binding of the
    // new prefix (or of C-b) wins, wherever it is written.
    assert_eq!(
        cmds("keys.C-b = \"last-window\"\nprefix = \"C-a\""),
        [
            "set-option -g prefix C-a",
            "unbind-key -T prefix C-b",
            "bind-key -T prefix C-a send-prefix",
            "bind-key -T prefix C-b last-window",
        ]
    );

    let mut m = model_with("prefix = \"C-a\"");
    assert_eq!(opt(&m, "prefix"), "C-a");
    assert_eq!(bound(&m, PREFIX_TABLE, "C-a"), Some((false, None, "send-prefix".into())));
    assert_eq!(bound(&m, PREFIX_TABLE, "C-b"), None);
    // The prefix state machine uses it.
    let c = ClientId(1);
    m.client_connect(
        c,
        lif_core::ClientInfo {
            name: "/dev/ttys001".into(),
            term: "xterm".into(),
            size: Size::new(80, 24),
            cwd: "/".into(),
            pid: 1,
            terminal: true,
        },
    );
    let mut h = TestHost::default();
    assert_eq!(m.handle_key(c, &key("C-a").to_event(), 0, &mut h), lif_core::KeyDecision::Swallow);
    match m.handle_key(c, &key("%").to_event(), 0, &mut h) {
        lif_core::KeyDecision::Run(list) => assert_eq!(list.to_string(), "split-window -h"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn theme_styles() {
    let cases: &[(&str, &str)] = &[
        ("status = { fg = \"black\", bg = \"#a6e3a1\", bold = true }", "set-option -g status-style \"fg=black,bg=#a6e3a1,bold\""),
        ("status = { bold = true, bg = \"colour235\", fg = \"brightwhite\" }", "set-option -g status-style bold,bg=colour235,fg=brightwhite"),
        ("status-left = { italic = true, dim = false }", "set-option -g status-left-style italics,nodim"),
        ("status-right = { underline = \"curly\", underline-color = \"red\" }", "set-option -g status-right-style curly-underscore,us=red"),
        ("window = {}", "set-option -g window-status-style default"),
        (
            "current-window = { reverse = true, blink = true, hidden = false, strikethrough = true, overline = true }",
            "set-option -g window-status-current-style reverse,blink,nohidden,strikethrough,overline",
        ),
        ("last-window = \"fg=red,bold\"", "set-option -g window-status-last-style fg=red,bold"),
        ("activity = { underline = true }", "set-option -g window-status-activity-style underscore"),
        ("bell = { underline = false }", "set-option -g window-status-bell-style nounderscore"),
    ];
    for (line, want) in cases {
        assert_eq!(cmds(&format!("[theme]\n{line}")), [*want], "{line}");
    }
    let m = model_with("[theme]\nstatus = { fg = \"black\", bg = \"#a6e3a1\" }\ncurrent-window = { bold = true }");
    assert_eq!(opt(&m, "status-style"), "fg=black,bg=#a6e3a1");
    assert_eq!(opt(&m, "window-status-current-style"), "bold");
}

#[test]
fn pane_border_selection_message_and_match_styles() {
    let c = Config::parse(
        "[theme]\nborder = { fg = \"red\" }\nactive-border = \"fg=green\"\nselection = {}\nmessage = { bg = \"yellow\" }\nmatch = { bg = \"blue\" }\ncurrent-match = \"bg=red\"",
    );
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    assert_eq!(
        c.command_strings(),
        [
            "set-option -g pane-border-style fg=red",
            "set-option -g pane-active-border-style fg=green",
            "set-option -g mode-style default",
            "set-option -g message-style bg=yellow",
            "set-option -g copy-mode-match-style bg=blue",
            "set-option -g copy-mode-current-match-style bg=red",
        ]
    );
    // Their values are checked.
    assert_eq!(diag("[theme]\nborder = { fg = \"rd\" }"), "2:17: error: `theme.border`: unknown color \"rd\", did you mean \"red\"?");
}

// ------------------------------------------------- bindings -> commands --

#[test]
fn every_binding_form_gives_the_exact_command() {
    let cases: &[(&str, &[&str])] = &[
        ("[keys]\n\"|\" = \"split-window -h\"", &["bind-key -T prefix | \"split-window -h\""]),
        ("[keys]\nc = \"new-window -c '#{pane_current_path}'\"", &["bind-key -T prefix c \"new-window -c '#{pane_current_path}'\""]),
        ("[keys]\nr = [\"select-layout tiled\", \"display-message done\"]", &["bind-key -T prefix r \"select-layout tiled ; display-message done\""]),
        ("[keys]\nh = { command = \"resize-pane -L 5\", repeat = true }", &["bind-key -r -T prefix h \"resize-pane -L 5\""]),
        ("[keys]\nh = { command = \"select-pane -L\", note = \"Go left\" }", &["bind-key -N \"Go left\" -T prefix h \"select-pane -L\""]),
        (
            "[keys]\nh = { command = [\"select-pane -L\", \"resize-pane -Z\"], repeat = false }",
            &["bind-key -T prefix h \"select-pane -L ; resize-pane -Z\""],
        ),
        ("[keys]\nx = false", &["unbind-key -T prefix x"]),
        ("[keys]\nx = \"none\"", &["unbind-key -T prefix x"]),
        ("[keys]\nx = \"None\"", &["unbind-key -T prefix x"]),
        ("[keys.prefix]\nx = \"kill-pane\"", &["bind-key -T prefix x kill-pane"]),
        ("[keys.root]\nM-Left = \"select-pane -L\"", &["bind-key -T root M-Left \"select-pane -L\""]),
        ("[keys.root]\n\"C-M-x\" = false", &["unbind-key -T root C-M-x"]),
        ("[keys.copy-mode]\n\"C-w\" = \"copy-mode -q\"", &["bind-key -T copy-mode C-w \"copy-mode -q\""]),
        ("[keys.copy-mode-vi]\nq = \"copy-mode -q\"", &["bind-key -T copy-mode-vi q \"copy-mode -q\""]),
        // Key names are normalized and escaped for the command language.
        ("[keys]\n\"C-A\" = \"last-window\"", &["bind-key -T prefix C-a last-window"]),
        ("[keys]\nDelete = \"kill-pane\"", &["bind-key -T prefix DC kill-pane"]),
        ("[keys]\n\";\" = \"last-pane\"", &["bind-key -T prefix \\; last-pane"]),
        ("[keys]\n'\"' = \"split-window\"", &["bind-key -T prefix \\\" split-window"]),
        ("[keys]\n\"#\" = \"list-buffers\"", &["bind-key -T prefix \\# list-buffers"]),
        ("[keys]\n\"~\" = \"list-keys\"", &["bind-key -T prefix \\~ list-keys"]),
        ("[keys]\n\"'\" = \"list-keys\"", &["bind-key -T prefix \\' list-keys"]),
        ("[keys]\n\"\\\\\" = \"list-keys\"", &["bind-key -T prefix \\\\ list-keys"]),
        ("[keys]\nSpace = \"next-layout\"", &["bind-key -T prefix Space next-layout"]),
        ("[keys]\n\" \" = \"next-layout\"", &["bind-key -T prefix Space next-layout"]),
        // A command string with its own quoting and block syntax.
        (
            "[keys]\n\"$\" = 'command-prompt -I \"#S\" { rename-session \"%%\" }'",
            &["bind-key -T prefix \\$ \"command-prompt -I \\\"#S\\\" { rename-session \\\"%%\\\" }\""],
        ),
    ];
    for (src, want) in cases {
        assert_eq!(cmds(src), *want, "{src}");
    }
}

#[test]
fn bindings_reach_the_model() {
    let m = model_with(
        r##"
[keys]
"|" = "split-window -h -c '#{pane_current_path}'"
h = { command = "resize-pane -L 5", repeat = true, note = "shrink" }
r = ["select-layout tiled", "display-message 'tiled!'"]
'"' = false
";" = "last-window"
"$" = 'command-prompt -I "#S" { rename-session "%%" }'
[keys.root]
"M-Left" = "select-pane -L"
[keys.copy-mode-vi]
q = "copy-mode -q"
"##,
    );
    assert_eq!(bound(&m, PREFIX_TABLE, "|"), Some((false, None, "split-window -h -c \"#{pane_current_path}\"".into())));
    assert_eq!(bound(&m, PREFIX_TABLE, "h"), Some((true, Some("shrink".into()), "resize-pane -L 5".into())));
    assert_eq!(bound(&m, PREFIX_TABLE, "r"), Some((false, None, "select-layout tiled ; display-message tiled!".into())));
    assert_eq!(bound(&m, PREFIX_TABLE, "\""), None);
    assert_eq!(bound(&m, PREFIX_TABLE, ";"), Some((false, None, "last-window".into())));
    assert_eq!(bound(&m, PREFIX_TABLE, "$"), Some((false, None, "command-prompt -I \"#S\" { rename-session \"%%\" }".into())));
    assert_eq!(bound(&m, ROOT_TABLE, "M-Left"), Some((false, None, "select-pane -L".into())));
    assert_eq!(bound(&m, "copy-mode-vi", "q"), Some((false, None, "copy-mode -q".into())));
    // Untouched defaults stay.
    assert_eq!(bound(&m, PREFIX_TABLE, "c"), Some((false, Some("Create a new window".into()), "new-window".into())));
}

#[test]
fn options_reach_the_model() {
    let m = model_with(
        r##"
prefix = "C-a"
base-index = 1
pane-base-index = 1
default-shell = "/bin/zsh"
default-command = "exec zsh -l"
scrollback = "32MB"
history-limit = 50000
automatic-rename = false
renumber-windows = true
repeat-time = 300
mouse = false
set-clipboard = "on"
display-time = 1500
mode-keys = "vi"
[status]
enabled = true
position = "top"
left = "#S "
right = "%H:%M"
"##,
    );
    for (name, want) in [
        ("prefix", "C-a"),
        ("base-index", "1"),
        ("pane-base-index", "1"),
        ("default-shell", "/bin/zsh"),
        ("default-command", "exec zsh -l"),
        ("scrollback-bytes", "33554432"),
        ("history-limit", "50000"),
        ("automatic-rename", "off"),
        ("renumber-windows", "on"),
        ("repeat-time", "300"),
        ("mouse", "off"),
        ("set-clipboard", "on"),
        ("display-time", "1500"),
        ("mode-keys", "vi"),
        ("status", "on"),
        ("status-position", "top"),
        ("status-left", "#S "),
        ("status-right", "%H:%M"),
    ] {
        assert_eq!(opt(&m, name), want, "{name}");
    }
}

#[test]
fn every_command_string_is_parseable_by_lif_core() {
    let src = format!(
        "{}\n[keys.copy-mode]\nF1 = \"display-message 'it''s'\"",
        example().replace("prefix = \"C-b\"", "prefix = \"C-a\"").replace("prefix2 = false", "prefix2 = \"C-s\"")
    );
    let c = Config::parse(&src);
    assert!(!c.has_errors(), "{}", c.report());
    for e in &c.commands {
        parse_string(&e.command).unwrap_or_else(|err| panic!("{}: {err}", e.command));
    }
}

// --------------------------------------------------------- diagnostics --

#[test]
fn unknown_keys_get_suggestions() {
    assert_eq!(diag("mouze = true"), "1:1: error: unknown option `mouze`, did you mean `mouse`?");
    assert_eq!(diag("\n  base-idnex = 1"), "2:3: error: unknown option `base-idnex`, did you mean `base-index`?");
    assert_eq!(diag("frobnicate = 1"), "1:1: error: unknown option `frobnicate`");
    assert_eq!(diag("[theem]\nx = 1"), "1:2: error: unknown section [theem], did you mean [theme]?");
    assert_eq!(diag("[status]\nposiiton = \"top\""), "2:1: error: unknown key `posiiton` in [status], did you mean `position`?");
    assert_eq!(
        diag("[status]\nfoo = 1"),
        "2:1: error: unknown key `foo` in [status] (expected one of: enabled, position, justify, interval, left, left-length, right, right-length, window-format, current-window-format, separator)"
    );
    assert_eq!(diag("[theme]\nborders = {}"), "2:1: error: unknown key `borders` in [theme], did you mean `border`?");
    assert_eq!(
        diag("[theme]\nstatus = { fg = \"red\", italics = true }"),
        "2:24: error: `theme.status`: unknown style key `italics`, did you mean `italic`? (style keys: fg, bg, underline-color, bold, dim, italic, underline, blink, reverse, hidden, strikethrough, overline)"
    );
    assert_eq!(
        diag("[keys.copy-mode-vy]\nq = \"copy-mode -q\""),
        "1:7: error: unknown key table [keys.copy-mode-vy], did you mean [keys.copy-mode-vi]? (tables are: prefix, root, copy-mode, copy-mode-vi)"
    );
    assert_eq!(
        diag("[keys]\nh = { comand = \"select-pane -L\" }"),
        "2:7: error: key `h` in [keys]: unknown binding field `comand`, did you mean `command`? (fields: command, repeat, note)"
    );
    // tmux option names point to where they live here.
    assert_eq!(diag("status-left = \"x\""), "1:1: error: `status-left` is set as `left` in the [status] section");
    assert_eq!(diag("status-style = \"x\""), "1:1: error: `status-style` is set as `status` in the [theme] section");
    assert_eq!(diag("scrollback-bytes = 1"), "1:1: error: `scrollback-bytes` is set as `scrollback` (bytes, or a size such as \"32MB\")");
    assert_eq!(diag("[status]\nstatus-position = \"top\""), "2:1: error: `status-position` is called `position` in [status]");
}

#[test]
fn type_errors_name_the_expected_type() {
    for (src, want) in [
        ("mouse = \"on\"", "1:9: error: `mouse`: expected true or false, found string \"on\" (write `mouse = true`)"),
        ("mouse = 1", "1:9: error: `mouse`: expected true or false, found integer 1"),
        ("base-index = \"1\"", "1:14: error: `base-index`: expected an integer, found string \"1\""),
        ("default-shell = 3", "1:17: error: `default-shell`: expected a string, found integer 3"),
        ("mode-keys = \"vim\"", "1:13: error: `mode-keys`: unknown value \"vim\", did you mean \"vi\"? (expected one of: emacs, vi)"),
        ("mode-keys = true", "1:13: error: `mode-keys`: expected one of: emacs, vi, found boolean true"),
        ("set-clipboard = 1", "1:17: error: `set-clipboard`: expected true, false or one of: off, external, on, found integer 1"),
        (
            "scrollback = \"32 parsecs\"",
            "1:14: error: `scrollback`: expected a size such as \"32MB\" (units B, KB, MB, GB; 1 KB = 1024 bytes), found string \"32 parsecs\"",
        ),
        ("scrollback = -5", "1:14: error: `scrollback`: expected a size in bytes, found integer -5"),
        ("scrollback = true", "1:14: error: `scrollback`: expected a size in bytes or a string such as \"32MB\", found boolean true"),
        ("prefix = 1", "1:10: error: `prefix`: expected a key name such as \"C-a\", or false for none, found integer 1"),
        ("prefix = true", "1:10: error: `prefix`: expected a key name such as \"C-a\", or false for none, found boolean true"),
        ("default-size = 80", "1:16: error: `default-size`: expected a size such as \"80x24\", found integer 80"),
        (
            "main-pane-width = true",
            "1:19: error: `main-pane-width`: expected a number of cells or a percentage such as \"50%\", found boolean true",
        ),
        ("status = \"top\"", "1:1: error: `status` must be true, false or a [status] table, found string \"top\""),
        ("theme = \"dark\"", "1:1: error: `theme` must be a table of styles ([theme]), found string \"dark\""),
        ("keys = 1", "1:1: error: `keys` must be a table of key bindings ([keys]), found integer 1"),
        ("[keys]\nroot = 1", "2:1: error: `keys.root` must be a table of key bindings, found integer 1"),
        ("[keys]\nh = { command = \"kill-pane\", repeat = \"yes\" }", "2:39: error: key `h` in [keys]: `repeat` expects true or false, found string \"yes\""),
        ("[keys]\nh = { command = \"kill-pane\", note = 1 }", "2:37: error: key `h` in [keys]: `note` expects a string, found integer 1"),
        (
            "[keys]\nh = { repeat = true }",
            "2:5: error: key `h` in [keys]: a binding table needs `command`, e.g. { command = \"resize-pane -L 5\", repeat = true }",
        ),
        (
            "[keys]\nh = 5",
            "2:5: error: key `h` in [keys]: expected a command string, a list of commands, false to unbind, or { command = ..., repeat = true }, found integer 5",
        ),
        (
            "[keys]\nh = true",
            "2:5: error: key `h` in [keys]: expected a command string, a list of commands, false to unbind, or { command = ..., repeat = true }, found boolean true",
        ),
        ("[keys]\nh = []", "2:5: error: key `h` in [keys]: empty list of commands"),
        ("[keys]\nh = [\"kill-pane\", 3]", "2:19: error: key `h` in [keys]: expected a command string, found integer 3"),
        (
            "[theme]\nstatus = 3",
            "2:10: error: `theme.status`: expected a style table such as { fg = \"black\", bg = \"green\" } or a tmux style string, found integer 3",
        ),
        (
            "[theme]\nstatus = { fg = 3 }",
            "2:17: error: `theme.status`: `fg` expects a color: a name (black, red, green, yellow, blue, magenta, cyan, white, or bright...), colour0 to colour255, #rrggbb, or default, found integer 3",
        ),
        ("[theme]\nstatus = { bold = \"yes\" }", "2:19: error: `theme.status`: `bold` expects true or false, found string \"yes\""),
        (
            "[theme]\nstatus = { underline = \"wavy\" }",
            "2:24: error: `theme.status`: `underline` expects true, false, \"single\", \"double\", \"curly\", \"dotted\" or \"dashed\", found string \"wavy\"",
        ),
        ("[theme]\nstatus = \"fg=red,blod\"", "2:10: error: `theme.status`: unknown style attribute `blod`, did you mean `bold`?"),
    ] {
        assert_eq!(diag(src), want, "{src}");
    }
}

#[test]
fn values_lif_core_rejects_are_reported_at_the_value() {
    assert_eq!(diag("base-index = -1"), "1:14: error: `base-index`: value is too small: -1");
    assert_eq!(diag("repeat-time = 99999"), "1:15: error: `repeat-time`: value is too large: 99999");
    assert_eq!(diag("main-pane-width = \"wide\""), "1:19: error: `main-pane-width`: invalid value: wide");
    assert_eq!(diag("default-size = \"80by24\""), "1:16: error: `default-size`: invalid value: 80by24");
    // A bad setting does not stop the others.
    let c = Config::parse("base-index = -1\nmouse = false\n");
    assert_eq!(c.command_strings(), ["set-option -g mouse off"]);
}

#[test]
fn bad_key_names() {
    assert_eq!(diag("[keys]\n\"C-Lef\" = \"kill-pane\""), "2:1: error: unknown key name `C-Lef`, did you mean `C-Left`?");
    assert_eq!(diag("[keys.root]\nEntr = \"kill-pane\""), "2:1: error: unknown key name `Entr`, did you mean `Enter`? in [keys.root]");
    assert_eq!(
        diag("[keys]\n\"Ctrl-a\" = \"kill-pane\""),
        "2:1: error: unknown key name `Ctrl-a`: write modifiers as C- (Ctrl), M- (Alt/Meta) and S- (Shift), e.g. `C-a`"
    );
    assert_eq!(diag("[keys]\n\"\" = \"kill-pane\""), "2:1: error: empty key name");
    assert_eq!(
        diag("[keys]\nBogus = \"kill-pane\""),
        "2:1: error: unknown key name `Bogus` (expected tmux key names such as `C-a`, `M-Left`, `F5`, `Enter` or a single character)"
    );
    assert_eq!(
        diag("prefix = \"C-\""),
        "1:10: error: `prefix`: unknown key name `C-` (expected tmux key names such as `C-a`, `M-Left`, `F5`, `Enter` or a single character)"
    );
    // The same key twice in a table (TOML sees two different keys).
    assert_eq!(
        diag("[keys]\n\"C-a\" = \"kill-pane\"\n\"c-A\" = \"kill-window\""),
        "3:1: error: key `c-A` is bound twice in [keys] (it is the same key as the one on line 2)"
    );
    // ...but the same key in two tables is fine.
    assert_eq!(cmds("[keys]\nx = \"kill-pane\"\n[keys.root]\nx = \"kill-pane\"").len(), 2);
}

#[test]
fn bad_command_strings_point_at_the_value() {
    assert_eq!(
        diag("[keys]\n\"|\" = \"splt-window -h\""),
        "2:7: error: key `|` in [keys]: unknown command `splt-window`, did you mean `split-window`?"
    );
    assert_eq!(diag("[keys]\nx =   \"kill-pane -Q\""), "2:7: error: key `x` in [keys]: command kill-pane: unknown flag -Q");
    assert_eq!(diag("[keys]\nx = \"customize-mode\""), "2:5: error: key `x` in [keys]: unsupported command: customize-mode");
    assert_eq!(diag("[keys]\nx = \"split-window ; { \""), "2:5: error: key `x` in [keys]: syntax error: missing }");
    assert_eq!(diag("[keys]\nx = \"  # only a comment\""), "2:5: error: key `x` in [keys]: empty command");
    assert_eq!(
        diag("[keys]\nx = [\"kill-pane\",\n  \"lst-keys\"]"),
        "3:3: error: key `x` in [keys]: unknown command `lst-keys`, did you mean `list-keys`?"
    );
    assert_eq!(
        diag("[keys]\nx = { command = \"slect-pane -L\" }"),
        "2:17: error: key `x` in [keys]: unknown command `slect-pane`, did you mean `select-pane`?"
    );
    // Copy-mode commands (send-keys -X) are lif-core commands.
    let c = Config::parse("[keys.copy-mode-vi]\nv = \"send-keys -X begin-selection\"");
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    assert_eq!(c.command_strings(), ["bind-key -T copy-mode-vi v \"send-keys -X begin-selection\""]);
}

#[test]
fn all_diagnostics_are_returned_with_everything_that_parsed() {
    let c = Config::parse("mouze = true\nbase-index = 1\nmouse = \"on\"\n[keys]\n\"|\" = \"splt-window\"\nx = false\n[theme]\nborder = {}\n");
    let d: Vec<(u32, Severity)> = c.diagnostics.iter().map(|d| (d.line, d.severity)).collect();
    assert_eq!(d, [(1, Severity::Error), (3, Severity::Error), (5, Severity::Error)]);
    assert_eq!(
        c.command_strings(),
        ["set-option -g base-index 1", "set-option -g pane-border-style default", "unbind-key -T prefix x"]
    );
    assert_eq!(c.commands[0], Entry { command: "set-option -g base-index 1".into(), line: 2, column: 14 });
}

#[test]
fn report_names_the_file() {
    let mut c = Config::parse("mouze = true\n");
    assert_eq!(c.report(), "config.toml:1:1: error: unknown option `mouze`, did you mean `mouse`?\n");
    c.path = Some(PathBuf::from("/home/u/.config/lif/config.toml"));
    assert_eq!(c.report(), "/home/u/.config/lif/config.toml:1:1: error: unknown option `mouze`, did you mean `mouse`?\n");
    assert_eq!(Config::parse("").report(), "");
}

#[test]
fn apply_reports_failures_at_their_setting() {
    let c = Config {
        path: None,
        commands: vec![
            Entry { command: "set-option -g mouse off".into(), line: 1, column: 9 },
            Entry { command: "set-option -g mouse maybe".into(), line: 2, column: 9 },
            Entry { command: "bind-key -T prefix x kill-pane".into(), line: 3, column: 5 },
        ],
        startup: vec![],
        diagnostics: vec![],
    };
    let mut m = Model::new("h");
    let d = c.apply(&mut m, &mut TestHost::default());
    assert_eq!(d, [Diagnostic { severity: Severity::Error, line: 2, column: 9, message: "bad value: maybe".into() }]);
    assert_eq!(opt(&m, "mouse"), "off");
    assert!(bound(&m, PREFIX_TABLE, "x").is_some());
}

// --------------------------------------------------------- the example --

#[test]
fn example_parses_cleanly_to_the_expected_commands() {
    let c = Config::parse(example());
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    let want = [
        "set-option -g prefix C-b",
        "set-option -g prefix2 None",
        "set-option -g repeat-time 500",
        "set-option -g mouse on",
        "set-option -g mouse-drag-end copy-pipe-and-cancel",
        "set-option -g mode-keys emacs",
        "set-option -g word-separators \"!\\\"#\\$%&'()*+,-./:;<=>?@[\\\\]^`{|}~\"",
        "set-option -g wrap-search on",
        "set-option -s set-clipboard external",
        "set-option -s copy-command ''",
        "set-option -g scrollback-bytes 10485760",
        "set-option -g history-limit 4000",
        "set-option -g display-time 750",
        "set-option -g display-panes-time 1000",
        "set-option -s escape-time 5",
        "set-option -s exit-empty on",
        "set-option -s buffer-limit 50",
        "set-option -g detach-on-destroy on",
        "set-option -g destroy-unattached off",
        "set-option -s focus-events on",
        "set-option -g base-index 0",
        "set-option -g pane-base-index 0",
        "set-option -g renumber-windows off",
        "set-option -g automatic-rename on",
        "set-option -g automatic-rename-format \"#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}\"",
        "set-option -g remain-on-exit off",
        "set-option -g clock-mode-colour blue",
        "set-option -g clock-mode-style 24",
        "set-option -g synchronize-panes off",
        "set-option -g bell-action any",
        "set-option -g visual-bell off",
        "set-option -g monitor-bell on",
        "set-option -g monitor-activity off",
        "set-option -g activity-action other",
        "set-option -g visual-activity off",
        "set-option -g monitor-silence 0",
        "set-option -g silence-action other",
        "set-option -g visual-silence off",
        "set-option -g default-size 80x24",
        "set-option -g window-size latest",
        "set-option -g aggressive-resize off",
        "set-option -g main-pane-width 80",
        "set-option -g main-pane-height 24",
        "set-option -g other-pane-width 0",
        "set-option -g other-pane-height 0",
        "set-option -g tiled-layout-max-columns 0",
        "set-option -g pane-border-status off",
        "set-option -g pane-border-format \"#{?pane_active,#[reverse],}#{pane_index}#[default] \\\"#{pane_title}\\\"\"",
        "set-option -g popup-border-lines single",
        "set-option -g menu-border-lines single",
        "set-option -g set-titles off",
        "set-option -g set-titles-string \"#S:#I:#W - \\\"#T\\\"\"",
        "set-option -g allow-passthrough off",
        "set-option -s tmux-compat off",
        "set-option -g status on",
        "set-option -g status-position bottom",
        "set-option -g status-justify left",
        "set-option -g status-interval 15",
        "set-option -g status-left \"[#{session_name}] \"",
        "set-option -g status-left-length 10",
        "set-option -g status-right \"#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\\\"#{=21:pane_title}\\\" %H:%M %d-%b-%y\"",
        "set-option -g status-right-length 40",
        "set-option -g window-status-format \"#I:#W#{?window_flags,#{window_flags}, }\"",
        "set-option -g window-status-current-format \"#I:#W#{?window_flags,#{window_flags}, }\"",
        "set-option -g window-status-separator \" \"",
        "set-option -g status-style bg=green,fg=black",
        "set-option -g status-left-style default",
        "set-option -g status-right-style default",
        "set-option -g window-status-style default",
        "set-option -g window-status-current-style underscore",
        "set-option -g window-status-last-style default",
        "set-option -g window-status-activity-style reverse",
        "set-option -g window-status-bell-style reverse",
        "set-option -g pane-border-style default",
        "set-option -g pane-active-border-style fg=green",
        "set-option -g mode-style bg=yellow,fg=black",
        "set-option -g message-style bg=yellow,fg=black",
        "set-option -g copy-mode-match-style bg=cyan,fg=black",
        "set-option -g copy-mode-current-match-style bg=magenta,fg=black",
        "set-option -g window-style default",
        "set-option -g window-active-style default",
        "set-option -g popup-style default",
        "set-option -g popup-border-style default",
        "set-option -g menu-style default",
        "set-option -g menu-selected-style bg=yellow,fg=black",
        "set-option -g menu-border-style default",
        "bind-key -T prefix C-b send-prefix",
        "bind-key -T prefix \\\" split-window",
        "bind-key -T prefix \\% \"split-window -h\"",
        "bind-key -r -T prefix Up \"select-pane -U\"",
    ];
    assert_eq!(c.command_strings(), want);
}

#[test]
fn example_values_are_lif_defaults() {
    // The example documents defaults: applying it changes no option and no
    // binding of a fresh model.
    let mut fresh = Model::new("h");
    let mut m = Model::new("h");
    let c = Config::parse(example());
    assert!(c.apply(&mut m, &mut TestHost::default()).is_empty());
    let ctx = CmdContext::none(WallClock::new(0, 0));
    let show = |m: &mut Model, cmd: &str| m.execute_list(&parse_string(cmd).unwrap(), &ctx, &mut TestHost::default()).unwrap().stdout;
    for cmd in ["show-options -g", "show-options -s", "show-options -gw", "list-keys"] {
        assert_eq!(show(&mut m, cmd), show(&mut fresh, cmd), "{cmd}");
    }
}

#[test]
fn example_commented_settings_are_valid_too() {
    // Uncomment every "# key = value" line of the example: the result must
    // load without errors or warnings.
    let mut out = String::new();
    for line in example().lines() {
        let uncommented = line.strip_prefix("# ").filter(|rest| {
            let key = rest.split(" = ").next().unwrap_or("");
            rest.contains(" = ") && !key.is_empty() && (!key.contains(' ') || key.starts_with('"'))
        });
        out.push_str(uncommented.unwrap_or(line));
        out.push('\n');
    }
    let c = Config::parse(&out);
    assert!(!c.has_errors(), "{}\n{out}", c.report());
    assert_eq!(c.diagnostics.len(), 0, "{}", c.report());
    let cmds = c.command_strings();
    for want in [
        "set-option -g default-shell /bin/zsh",
        "set-option -g default-command ''",
        "bind-key -T prefix | \"split-window -h -c '#{pane_current_path}'\"",
        "bind-key -r -T prefix h \"select-pane -L\"",
        "bind-key -T prefix x kill-pane",
        "unbind-key -T prefix C-o",
        "bind-key -T root M-Right \"select-pane -R\"",
    ] {
        assert!(cmds.iter().any(|c| c == want), "missing {want} in {cmds:#?}");
    }
}

// ------------------------------------------------------------- the path --

#[test]
fn path_resolution() {
    let s = |v: &'static str| Some(OsStr::new(v));
    assert_eq!(resolve_path(s("/etc/lif.toml"), s("/x"), s("/home/u")), Some(PathBuf::from("/etc/lif.toml")));
    assert_eq!(resolve_path(s("rel/lif.toml"), None, None), Some(PathBuf::from("rel/lif.toml")));
    assert_eq!(resolve_path(None, s("/x/cfg"), s("/home/u")), Some(PathBuf::from("/x/cfg/lif/config.toml")));
    assert_eq!(resolve_path(s(""), s("/x/cfg"), s("/home/u")), Some(PathBuf::from("/x/cfg/lif/config.toml")));
    assert_eq!(resolve_path(None, None, s("/home/u")), Some(PathBuf::from("/home/u/.config/lif/config.toml")));
    assert_eq!(resolve_path(None, s(""), s("/home/u")), Some(PathBuf::from("/home/u/.config/lif/config.toml")));
    // A relative XDG_CONFIG_HOME is invalid per the XDG spec.
    assert_eq!(resolve_path(None, s("cfg"), s("/home/u")), Some(PathBuf::from("/home/u/.config/lif/config.toml")));
    assert_eq!(resolve_path(None, None, None), None);
    assert_eq!(resolve_path(None, None, s("")), None);
}

#[test]
fn files_on_disk() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lif-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Missing: empty, no diagnostics.
    let missing = Config::load_file(&dir.join("nope.toml"));
    assert!(missing.commands.is_empty() && missing.diagnostics.is_empty());
    assert_eq!(missing.path, Some(dir.join("nope.toml")));
    // Present.
    let p = dir.join("config.toml");
    std::fs::write(&p, "mouse = false\nmouze = 1\n").unwrap();
    let c = Config::load_file(&p);
    assert_eq!(c.command_strings(), ["set-option -g mouse off"]);
    assert_eq!(c.report(), format!("{}:2:1: error: unknown option `mouze`, did you mean `mouse`?\n", p.display()));
    // Not UTF-8.
    std::fs::write(&p, b"mouse = true\n\xff\xfe").unwrap();
    let c = Config::load_file(&p);
    assert!(c.commands.is_empty());
    assert_eq!(c.diagnostics[0].to_string(), "error: file is not valid UTF-8 (byte 13)");
    // A directory is unreadable as a file.
    let c = Config::load_file(&dir);
    assert!(c.diagnostics[0].message.starts_with("cannot read file"), "{:?}", c.diagnostics);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------- hostile files and input --

/// `Config::load_file` on another thread, with a deadline: a reader that
/// blocks must fail the test, not hang it. `unblock` runs on timeout so the
/// stuck thread can finish.
fn load_with_deadline(path: &std::path::Path, unblock: impl FnOnce()) -> Config {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(Config::load_file(&p));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(c) => c,
        Err(_) => {
            unblock();
            panic!("loading {} did not return within 5 s", path.display());
        }
    }
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lif-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_fifo_is_refused_without_blocking() {
    let dir = scratch_dir("fifo");
    let fifo = dir.join("config.toml");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    // Nobody ever writes to it: a blocking open or read would never return.
    let c = load_with_deadline(&fifo, || {
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    });
    assert!(c.commands.is_empty());
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    assert_eq!(c.diagnostics[0].message, "cannot read file: not a regular file");
    let e = lif_config::read_file_limited(&fifo).unwrap_err();
    assert_eq!(e.to_string(), "not a regular file");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_device_is_refused_without_reading_it() {
    // /dev/zero never ends: reading it to the end would exhaust memory.
    let c = load_with_deadline(std::path::Path::new("/dev/zero"), || {});
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    assert_eq!(c.diagnostics[0].message, "cannot read file: not a regular file");
    assert_eq!(lif_config::read_file_limited(std::path::Path::new("/dev/null")).unwrap_err().to_string(), "not a regular file");
}

#[test]
fn file_size_is_capped() {
    let dir = scratch_dir("size");
    let p = dir.join("config.toml");
    let limit = lif_config::MAX_FILE_SIZE as usize;
    assert_eq!(limit, 1024 * 1024);
    // Exactly at the limit: read whole.
    let mut text = "#".repeat(limit - 15);
    text.push_str("
mouse = false
");
    assert_eq!(text.len(), limit);
    std::fs::write(&p, &text).unwrap();
    assert_eq!(lif_config::read_file_limited(&p).unwrap().len(), limit);
    let c = Config::load_file(&p);
    assert_eq!(c.command_strings(), ["set-option -g mouse off"], "{:?}", c.diagnostics);
    // One byte more: refused, nothing applied.
    text.insert(0, '#');
    std::fs::write(&p, &text).unwrap();
    let c = Config::load_file(&p);
    assert!(c.commands.is_empty());
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    assert_eq!(c.diagnostics[0].message, "cannot read file: file too large (1048577 bytes, limit 1 MiB)");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_symlinked_config_is_followed() {
    let dir = scratch_dir("link");
    let real = dir.join("real.toml");
    std::fs::write(&real, "mouse = false
").unwrap();
    let link = dir.join("config.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(Config::load_file(&link).command_strings(), ["set-option -g mouse off"]);
    // A link to a fifo is refused like the fifo.
    let fifo = dir.join("fifo");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let link2 = dir.join("fifo.toml");
    std::os::unix::fs::symlink(&fifo, &link2).unwrap();
    let c = load_with_deadline(&link2, || {
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    });
    assert_eq!(c.diagnostics[0].message, "cannot read file: not a regular file");
    std::fs::remove_dir_all(&dir).unwrap();
}

const DEEP: usize = 100_000;

#[test]
fn deeply_nested_toml_is_an_error_not_a_crash() {
    for text in [
        format!("mouse = {}{}\n", "[".repeat(DEEP), "]".repeat(DEEP)),
        format!("mouse = {}1{}\n", "{ a = ".repeat(DEEP), " }".repeat(DEEP)),
        format!("{} = 1\n", vec!["a"; DEEP].join(".")),
        format!("[{}]\n", vec!["a"; DEEP].join(".")),
    ] {
        let c = Config::parse(&text);
        assert!(c.commands.is_empty());
        assert!(c.has_errors());
    }
}

#[test]
fn deeply_nested_commands_are_an_error_not_a_crash() {
    let blocks = format!("{}{}", "{ ".repeat(DEEP), " }".repeat(DEEP));
    let ifs = format!("{}display-message x{}", "if-shell true { ".repeat(DEEP), " }".repeat(DEEP));
    // In config.toml key bindings.
    for cmd in [&blocks, &ifs] {
        let text = format!("[keys]\nx = '{cmd}'\ny = ['{cmd}']\nz = {{ command = '{cmd}' }}\n");
        let c = Config::parse(&text);
        assert!(c.commands.is_empty(), "{:?}", c.commands.len());
        assert_eq!(c.diagnostics.len(), 3);
        for d in &c.diagnostics {
            assert!(d.message.ends_with("syntax error: { } blocks nested more than 100 deep"), "{}", &d.message[..d.message.len().min(200)]);
        }
    }
    // In a .tmux.conf.
    for cmd in [&blocks, &ifs] {
        let m = migrate_tmux_conf(&format!("bind x {cmd}\nset -g mouse on\n"));
        assert_eq!(m.untranslated.len(), 1);
        assert_eq!(m.untranslated[0].reason, "syntax error: { } blocks nested more than 100 deep");
        assert!(m.toml.contains("mouse = true"));
    }
    // The server's check for a tmux file given to source-file.
    assert!(!lif_config::looks_like_tmux_commands(&blocks));
    assert!(!lif_config::looks_like_tmux_commands(&format!("# tmux\n{ifs}\n")));
    assert!(lif_config::looks_like_tmux_commands("# tmux\nset -g mouse on\n"));
    assert!(!lif_config::looks_like_tmux_commands("mouse = true\n"));
    // Ordinary nesting still works.
    let ok = format!("{}display-message x{}", "bind-key -n F12 { ".repeat(8), " }".repeat(8));
    let c = Config::parse(&format!("[keys]\nx = '{ok}'\n"));
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(c.commands.len(), 1);
}

// ----------------------------------------------------- tmux.conf migration --

const TMUX_CONF: &str = r##"# ~/.tmux.conf

# Use C-a, like screen
set -g prefix C-a
unbind C-b
bind C-a send-prefix

set -g mouse on
set-option -g history-limit 50000
set -sg escape-time 0
set -g base-index 1
setw -g pane-base-index 1
set -g renumber-windows on
setw -g mode-keys vi
set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",xterm-256color:Tc"
set -g status-position top
set -g status-style 'bg=colour235 fg=colour136'
set -g status-left "#[fg=green]#S "
set -g status-right '#(whoami)@#H %H:%M'
set -g status-interval 5
set -g pane-active-border-style fg=green
set -g @plugin 'tmux-plugins/tpm'

# Splits keep the current directory
bind | split-window -h -c "#{pane_current_path}"
bind - split-window -v -c "#{pane_current_path}"
unbind '"'
unbind %

# vim-style pane movement
bind -r h select-pane -L
bind -r j select-pane -D
bind-key -r -N "Grow left" H resize-pane -L 5
bind -n M-Left select-pane -L
bind -n M-Right select-pane -R
bind r source-file ~/.tmux.conf \; display "Reloaded"
bind x kill-pane \; display-message "killed"
bind-key -T copy-mode-vi v send -X begin-selection
bind -T copy-mode q send -X cancel
bind -T mytable x kill-pane
unbind -a -T root
if-shell "test -f ~/.local.conf" "source ~/.local.conf"
run '~/.tmux/plugins/tpm/tpm'
set -g status-left-length \
    30
setw -g automatic-rename
set mouse off
"##;

#[test]
fn tmux_conf_translation() {
    let m = migrate_tmux_conf(TMUX_CONF);
    let expected_toml = r##"# lif config.toml, translated from .tmux.conf.

# Use C-a, like screen
prefix = "C-a"
mouse = true
history-limit = 50000
escape-time = 0
base-index = 1
pane-base-index = 1
renumber-windows = true
mode-keys = "vi"

[status]
position = "top"
left = "#[fg=green]#S "
right = "#(whoami)@#H %H:%M"
interval = 5
left-length = 30

[theme]
status = "bg=colour235 fg=colour136"
active-border = "fg=green"

[keys]
# Splits keep the current directory
"|" = 'split-window -h -c "#{pane_current_path}"'
"-" = 'split-window -v -c "#{pane_current_path}"'
'"' = false
"%" = false
# vim-style pane movement
h = { command = "select-pane -L", repeat = true }
j = { command = "select-pane -D", repeat = true }
H = { command = "resize-pane -L 5", repeat = true, note = "Grow left" }
x = "kill-pane ; display-message killed"

[keys.root]
M-Left = "select-pane -L"
M-Right = "select-pane -R"

[keys.copy-mode]
q = "send -X cancel"

[keys.copy-mode-vi]
v = "send -X begin-selection"

[startup]
run = [
    "set-option -g @plugin tmux-plugins/tpm",
]
"##;
    assert_eq!(m.toml, expected_toml);
    let un: Vec<(usize, &str)> = m.untranslated.iter().map(|u| (u.line, u.reason.as_str())).collect();
    assert_eq!(
        un,
        [
            (15, "lif has no option default-terminal"),
            (16, "set-option -a has no config.toml equivalent"),
            (
                37,
                "source-file of a tmux configuration file: lif reloads its own config.toml (source-file ~/.config/lif/config.toml)",
            ),
            (41, "key table mytable is not supported (config.toml has prefix, root, copy-mode, copy-mode-vi)"),
            (42, "unbind-key -a has no config.toml equivalent"),
            (43, "it runs source-file on a tmux file: translate that file as well and add what it gives to this file"),
            (
                44,
                "run-shell runs something written for tmux, which would talk to tmux and not to lif: add it to run in [startup] yourself if it works without tmux",
            ),
            (47, "no value given for automatic-rename"),
            (48, "only global options (set -g) can be translated"),
        ]
    );
    assert_eq!(m.untranslated[0].text, "set -g default-terminal \"tmux-256color\"");

    // The translation loads and means what the .tmux.conf meant.
    let c = Config::parse(&m.toml);
    assert!(!c.has_errors(), "{}", c.report());
    assert_eq!(c.startup.iter().map(|e| (e.command.as_str(), e.line)).collect::<Vec<_>>(), [("set-option -g @plugin tmux-plugins/tpm", 48)]);
    let mut model = Model::new("h");
    assert!(c.apply(&mut model, &mut TestHost::default()).is_empty());
    assert_eq!(opt(&model, "prefix"), "C-a");
    assert_eq!(opt(&model, "escape-time"), "0");
    assert_eq!(opt(&model, "status-style"), "bg=colour235 fg=colour136");
    assert_eq!(bound(&model, PREFIX_TABLE, "C-a"), Some((false, None, "send-prefix".into())));
    assert_eq!(bound(&model, PREFIX_TABLE, "C-b"), None);
    assert_eq!(bound(&model, PREFIX_TABLE, "|"), Some((false, None, "split-window -h -c \"#{pane_current_path}\"".into())));
    assert_eq!(bound(&model, PREFIX_TABLE, "H"), Some((true, Some("Grow left".into()), "resize-pane -L 5".into())));
    assert_eq!(bound(&model, PREFIX_TABLE, "%"), None);
    assert_eq!(bound(&model, PREFIX_TABLE, "x"), Some((false, None, "kill-pane ; display-message killed".into())));
    assert_eq!(bound(&model, ROOT_TABLE, "M-Right"), Some((false, None, "select-pane -R".into())));
    assert_eq!(bound(&model, "copy-mode-vi", "v"), Some((false, None, "send-keys -X begin-selection".into())));
}

#[test]
fn tmux_conf_details() {
    // A later setting of the same option or key replaces the earlier one.
    let m = migrate_tmux_conf("set -g mouse on\nset -g mouse off\nbind x kill-pane\nbind x kill-window\n");
    assert_eq!(m.toml, "# lif config.toml, translated from .tmux.conf.\n\nmouse = false\n\n[keys]\nx = \"kill-window\"\n");
    // Without a prefix change, unbinding C-b is kept.
    let m = migrate_tmux_conf("unbind C-b\n");
    assert!(m.toml.contains("C-b = false"), "{}", m.toml);
    // Blocks, single-string commands, odd keys, setw/-w.
    let m = migrate_tmux_conf("bind '\"' { split-window ; select-layout tiled }\nbind ';' 'display-message \"a b\"'\nset -wg monitor-activity on\n");
    assert_eq!(
        m.toml,
        "# lif config.toml, translated from .tmux.conf.\n\nmonitor-activity = true\n\n[keys]\n'\"' = \"split-window ; select-layout tiled\"\n\";\" = 'display-message \"a b\"'\n"
    );
    assert!(m.untranslated.is_empty());
    assert!(Config::parse(&m.toml).diagnostics.is_empty());
    // Bad values and keys are reported, not guessed.
    let m = migrate_tmux_conf("set -g mouse maybe\nset -g base-index one\nbind Bogus kill-pane\nset -g status 2\nbind q\n");
    let r: Vec<&str> = m.untranslated.iter().map(|u| u.reason.as_str()).collect();
    assert_eq!(
        r,
        [
            "value maybe is not valid for mouse",
            "value one is not valid for base-index",
            "unknown key: Bogus",
            "value 2 is not valid for status",
            "bind-key without a command"
        ]
    );
    // Lexer errors are reported for the line.
    let m = migrate_tmux_conf("bind x { kill-pane\n");
    assert_eq!(m.untranslated[0].reason, "syntax error: missing }");
}

#[test]
fn tmux_conf_copy_pipe_and_mouse() {
    // copy-pipe with a command is a binding like any other: lif runs it.
    let m = migrate_tmux_conf("bind -T copy-mode-vi y send -X copy-pipe-and-cancel \"pbcopy\"\nset -s copy-command 'wl-copy'\n");
    assert!(m.untranslated.is_empty(), "{:?}", m.untranslated);
    assert!(m.toml.contains("copy-command = \"wl-copy\""), "{}", m.toml);
    assert!(m.toml.contains("[keys.copy-mode-vi]\ny = \"send -X copy-pipe-and-cancel pbcopy\""), "{}", m.toml);
    assert!(Config::parse(&m.toml).diagnostics.is_empty());
    // MouseDragEnd1Pane is the mouse-drag-end option.
    let m = migrate_tmux_conf("bind -T copy-mode-vi MouseDragEnd1Pane send -X copy-selection-no-clear\n");
    assert!(m.untranslated.is_empty(), "{:?}", m.untranslated);
    assert!(m.toml.contains("mouse-drag-end = \"copy-pipe-no-clear\""), "{}", m.toml);
    let m = migrate_tmux_conf("unbind -T copy-mode MouseDragEnd1Pane\n");
    assert!(m.toml.contains("mouse-drag-end = \"none\""), "{}", m.toml);
    // With a pipe command: the mode is translated, the command reported.
    let m = migrate_tmux_conf("bind -T copy-mode-vi MouseDragEnd1Pane send -X copy-pipe-and-cancel \"pbcopy\"\n");
    assert!(m.toml.contains("mouse-drag-end = \"copy-pipe-and-cancel\""), "{}", m.toml);
    assert_eq!(m.untranslated.len(), 1);
    assert!(m.untranslated[0].reason.contains("copy-command = \"pbcopy\""), "{}", m.untranslated[0].reason);
    assert!(Config::parse(&m.toml).diagnostics.is_empty());
    // Mouse keys of the root table are bindings like any other.
    let m = migrate_tmux_conf(
        "bind -n MouseDown3Pane display-menu -t = -x M -y M Split s 'split-window -h'\nbind -n C-MouseDown1Status kill-window -t =\nunbind -n MouseDown2Pane\n",
    );
    assert!(m.untranslated.is_empty(), "{:?}", m.untranslated.iter().map(|u| &u.reason).collect::<Vec<_>>());
    assert!(m.toml.contains("[keys.root]\nMouseDown3Pane = 'display-menu -t = -x M -y M Split s \"split-window -h\"'\nC-MouseDown1Status = \"kill-window -t =\"\nMouseDown2Pane = false\n"), "{}", m.toml);
    assert!(Config::parse(&m.toml).diagnostics.is_empty());
    // Those lif does not have, and the mouse inside copy mode, say why
    // they are left out.
    let m = migrate_tmux_conf(
        "bind -n MouseDown1ScrollbarUp copy-mode -u\nbind -T copy-mode-vi WheelUpPane send -X -N 2 scroll-up\nbind -T copy-mode-vi MouseDragEnd1Pane send -X select-word\n",
    );
    assert_eq!(m.untranslated[0].reason, "unknown key: MouseDown1ScrollbarUp");
    assert!(m.untranslated[1].reason.contains("mouse actions inside copy mode are built in"), "{}", m.untranslated[1].reason);
    assert!(m.untranslated[2].reason.contains("mouse-drag-end is"), "{}", m.untranslated[2].reason);
}

// ------------------------------------------------------------- robustness --

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn string_values_read_back_exactly() {
    // Whatever a string holds, the option ends up with exactly that value
    // (quoting for the command language must round-trip).
    const CHARS: &[char] = &['a', ' ', '"', '\'', '\\', '$', '#', '{', '}', ';', '~', '%', '\n', '\t', '\r', '\u{1b}', '\u{7f}', 'é', '-', '='];
    let mut rng = XorShift(42);
    for _ in 0..3000 {
        let s: String = (0..rng.below(12)).map(|_| CHARS[rng.below(CHARS.len())]).collect();
        let toml = format!("[status]\nleft = \"{}\"", s.chars().map(|c| format!("\\u{:04X}", c as u32)).collect::<String>());
        let mut m = Model::new("h");
        let c = Config::parse(&toml);
        assert!(c.diagnostics.is_empty(), "{s:?}: {}", c.report());
        assert!(c.apply(&mut m, &mut TestHost::default()).is_empty());
        assert_eq!(opt(&m, "status-left"), s, "{:?}", c.command_strings());
    }
}

#[test]
fn garbage_never_panics() {
    const PIECES: &[&str] = &[
        "[", "]", "[[", "]]", "{", "}", "=", ",", ".", "\"", "'", "\"\"\"", "'''", "\\", "\\u", "\\U0010FFFF", "#", "\n", "\r\n", " ",
        "\t", "keys", "root", "copy-mode-vi", "status", "theme", "prefix", "mouse", "scrollback", "base-index", "C-a", "M-Left", "true",
        "false", "1", "-1", "0x", "9999999999999999999999", "1979-05-27T07:32:00Z", "inf", "nan", "3.5", "\"split-window -h\"",
        "{ command = ", "repeat", "fg", "\"#a6e3a1\"", "é", "\u{0}", "\u{7f}", "😀", ";", "{ fg = \"red\" }", "\"none\"", "set -g ",
        "bind ", "unbind ", "-n ", "-r ", "-T ", "\\;",
    ];
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for round in 0..4000 {
        let mut s = String::new();
        if round % 2 == 0 {
            for _ in 0..rng.below(40) {
                s.push_str(PIECES[rng.below(PIECES.len())]);
            }
        } else {
            let bytes: Vec<u8> = (0..rng.below(120)).map(|_| rng.next() as u8).collect();
            s = String::from_utf8_lossy(&bytes).into_owned();
        }
        let c = Config::parse(&s);
        for d in &c.diagnostics {
            assert!(d.line >= 1 && d.column >= 1, "{s:?}: {d:?}");
        }
        for e in &c.commands {
            parse_string(&e.command).unwrap_or_else(|err| panic!("{s:?}: {} -> {err}", e.command));
        }
        // Whatever the translator writes is valid TOML.
        let m = migrate_tmux_conf(&s);
        let back = Config::parse(&m.toml);
        assert!(!back.diagnostics.iter().any(|d| d.message.starts_with("invalid TOML")), "{s:?}\n{}\n{}", m.toml, back.report());
    }
}

// ----------------------------------------------------- hooks and env --

#[test]
fn hooks_and_env_sections_give_the_exact_commands() {
    let c = Config::parse(
        r##"
[hooks]
after-split-window = "select-layout tiled"
session-created = ["run-shell 'echo #{hook_session_name}'", "display-message hi"]
pane-exited = false

[env]
EDITOR = "nvim -u NONE"
SSH_ASKPASS = false
"-odd" = "-v"
"##,
    );
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    assert_eq!(
        c.command_strings(),
        [
            "set-hook -g after-split-window \"select-layout tiled\"",
            "set-hook -g session-created \"run-shell 'echo #{hook_session_name}'\"",
            "set-hook -ga session-created \"display-message hi\"",
            "set-hook -gu pane-exited",
            "set-environment -g -- EDITOR \"nvim -u NONE\"",
            "set-environment -gr -- SSH_ASKPASS",
            "set-environment -g -- -odd -v",
        ]
    );
    // They mean what they say.
    let mut m = Model::new("h");
    assert!(c.apply(&mut m, &mut TestHost::default()).is_empty());
    let ctx = CmdContext::none(WallClock::new(0, 0));
    let mut show = |cmd: &str| m.execute_list(&parse_string(cmd).unwrap(), &ctx, &mut TestHost::default()).unwrap().stdout;
    assert_eq!(show("show-hooks -g session-created"), "session-created[0] run-shell \"echo #{hook_session_name}\"\nsession-created[1] display-message hi\n");
    assert_eq!(show("show-environment -g"), "-odd=-v\nEDITOR=nvim -u NONE\n-SSH_ASKPASS\n");
    // Applying the file again (a reload) does not pile hooks up.
    assert!(c.apply(&mut m, &mut TestHost::default()).is_empty());
    let mut show = |cmd: &str| m.execute_list(&parse_string(cmd).unwrap(), &ctx, &mut TestHost::default()).unwrap().stdout;
    assert_eq!(show("show-hooks -g session-created").lines().count(), 2);
}

#[test]
fn hooks_and_env_mistakes_are_diagnosed() {
    assert_eq!(diag("[hooks]\nafter-new-windw = \"display hi\""), "2:1: error: unknown hook `after-new-windw`, did you mean `after-new-window`?");
    assert_eq!(diag("[hooks]\npane-exited = \"frobnicate\""), "2:15: error: hook `pane-exited` in [hooks]: unknown command `frobnicate`");
    assert_eq!(
        diag("[hooks]\npane-exited = 3"),
        "2:15: error: hook `pane-exited` in [hooks]: expected a command string, a list of them, or false for none, found integer 3"
    );
    assert_eq!(diag("[hooks]\npane-exited = [\"display a\", 4]"), "2:29: error: hook `pane-exited` in [hooks]: expected a command string, found integer 4");
    assert_eq!(diag("hooks = 1"), "1:1: error: `hooks` must be a table of hooks ([hooks]), found integer 1");
    assert_eq!(diag("[env]\nA = 1"), "2:5: error: variable `A` in [env]: expected a string, or false to remove the variable, found integer 1");
    assert_eq!(diag("[env]\n\"A=B\" = \"x\""), "2:1: error: `A=B` is not a variable name (it is empty or contains `=`)");
    assert_eq!(diag("env = \"x\""), "1:1: error: `env` must be a table of environment variables ([env]), found string \"x\"");
    assert_eq!(diag("[hook]\nx = 1"), "1:2: error: unknown section [hook], did you mean [hooks]?");
}

#[test]
fn tmux_conf_hooks_and_environment_are_translated() {
    let m = migrate_tmux_conf(
        "# keep it tidy\nset-hook -g after-split-window 'select-layout tiled'\n\
         set-hook -g session-created 'display one'\nset-hook -ga session-created 'display two'\n\
         set-hook -gu pane-exited\nset-hook after-new-window 'display x'\nset-hook -g no-such-hook 'display x'\n\
         setenv -g EDITOR vim\nset-environment -gr DISPLAY\nsetenv LOCAL 1\nsetenv -gu GONE\n",
    );
    assert_eq!(
        m.toml,
        "# lif config.toml, translated from .tmux.conf.\n\n[hooks]\n# keep it tidy\nafter-split-window = \"select-layout tiled\"\n\
         session-created = [\"display one\", \"display two\"]\npane-exited = false\n\n[env]\nEDITOR = \"vim\"\nDISPLAY = false\n"
    );
    let un: Vec<(usize, &str)> = m.untranslated.iter().map(|u| (u.line, u.reason.as_str())).collect();
    assert_eq!(
        un,
        [
            (6, "only global hooks (set-hook -g) can be translated"),
            (7, "lif has no hook no-such-hook"),
            (10, "only the global environment (set-environment -g) can be translated"),
            (11, "set-environment -u has no config.toml equivalent"),
        ]
    );
    let c = Config::parse(&m.toml);
    assert!(c.diagnostics.is_empty(), "{}", c.report());
}

/// `[startup]`: commands the server runs once when it starts. They are
/// not among the settings, and are checked without being run.
#[test]
fn startup_commands() {
    let c = Config::parse("mouse = false\n[startup]\nrun = [\n  \"new-session -d -s main\",\n  \"run-shell 'touch /nonexistent/x'\",\n]\n");
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    assert_eq!(c.command_strings(), ["set-option -g mouse off"]);
    assert_eq!(
        c.startup,
        [
            Entry { command: "new-session -d -s main".into(), line: 4, column: 3 },
            Entry { command: "run-shell 'touch /nonexistent/x'".into(), line: 5, column: 3 },
        ]
    );
    // Applying the settings runs none of them.
    let mut m = Model::new("h");
    assert!(c.apply(&mut m, &mut TestHost::default()).is_empty());
    assert_eq!(m.sessions().count(), 0);
    // One command needs no list.
    assert_eq!(Config::parse("[startup]\nrun = \"display-message hi\"\n").startup.len(), 1);
    for (text, want) in [
        ("[startup]\nrun = [\"splt-window\"]\n", "2:8: error: run in [startup]: unknown command `splt-window`, did you mean `split-window`?"),
        ("[startup]\nrun = [5]\n", "2:8: error: run in [startup]: expected a command string, found integer 5"),
        ("[startup]\nrun = true\n", "2:7: error: run in [startup]: expected a command string or a list of them, found boolean true"),
        ("[startup]\nran = \"x\"\n", "2:1: error: unknown key `ran` in [startup], did you mean `run`? (it has one key: run)"),
        ("startup = 3\n", "1:1: error: `startup` must be a table ([startup] with run = [...]), found integer 3"),
        ("[startup]\nrun = [\"customize-mode\"]\n", "2:8: error: run in [startup]: unsupported command: customize-mode"),
    ] {
        let c = Config::parse(text);
        assert_eq!(c.diagnostics.iter().map(|d| d.to_string()).collect::<Vec<_>>(), [want], "{text}");
        assert!(c.startup.is_empty(), "{text}");
    }
    // From a .tmux.conf: commands that are not settings, with their
    // comments; not what would read a tmux file or talk to tmux.
    let m = migrate_tmux_conf(
        "set -g @theme 'dark blue'\n# a session to start with\nnew-session -d -s main\nif-shell 'test -n \"$SSH_TTY\"' 'set -g status-style bg=red'\nrun 'date > /dev/null'\nrun '~/.tmux/plugins/tpm/tpm'\nsource-file ~/.tmux/theme.conf\nif-shell true 'source ~/x.conf'\n",
    );
    assert_eq!(
        m.toml,
        "# lif config.toml, translated from .tmux.conf.\n\n[startup]\nrun = [\n    'set-option -g @theme \"dark blue\"',\n    # a session to start with\n    \"new-session -d -s main\",\n    'if-shell \"test -n \\\"\\$SSH_TTY\\\"\" \"set -g status-style bg=red\"',\n    'run-shell \"date > /dev/null\"',\n]\n"
    );
    assert_eq!(m.untranslated.iter().map(|u| u.line).collect::<Vec<_>>(), [6, 7, 8]);
    assert!(m.untranslated[0].reason.starts_with("run-shell runs something written for tmux"), "{}", m.untranslated[0].reason);
    assert!(m.untranslated[1].reason.starts_with("source-file names a tmux file"), "{}", m.untranslated[1].reason);
    assert!(m.untranslated[2].reason.starts_with("it runs source-file on a tmux file"), "{}", m.untranslated[2].reason);
    let c = Config::parse(&m.toml);
    assert!(c.diagnostics.is_empty(), "{}", c.report());
    assert_eq!(c.startup.len(), 4);
}
