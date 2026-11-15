//! A model to try commands on while loading, so that values lif-core
//! rejects are reported with their position in the file.

use lif_core::{
    CaptureRange, ClientId, CmdContext, CopyModeOptions, DetachReason, Host, Model, PaneId, PaneInfo, PaneInput, Prompt,
    Redraw, SessionId, SpawnSpec, WallClock, parse_string,
};
use lif_types::Size;

/// A host for a model without panes or clients: config commands never
/// need one, and anything that would is refused.
pub(crate) struct NullHost;

impl Host for NullHost {
    fn spawn_pane(&mut self, _spec: &SpawnSpec) -> Result<(), String> {
        Err("not allowed in a config file".to_string())
    }
    fn kill_pane(&mut self, _pane: PaneId) {}
    fn resize_pane(&mut self, _pane: PaneId, _size: Size) {}
    fn send_to_pane(&mut self, _pane: PaneId, _input: PaneInput<'_>) -> Result<(), String> {
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

/// A fresh model with lif's defaults, the commands of the file run on it
/// one by one.
pub(crate) struct Scratch {
    pub(crate) model: Model,
}

impl Scratch {
    pub(crate) fn new() -> Scratch {
        Scratch { model: Model::new("lif-config") }
    }

    /// Whether lif-core knows a (global) option by this exact name.
    pub(crate) fn has_option(&self, name: &str) -> bool {
        self.model.option_text(name, None, None).is_some()
    }

    /// Run one command string; lif-core's error message on failure.
    pub(crate) fn run(&mut self, cmd: &str) -> Result<(), String> {
        let list = parse_string(cmd).map_err(|e| e.0)?;
        let ctx = CmdContext::none(WallClock::new(0, 0));
        self.model.execute_list(&list, &ctx, &mut NullHost).map(|_| ()).map_err(|e| e.0)
    }
}
