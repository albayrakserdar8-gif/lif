//! Key tables against tmux 3.8's own \`list-keys\` output, and key handling.

mod common;
use common::*;
use lif_copy::{Action, Command, KeyTable, LEFT_OUT, ModeKeys, Outcome};
use lif_core::KeyCode;

const TMUX_KEYS: &str = include_str!("fixtures/tmux-3.8-copy-mode-keys.txt");

fn unescape(k: &str) -> String {
    let k = k.trim_matches('"');
    k.strip_prefix('\\').unwrap_or(k).to_string()
}

#[test]
fn every_tmux_default_binding_is_present() {
    let vi = KeyTable::defaults(ModeKeys::Vi);
    let emacs = KeyTable::defaults(ModeKeys::Emacs);
    let mut checked = 0;
    let mut left_out = Vec::new();
    for line in TMUX_KEYS.lines() {
        let mut it = line.split_whitespace();
        assert_eq!(it.next(), Some("bind-key"));
        assert_eq!(it.next(), Some("-T"));
        let table_name = it.next().unwrap();
        let key = unescape(it.next().unwrap());
        let rest: Vec<&str> = it.collect();
        let rest = rest.join(" ");
        let table = if table_name == "copy-mode-vi" { &vi } else { &emacs };
        if ["Mouse", "Wheel", "DoubleClick", "TripleClick"].iter().any(|p| key.contains(p)) {
            // Mouse bindings are CopyMode::handle_mouse's job.
            continue;
        }
        let code = KeyCode::parse(&key).unwrap_or_else(|| panic!("{key}"));
        let got = table.get(&code);
        let want = if let Some(cmd) = rest.strip_prefix("send-keys -X ") {
            let pipe = cmd.trim().starts_with("copy-pipe");
            Command::from_name(cmd.trim()).map(if pipe { Action::Pipe } else { Action::Command })
        } else if rest.starts_with("send-keys -FX search-forward") {
            Some(Action::SearchWord { forward: true })
        } else if rest.starts_with("send-keys -FX search-backward") {
            Some(Action::SearchWord { forward: false })
        } else if rest.contains("(repeat)") {
            Some(Action::Count(key.chars().last().unwrap().to_digit(10).unwrap() as u8))
        } else if let Some(i) = rest.find("send-keys -X ") {
            let cmd = rest[i + 13..].split_whitespace().next().unwrap();
            Command::from_name(cmd).map(Action::Ask)
        } else {
            panic!("unexpected binding: {line}");
        };
        match want {
            Some(w) => {
                assert_eq!(got, Some(w), "{table_name} {key}: {rest}");
                checked += 1;
            }
            None => {
                assert_eq!(got, None, "{table_name} {key}");
                left_out.push(format!("{table_name} {key}: {rest}"));
            }
        }
    }
    assert!(checked > 140, "{checked}");
    // Only the bindings listed in LEFT_OUT are missing.
    assert_eq!(left_out.len(), LEFT_OUT.len(), "{left_out:?}");
    for (t, k, _) in LEFT_OUT {
        assert!(left_out.iter().any(|l| l.starts_with(&format!("{t} {k}:"))), "{t} {k}");
    }
}

#[test]
fn tables_are_data() {
    let mut t = KeyTable::defaults(ModeKeys::Vi);
    t.bind_str("y", "copy-selection-and-cancel").unwrap();
    t.bind_str("C-M-p", "ask search-backward").unwrap();
    assert!(t.bind_str("y", "no-such-command").is_err());
    assert!(t.bind_str("Nope-Key", "cancel").is_err());
    assert!(t.unbind(&KeyCode::parse("q").unwrap()));
    assert_eq!(t.get(&KeyCode::parse("q").unwrap()), None);
    assert_eq!(t.get(&KeyCode::parse("y").unwrap()), Some(Action::Command(Command::CopySelectionAndCancel)));
}

