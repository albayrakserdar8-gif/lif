use super::*;

fn argv(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

fn one(s: &[&str]) -> Command {
    parse(&argv(s)).unwrap()
}

fn err(s: &[&str]) -> String {
    parse_list(&argv(s)).unwrap_err().0
}

#[test]
fn names_aliases_prefixes() {
    assert_eq!(one(&["new"]).name(), "new-session");
    assert_eq!(one(&["a"]).name(), "attach-session");
    assert_eq!(one(&["attach"]).name(), "attach-session");
    assert_eq!(one(&["ls"]).name(), "list-sessions");
    assert_eq!(one(&["splitw"]).name(), "split-window");
    assert_eq!(one(&["splitp"]).name(), "split-window");
    assert_eq!(one(&["split-pane"]).name(), "split-window");
    assert_eq!(one(&["neww"]).name(), "new-window");
    assert_eq!(one(&["selectp"]).name(), "select-pane");
    assert_eq!(one(&["killp"]).name(), "kill-pane");
    assert_eq!(one(&["last"]).name(), "last-window");
    assert_eq!(one(&["set", "-g", "status", "off"]).name(), "set-option");
    assert_eq!(one(&["show"]).name(), "show-options");
    assert_eq!(one(&["split"]).name(), "split-window");
    assert_eq!(one(&["has"]).name(), "has-session");
    assert_eq!(one(&["kill-ser"]).name(), "kill-server");
    // Messages verified against tmux 3.8.
    assert_eq!(err(&["foo"]), "unknown command: foo");
    assert_eq!(err(&["sel"]), "ambiguous command: sel, could be: select-layout, select-pane, select-window");
    assert_eq!(err(&["kill-s"]), "ambiguous command: kill-s, could be: kill-server, kill-session");
    assert_eq!(err(&["switch-mode"]), "unsupported command: switch-mode");
    assert_eq!(parse(&["choose-window".to_string()]).unwrap().name(), "choose-tree", "tmux's command-alias");
}

#[test]
fn flags_like_getopt() {
    let c = one(&["split-window", "-dh", "-tfoo", "-l", "30%", "vim", "-x"]);
    assert!(c.args().has('d') && c.args().has('h'));
    assert_eq!(c.args().get('t'), Some("foo"));
    assert_eq!(c.args().get('l'), Some("30%"));
    assert_eq!(c.args().strings(0), vec!["vim", "-x"]);
    let c = one(&["split-window", "-h", "--", "-x"]);
    assert_eq!(c.args().value(0), Some("-x"));
    let c = one(&["send-keys", "-t", "-1", "-", "a"]);
    assert_eq!(c.args().get('t'), Some("-1"));
    assert_eq!(c.args().strings(0), vec!["-", "a"]);
    let c = one(&["select-window", "-t", "1", "-t", "2"]);
    assert_eq!(c.args().get('t'), Some("2"));
    assert_eq!(one(&["new", "-dd"]).args().count('d'), 2);
    // tmux 3.8 messages.
    assert_eq!(err(&["kill-pane", "-z"]), "command kill-pane: unknown flag -z");
    assert_eq!(err(&["kill-pane", "a", "b"]), "command kill-pane: too many arguments (need at most 0)");
    assert_eq!(err(&["rename-session"]), "command rename-session: too few arguments (need at least 1)");
    assert_eq!(err(&["split-window", "-t"]), "command split-window: -t expects an argument");
    assert_eq!(err(&["bind-key"]), "command bind-key: too few arguments (need at least 1)");
    assert_eq!(err(&["kill-pane", "-%"]), "command kill-pane: invalid flag -%");
}

#[test]
fn argv_command_lists() {
    let l = parse_list(&argv(&["new", "-d", ";", "splitw", "-h"])).unwrap();
    assert_eq!(l.to_string(), "new-session -d ; split-window -h");
    let l = parse_list(&argv(&["new", "-d;", "splitw"])).unwrap();
    assert_eq!(l.commands().len(), 2);
    let l = parse_list(&argv(&["display", "-p", "a\\;", ";"])).unwrap();
    assert_eq!(l.commands().len(), 1);
    assert_eq!(l.commands()[0].args().value(0), Some("a;"));
    assert!(parse(&argv(&["new", ";", "ls"])).is_err());
}

#[test]
fn command_strings() {
    let l = parse_string("new-window -n 'my win' ; split-window -h \"vim  x\"").unwrap();
    assert_eq!(l.commands().len(), 2);
    assert_eq!(l.commands()[0].args().get('n'), Some("my win"));
    assert_eq!(l.commands()[1].args().value(0), Some("vim  x"));
    let l = parse_string("display -p \"a\\tb\\\"c\\u00e9\\101\"").unwrap();
    assert_eq!(l.commands()[0].args().value(0), Some("a\tb\"céA"));
    let l = parse_string("bind x split-window \\; new-window").unwrap();
    assert_eq!(l.commands()[0].args().strings(0), vec!["x", "split-window", ";", "new-window"]);
    let l = parse_string("display #{pane_id} # trailing comment\n# full comment\nls").unwrap();
    assert_eq!(l.commands().len(), 2);
    assert_eq!(l.commands()[0].args().value(0), Some("#{pane_id}"));
    let l = parse_string("display -p #{?pane_active,a b,c}").unwrap();
    assert_eq!(l.commands()[0].args().value(0), Some("#{?pane_active,a b,c}"));
    let l = parse_string("bind -n M-x { split-window -h ; select-pane -L }").unwrap();
    let c = &l.commands()[0];
    match &c.args().values()[1] {
        Arg::Block(b) => assert_eq!(b.to_string(), "split-window -h ; select-pane -L"),
        other => panic!("{other:?}"),
    }
    let l = parse_string("new -d\\\n -s x").unwrap();
    assert_eq!(l.commands()[0].args().get('s'), Some("x"));
    let l = parse_string("display a;b").unwrap();
    assert_eq!(l.commands()[0].args().value(0), Some("a;b"));
    let l = parse_string("display a; ls").unwrap();
    assert_eq!(l.commands().len(), 2);
    assert!(parse_string("display 'oops").is_err());
    assert!(parse_string("bind x { ls").is_err());
    assert!(parse_string("").unwrap().is_empty());
}

#[test]
fn printing_matches_tmux_list_keys() {
    let p = |s: &str| parse_string(s).unwrap().to_string();
    assert_eq!(p("splitw -h -c '#{pane_current_path}'"), "split-window -h -c \"#{pane_current_path}\"");
    assert_eq!(p("command-prompt -I '#S' { rename-session '%%' }"), "command-prompt -I \"#S\" { rename-session \"%%\" }");
    assert_eq!(p("confirm-before -p \"kill-pane #P? (y/n)\" kill-pane"), "confirm-before -p \"kill-pane #P? (y/n)\" kill-pane");
    assert_eq!(p("select-window -t :=0"), "select-window -t :=0");
    assert_eq!(p("new -s ''"), "new-session -s ''");
    assert_eq!(p("display 'a\"b'"), "display-message 'a\"b'");
    assert_eq!(p("display 'a\"b c$'"), "display-message \"a\\\"b c\\$\"");
    assert_eq!(p("display '~/x'"), "display-message \\~/x");
    assert_eq!(p("resizep -Z -t 1 -x 5"), "resize-pane -Z -t 1 -x 5");
    assert_eq!(escape_arg("\""), "\\\"");
    assert_eq!(escape_arg("#"), "\\#");
    assert_eq!(escape_arg("$"), "\\$");
    assert_eq!(escape_arg(";"), "\\;");
    assert_eq!(escape_arg("~"), "\\~");
    assert_eq!(escape_arg("a"), "a");
    let l = parse_list(&argv(&["a", "-t", "x", ";", "ls"])).unwrap();
    assert_eq!(l.print(true), "attach-session -t x \\; list-sessions");
}

#[test]
fn round_trip_through_printing() {
    for s in [
        "split-window -h -c \"#{pane_current_path}\"",
        "command-prompt -I \"#W\" { rename-window \"%%\" }",
        "bind-key -r -T prefix M-Up resize-pane -U 5",
        "display-message -p \"#{session_name}: #{?session_attached,yes,no}\"",
        "send-keys -t %1 'echo \"hi\"' Enter",
    ] {
        let once = parse_string(s).unwrap().to_string();
        let twice = parse_string(&once).unwrap().to_string();
        assert_eq!(once, twice, "{s}");
    }
}

#[test]
fn escape_arg_round_trips() {
    // xorshift64*, so failures reproduce.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    const ALPHABET: &[char] = &[
        'a', 'Z', '0', '-', ' ', '"', '\'', '\\', '$', '#', ';', '{', '}', '%', '~', '\n', '\t', '\r', '\x01', '\x1b',
        '\x7f', '\0', 'é', '中', '👍', '\u{301}', '=', ',', '!', '*',
    ];
    let check = |s: &str| {
        let q = escape_arg(s);
        let line = format!("display-message -- {q}");
        let list = parse_string(&line).unwrap_or_else(|e| panic!("{s:?} escaped as {q:?}: {e}"));
        let got = list.commands().first().and_then(|c| c.args().value(0)).map(str::to_string);
        assert_eq!(got.as_deref(), Some(s), "{s:?} escaped as {q:?}");
    };
    // The reported case: only `"` special, plus a backslash or a control.
    for s in ["a\"b\\c", "\"\n", "x\"\x01", "\\\"", "\"\x7f\""] {
        check(s);
    }
    for _ in 0..20_000 {
        let len = (next() % 8) as usize + 1;
        let s: String = (0..len).map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize]).collect();
        check(&s);
    }
}
