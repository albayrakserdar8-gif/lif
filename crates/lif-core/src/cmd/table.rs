//! The command table: every tmux 3.8 command name and alias (so prefixes are
//! ambiguous exactly when they are in tmux), with lif's flag templates for
//! the ones it implements.

use super::CmdError;

/// One command's definition.
#[derive(Debug, PartialEq, Eq)]
pub struct CmdEntry {
    /// Canonical name.
    pub name: &'static str,
    /// Short alias (`splitw`), if any.
    pub alias: Option<&'static str>,
    /// getopt-style flag template: a letter per flag, `:` after one that
    /// takes a value.
    pub template: &'static str,
    /// Minimum number of positional arguments.
    pub lower: usize,
    /// Maximum number of positional arguments (`None`: unlimited).
    pub upper: Option<usize>,
    /// Whether lif implements the command.
    pub supported: bool,
}

const fn c(name: &'static str, alias: Option<&'static str>, template: &'static str, lower: usize, upper: Option<usize>) -> CmdEntry {
    CmdEntry { name, alias, template, lower, upper, supported: true }
}

const fn u(name: &'static str, alias: Option<&'static str>) -> CmdEntry {
    CmdEntry { name, alias, template: "", lower: 0, upper: None, supported: false }
}

const N: Option<usize> = None;
const Z: Option<usize> = Some(0);
const O: Option<usize> = Some(1);

