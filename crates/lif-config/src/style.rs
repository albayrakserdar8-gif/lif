//! Theme styles: readable tables (`{ fg = "black", bold = true }`) and tmux
//! style strings (`"fg=black,bold"`), both turned into tmux style strings.

const NAMES: &[&str] = &["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/// What a color may be, for error messages.
pub(crate) const COLOR_HELP: &str =
    "a color: a name (black, red, green, yellow, blue, magenta, cyan, white, or bright...), colour0 to colour255, #rrggbb, or default";

/// The message for a color that does not parse; `hint` is a "did you mean".
pub(crate) fn unknown_color(c: &str, hint: &str) -> String {
    if hint.is_empty() { format!("unknown color {c:?} (expected {COLOR_HELP})") } else { format!("unknown color {c:?}{hint}") }
}

/// Every color name, for suggestions.
pub(crate) fn color_names() -> Vec<String> {
    let mut v: Vec<String> = NAMES.iter().map(|n| n.to_string()).collect();
    v.extend(NAMES.iter().map(|n| format!("bright{n}")));
    v.push("default".into());
    v
}

/// Whether lif-core's style parser accepts this color (same rules).
pub(crate) fn valid_color(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    if l == "default" || l == "terminal" || NAMES.contains(&l.as_str()) {
        return true;
    }
    if let Some(rest) = l.strip_prefix("bright") {
        return NAMES.contains(&rest);
    }
    if let Some(n) = l.strip_prefix("colour").or_else(|| l.strip_prefix("color")) {
        return n.parse::<u8>().is_ok();
    }
    if let Some(hex) = l.strip_prefix('#') {
        return hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    matches!(l.parse::<u8>(), Ok(90..=97))
}

/// Keys of a style table.
pub(crate) const TABLE_KEYS: &[&str] =
    &["fg", "bg", "underline-color", "bold", "dim", "italic", "underline", "blink", "reverse", "hidden", "strikethrough", "overline"];

/// The tmux attribute for a boolean style-table key.
pub(crate) fn attr(key: &str) -> Option<&'static str> {
    Some(match key {
        "bold" => "bold",
        "dim" => "dim",
        "italic" => "italics",
        "blink" => "blink",
        "reverse" => "reverse",
        "hidden" => "hidden",
        "strikethrough" => "strikethrough",
        "overline" => "overline",
        _ => return None,
    })
}

/// Values of `underline = "..."`.
pub(crate) const UNDERLINES: &[(&str, &str)] = &[
    ("single", "underscore"),
    ("double", "double-underscore"),
    ("curly", "curly-underscore"),
    ("dotted", "dotted-underscore"),
    ("dashed", "dashed-underscore"),
];

const ATTRS: &[&str] = &[
    "bright",
    "bold",
    "dim",
    "italics",
    "blink",
    "reverse",
    "hidden",
    "strikethrough",
    "overline",
    "underscore",
    "double-underscore",
    "curly-underscore",
    "dotted-underscore",
    "dashed-underscore",
];

/// Check a tmux style string (`fg=black,bg=green,bold`). lif-core ignores
/// what it does not understand; a config file should say so instead.
pub(crate) fn check_string(s: &str) -> Result<(), String> {
    for tok in s.split([',', ' ']).filter(|t| !t.is_empty()) {
        match tok.split_once('=') {
            Some((k @ ("fg" | "bg" | "us"), v)) => {
                if !valid_color(v) {
                    let names = color_names();
                    let hint = crate::suggest::did_you_mean(v, names.iter().map(|s| s.as_str()), ("\"", "\""));
                    return Err(format!("in `{k}={v}`: {}", unknown_color(v, &hint)));
                }
            }
            Some((k, _)) => return Err(format!("unsupported style directive `{k}=` (lif styles take fg=, bg= and us=)")),
            None => {
                let name = tok.strip_prefix("no").filter(|n| *n != "ne").unwrap_or(tok);
                if !(tok == "default" || tok == "none" || tok == "noattr" || ATTRS.contains(&name)) {
                    let hint = crate::suggest::did_you_mean(tok, ATTRS.iter().copied(), ("`", "`"));
                    return Err(format!("unknown style attribute `{tok}`{hint}"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        for c in ["black", "BrightRed", "colour0", "color255", "#a6e3a1", "default", "terminal", "91"] {
            assert!(valid_color(c), "{c}");
        }
        for c in ["blak", "colour256", "#abc", "#gggggg", "", "bright", "98"] {
            assert!(!valid_color(c), "{c}");
        }
    }

    #[test]
    fn style_strings() {
        assert!(check_string("fg=black,bg=green").is_ok());
        assert!(check_string("bold nounderscore default").is_ok());
        assert!(check_string("").is_ok());
        assert_eq!(
            check_string("fg=blak").unwrap_err(),
            "in `fg=blak`: unknown color \"blak\", did you mean \"black\"?"
        );
        assert_eq!(check_string("bolt").unwrap_err(), "unknown style attribute `bolt`, did you mean `bold`?");
        assert!(check_string("align=right").unwrap_err().contains("unsupported style directive"));
    }
}