#[test]
fn counts_jumps_prompts_and_exit_vi() {
    let text = "alpha beta gamma\ndelta epsilon\nzeta eta theta\niota kappa\nlambda mu";
    let (mut v, mut m) = pane(20, 3, text, ModeKeys::Vi);
    keys(&mut m, &mut v, &["g"]);
    assert_eq!(st(&m), (0, 0, 2));
    keys(&mut m, &mut v, &["2", "j"]);
    assert_eq!(st(&m), (0, 2, 2), "2j");
    keys(&mut m, &mut v, &["1", "0", "l"]);
    assert_eq!(m.cursor().col, 10, "10l ('0' extends a pending count)");
    keys(&mut m, &mut v, &["0"]);
    assert_eq!(m.cursor().col, 0, "0 alone is start-of-line");
    // f then a character jumps; ; repeats; Escape cancels a pending jump.
    assert_eq!(m.handle_key(&key("f"), &mut v), Outcome::Redraw);
    assert_eq!(m.pending_label(), Some("(jump forward)"));
    // Row 2 is "zeta eta theta".
    m.handle_key(&ch('t'), &mut v);
    assert_eq!(m.cursor().col, 2);
    keys(&mut m, &mut v, &[";"]);
    assert_eq!(m.cursor().col, 6);
    keys(&mut m, &mut v, &["f", "Escape"]);
    assert_eq!(m.cursor().col, 6);
    // / opens a prompt; accepting searches.
    match m.handle_key(&key("/"), &mut v) {
        Outcome::Prompt { label, incremental, .. } => {
            assert_eq!(label, "(search down)");
            assert!(!incremental);
        }
        o => panic!("{o:?}"),
    }
    let mut o = m.prompt_accept(&mut v, "kap+a");
    while o == Outcome::Searching {
        o = m.search_step(&mut v, 100);
    }
    assert_eq!(o, Outcome::Redraw);
    assert_eq!((m.cursor().row, m.cursor().col), (3, 5));
    // : goto-line
    assert!(matches!(m.handle_key(&key(":"), &mut v), Outcome::Prompt { label: "(goto line)", .. }));
    m.prompt_accept(&mut v, "2");
    assert_eq!(m.position().0, 2);
    // * searches the word under the cursor.
    keys(&mut m, &mut v, &["g", "w", "*"]);
    assert_eq!(m.search_string(), Some("beta"));
    // Space, motion, Enter copies and exits.
    keys(&mut m, &mut v, &["g", "Space", "e"]);
    let o = m.handle_key(&key("Enter"), &mut v);
    // Enter is copy-pipe-and-cancel: piped to copy-command when one is set.
    assert_eq!(o, Outcome::Copy { data: b"alpha".to_vec(), append: false, exit: true, pipe: true });
    assert_eq!(m.handle_key(&key("q"), &mut v), Outcome::Exit);
    assert_eq!(m.handle_key(&key("C-c"), &mut v), Outcome::Exit);
    // Unbound keys do nothing and drop a pending count.
    keys(&mut m, &mut v, &["5"]);
    assert_eq!(m.pending_count(), Some(5));
    assert_eq!(m.handle_key(&key("F12"), &mut v), Outcome::None);
    assert_eq!(m.pending_count(), None);
}

#[test]
fn emacs_meta_counts_and_incremental_prompt() {
    let text = "one two three\nfour five six\nseven eight";
    let (mut v, mut m) = pane(20, 3, text, ModeKeys::Emacs);
    keys(&mut m, &mut v, &["M-<", "M-2", "C-f"]);
    assert_eq!(m.cursor().col, 2);
    keys(&mut m, &mut v, &["M-3", "M-f"]);
    assert_eq!(m.cursor().col, 13, "three words forward, emacs end is after the word");
    match m.handle_key(&key("C-s"), &mut v) {
        Outcome::Prompt { incremental: true, label: "(search down)", .. } => {}
        o => panic!("{o:?}"),
    }
    assert_eq!(m.handle_key(&key("Escape"), &mut v), Outcome::Exit, "Escape cancels in emacs");
}