/// The table, sorted by name.
pub(crate) static TABLE: &[CmdEntry] = &[
    c("attach-session", Some("attach"), "c:dt:x", 0, Z),
    c("bind-key", Some("bind"), "nrN:T:", 1, N),
    c("break-pane", Some("breakp"), "abdPF:n:s:t:", 0, Z),
    c("capture-pane", Some("capturep"), "b:eE:JNpqS:t:", 0, Z),
    u("choose-buffer", None),
    u("choose-client", None),
    u("choose-tree", None),
    c("clear-history", Some("clearhist"), "Ht:", 0, Z),
    u("clear-prompt-history", Some("clearphist")),
    u("clock-mode", None),
    c("command-prompt", None, "1FI:ikNPp:T:t:", 0, O),
    c("confirm-before", Some("confirm"), "bc:p:t:y", 1, O),
    c("copy-mode", None, "deHMqt:u", 0, Z),
    u("customize-mode", None),
    c("delete-buffer", Some("deleteb"), "b:", 0, Z),
    c("detach-client", Some("detach"), "aPs:t:", 0, Z),
    u("display-menu", Some("menu")),
    c("display-message", Some("display"), "c:d:lpt:F:", 0, O),
    u("display-popup", Some("popup")),
    c("display-panes", Some("displayp"), "d:Nt:", 0, O),
    u("find-window", Some("findw")),
    c("has-session", Some("has"), "t:", 0, Z),
    c("if-shell", Some("if"), "bFt:", 2, Some(3)),
    c("join-pane", Some("joinp"), "bdhl:p:s:t:v", 0, Z),
    c("kill-pane", Some("killp"), "at:", 0, Z),
    c("kill-server", None, "", 0, Z),
    c("kill-session", None, "aCt:", 0, Z),
    c("kill-window", Some("killw"), "at:", 0, Z),
    c("last-pane", Some("lastp"), "t:Z", 0, Z),
    c("last-window", Some("last"), "t:", 0, Z),
    u("link-window", Some("linkw")),
    c("list-buffers", Some("lsb"), "F:f:", 0, Z),
    c("list-clients", Some("lsc"), "F:f:t:", 0, Z),
    c("list-commands", Some("lscm"), "F:", 0, O),
    c("list-keys", Some("lsk"), "1NP:T:", 0, O),
    c("list-panes", Some("lsp"), "aF:f:st:", 0, Z),
    c("list-sessions", Some("ls"), "F:f:", 0, Z),
    c("list-windows", Some("lsw"), "aF:f:t:", 0, Z),
    c("load-buffer", Some("loadb"), "b:t:w", 1, O),
    u("lock-client", Some("lockc")),
    u("lock-server", Some("lock")),
    u("lock-session", Some("locks")),
    u("move-pane", Some("movep")),
    c("move-window", Some("movew"), "abdkrs:t:", 0, Z),
    u("new-pane", Some("newp")),
    c("new-session", Some("new"), "Ac:dDe:EF:n:Ps:Xx:y:", 0, N),
    c("new-window", Some("neww"), "abc:de:F:kn:PSt:", 0, N),
    c("next-layout", Some("nextl"), "t:", 0, Z),
    c("next-window", Some("next"), "at:", 0, Z),
    c("paste-buffer", Some("pasteb"), "b:dprs:t:", 0, Z),
    u("pipe-pane", Some("pipep")),
    c("previous-layout", Some("prevl"), "t:", 0, Z),
    c("previous-window", Some("prev"), "at:", 0, Z),
    c("refresh-client", Some("refresh"), "St:", 0, Z),
    c("rename-session", Some("rename"), "t:", 1, O),
    c("rename-window", Some("renamew"), "t:", 1, O),
    c("resize-pane", Some("resizep"), "DLRt:Ux:y:Z", 0, O),
    u("resize-window", Some("resizew")),
    c("respawn-pane", Some("respawnp"), "c:e:kt:", 0, N),
    u("respawn-window", Some("respawnw")),
    c("rotate-window", Some("rotatew"), "Dt:UZ", 0, Z),
    c("run-shell", Some("run"), "bCc:d:Et:", 0, O),
    c("save-buffer", Some("saveb"), "ab:", 1, O),
    c("select-layout", Some("selectl"), "Enopt:", 0, O),
    c("select-pane", Some("selectp"), "DLlRT:t:UZ", 0, Z),
    c("select-window", Some("selectw"), "lnpTt:", 0, Z),
    c("send-keys", Some("send"), "FHlN:RXt:", 0, N),
    c("send-prefix", None, "2t:", 0, Z),
    u("server-access", None),
    c("set-buffer", Some("setb"), "ab:n:", 0, O),
    c("set-environment", Some("setenv"), "Fghrt:u", 1, Some(2)),
    c("set-hook", None, "agpRt:uw", 1, N),
    c("set-option", Some("set"), "aFgopqst:uw", 1, Some(2)),
    c("set-window-option", Some("setw"), "aFgoqt:u", 1, Some(2)),
    c("show-buffer", Some("showb"), "b:", 0, Z),
    c("show-environment", Some("showenv"), "ghst:", 0, O),
    c("show-hooks", None, "gpt:w", 0, O),
    c("show-messages", Some("showmsgs"), "JTt:", 0, Z),
    c("show-options", Some("show"), "gpqst:vw", 0, O),
    u("show-prompt-history", Some("showphist")),
    c("show-window-options", Some("showw"), "gvt:", 0, O),
    c("source-file", Some("source"), "Fnqvt:", 1, N),
    c("split-window", Some("splitw"), "bc:de:F:hl:p:Pt:vZ", 0, N),
    c("start-server", Some("start"), "", 0, Z),
    c("suspend-client", Some("suspendc"), "t:", 0, Z),
    c("swap-pane", Some("swapp"), "dDs:t:UZ", 0, Z),
    c("swap-window", Some("swapw"), "ds:t:", 0, Z),
    c("switch-client", Some("switchc"), "c:lnpt:T:Z", 0, Z),
    u("switch-mode", None),
    c("unbind-key", Some("unbind"), "anqT:", 0, O),
    u("unlink-window", Some("unlinkw")),
    c("wait-for", Some("wait"), "LSU", 1, O),
];

/// tmux's default `command-alias` entries.
const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("split-pane", "split-window"),
    ("splitp", "split-window"),
    ("server-info", "show-messages"),
    ("info", "show-messages"),
    ("choose-window", "choose-tree"),
    ("choose-session", "choose-tree"),
];

/// Find a command by name, alias or unambiguous prefix (tmux's `cmd_find`).
pub(crate) fn lookup_entry(name: &str) -> Result<&'static CmdEntry, CmdError> {
    let name = COMMAND_ALIASES.iter().find(|(a, _)| *a == name).map(|(_, t)| *t).unwrap_or(name);
    if let Some(e) = TABLE.iter().find(|e| e.alias == Some(name) || e.name == name) {
        return Ok(e);
    }
    let matches: Vec<&CmdEntry> = TABLE.iter().filter(|e| !name.is_empty() && e.name.starts_with(name)).collect();
    match matches.as_slice() {
        [] => Err(CmdError(format!("unknown command: {name}"))),
        [one] => Ok(one),
        many => Err(CmdError(format!(
            "ambiguous command: {name}, could be: {}",
            many.iter().map(|e| e.name).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// Every command name lif implements, for `list-commands`.
pub(crate) fn supported() -> impl Iterator<Item = &'static CmdEntry> {
    TABLE.iter().filter(|e| e.supported)
}
