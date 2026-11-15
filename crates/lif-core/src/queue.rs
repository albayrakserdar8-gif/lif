//! Commands that wait, and the shell commands of formats.
//!
//! `run-shell` and `if-shell` wait for a shell command the server runs,
//! `wait-for` for another client. The model never blocks: a command that
//! has to wait leaves a [`Block`] behind, and the list it is part of is
//! parked with the commands still to run. When the server reports the job
//! ([`Model::job_done`]) or the channel is signalled, the list carries on
//! where it stopped, and its outcome is handed to the server through
//! [`Model::take_finished`]. `if-shell` and `run-shell -C` put their
//! commands at the front of the list they are part of, as tmux inserts
//! them into its command queue.
//!
//! Only a list run from the outside (a client's command, a key binding)
//! is waited for. A list run by another command (`source-file`) parks
//! what is left of it and the outer command carries on without it.
//!
//! `#(command)` in a format never waits: it expands to the last line the
//! command printed the previous time and asks for a new run when that is
//! older than `status-interval` ([`FormatJobs`]).

use crate::cmd::{Arg, Args, CmdError, Command, CommandList, commands_from_args, parse_string};
use crate::format;
use crate::host::{Host, JobSpec, JobStatus, Redraw};
use crate::ids::{ClientId, SessionId};
use crate::model::{CmdContext, CmdOutput, Model};
use crate::target::{Flags, Found, Kind};
use crate::time::WallClock;
use std::collections::{BTreeMap, VecDeque};

/// Most command lists parked at once; a command that would park another
/// fails. Each holds its remaining commands and the output so far.
pub(crate) const MAX_PARKED: usize = 1024;

/// Most `wait-for` channels that are locked or signalled ahead of a
/// waiter (tmux keeps those too, without a bound).
pub(crate) const MAX_CHANNELS: usize = 1024;

/// Most `#()` commands remembered. A status line has a few; past this the
/// one used longest ago is forgotten.
pub(crate) const MAX_FORMAT_JOBS: usize = 64;

/// Longest `#()` output kept (its last line), in bytes.
const MAX_FORMAT_JOB_OUTPUT: usize = 4096;

/// Longest `run-shell -d` delay: tmux's limit for a timer in
/// milliseconds elsewhere (UINT_MAX, about 49 days).
const MAX_DELAY_MS: f64 = u32::MAX as f64;

/// What a parked list waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitKey {
    /// A job or timer the server reports with this id.
    Job(u64),
    /// `wait-for name`: until `wait-for -S name`.
    Channel(String),
    /// `wait-for -L name`: until the lock is free.
    Lock(String),
}

/// What happens with a job's result before the list carries on.
#[derive(Clone, Debug)]
pub(crate) enum Then {
    /// Nothing (a timer, a channel).
    Nothing,
    /// `run-shell`: its output is the command's, a failure sets the status.
    Shell { command: String },
    /// `run-shell -d`: the delay is over, start the command.
    StartShell(JobSpec),
    /// `run-shell -d -C`: run these next.
    Commands(CommandList),
    /// `if-shell`: run `yes` if the job exited with 0, else `no`.
    Branch { yes: Arg, no: Option<Arg> },
}

/// Left behind by a command that waits.
#[derive(Debug)]
pub(crate) struct Block {
    pub key: WaitKey,
    pub then: Then,
    /// `-b`: the list does not wait; only `then` happens later.
    pub background: bool,
}

/// A parked command list.
#[derive(Debug)]
pub(crate) struct Run {
    ctx: CmdContext,
    rest: VecDeque<Command>,
    out: String,
    status: i32,
    key: WaitKey,
    then: Then,
    /// Somebody is waiting for the outcome (a client's command).
    waited: bool,
    /// The list is a hook's: what it does fires no hooks.
    muted: bool,
}

/// A command list to run (from the start, or from where it was parked).
struct Work {
    rest: VecDeque<Command>,
    ctx: CmdContext,
    /// Output and status of the commands that ran before it was parked.
    out: String,
    status: i32,
    waited: bool,
    muted: bool,
}

/// How far [`Model::drive`] got.
struct Outcome {
    out: String,
    status: i32,
    error: Option<CmdError>,
    /// Parked, with somebody waiting for it.
    pending: bool,
    /// Parked: `out` is only what it gives its caller now.
    parked: bool,
}

/// The outcome of a command list that had been parked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    /// The client that ran it, if any (it may be gone by now).
    pub client: Option<ClientId>,
    /// The list was run from the outside and reported
    /// [`CmdOutput::pending`]: this is the answer its caller still owes.
    /// Otherwise nobody waits (`run-shell -b`, a list inside
    /// `source-file`): show the output if there is somewhere to show it.
    pub waited: bool,
    /// Output of the commands that ran.
    pub stdout: String,
    /// The exit status for the client (see [`CmdOutput::status`]).
    pub status: i32,
    /// The command that failed, which ended the list.
    pub error: Option<CmdError>,
}

#[derive(Debug, Default)]
pub(crate) struct Channel {
    locked: bool,
    /// Signalled with nobody waiting: the next `wait-for` returns at once.
    woken: bool,
}

/// The state of waiting commands.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    /// Lists being run inside each other right now.
    depth: u32,
    draining: bool,
    pub block: Option<Block>,
    /// Commands to run next in the current list.
    pub insert: Option<CommandList>,
    parked: Vec<Run>,
    ready: VecDeque<Run>,
    finished: Vec<Finished>,
    next_id: u64,
    channels: BTreeMap<String, Channel>,
}

/// One `#()` command.
#[derive(Debug)]
struct FormatJob {
    command: String,
    /// Last line of its last output.
    out: String,
    /// When it last started and was last expanded (Unix seconds).
    started: i64,
    used: i64,
    running: Option<u64>,
    /// A format asked for a new run; `start_format_jobs` starts it.
    wanted: bool,
    /// For the job's environment and directory: where it was last expanded.
    session: Option<SessionId>,
}

/// The `#()` commands of formats. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct FormatJobs {
    jobs: Vec<FormatJob>,
    /// Counts expansions, so the server can tell a status line that uses
    /// `#()` (and so wants redrawing every `status-interval`).
    uses: u64,
}

impl FormatJobs {
    /// `#(command)` expanded at `now`: its last output; a new run is
    /// asked for when the last one started `interval` seconds ago or more.
    pub(crate) fn expand(&mut self, command: &str, session: Option<SessionId>, now: i64, interval: i64) -> String {
        self.uses = self.uses.wrapping_add(1);
        let i = match self.jobs.iter().position(|j| j.command == command) {
            Some(i) => i,
            None => {
                if self.jobs.len() >= MAX_FORMAT_JOBS {
                    let Some(old) = (0..self.jobs.len()).filter(|i| self.jobs[*i].running.is_none()).min_by_key(|i| self.jobs[*i].used) else {
                        return String::new();
                    };
                    self.jobs.swap_remove(old);
                }
                self.jobs.push(FormatJob { command: command.to_string(), out: String::new(), started: i64::MIN, used: now, running: None, wanted: false, session });
                self.jobs.len() - 1
            }
        };
        let j = &mut self.jobs[i];
        j.used = now;
        j.session = session;
        if j.running.is_none() && now.saturating_sub(j.started) >= interval.max(1) {
            j.wanted = true;
        }
        j.out.clone()
    }
}

fn push_front(rest: &mut VecDeque<Command>, list: CommandList) {
    for c in list.0.into_iter().rev() {
        rest.push_front(c);
    }
}

/// `run-shell -d`: seconds, with a fraction, to milliseconds.
fn parse_delay(d: &str) -> Result<u64, CmdError> {
    match d.parse::<f64>() {
        Ok(s) if s >= 0.0 && s * 1000.0 <= MAX_DELAY_MS => Ok((s * 1000.0) as u64),
        _ => Err(CmdError(format!("invalid delay time: {d}"))),
    }
}

type R = Result<CmdOutput, CmdError>;

impl Model {
    /// Run a list of commands in order, stopping at the first error. The
    /// output of the commands that ran is concatenated. If a command has
    /// to wait (`run-shell`, `if-shell`, `wait-for`) the result says
    /// [`CmdOutput::pending`] and the outcome comes later through
    /// [`Model::take_finished`].
    pub fn execute_list(&mut self, list: &CommandList, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let work = Work { rest: list.0.iter().cloned().collect(), ctx: *ctx, out: String::new(), status: 0, waited: true, muted: false };
        let o = self.drive(work, host);
        match o.error {
            Some(e) => Err(e),
            None => Ok(CmdOutput { stdout: o.out, status: o.status, pending: o.pending }),
        }
    }

    /// Runs the list until it ends, fails or has to wait.
    fn drive(&mut self, work: Work, host: &mut dyn Host) -> Outcome {
        let Work { mut rest, ctx, mut out, mut status, waited, muted } = work;
        let nested = self.q.depth > 0;
        let waited = waited && !nested;
        let was_muted = self.hooks.muted;
        let muted = muted || was_muted;
        self.hooks.muted = muted;
        self.q.depth += 1;
        let mut error = None;
        let (mut pending, mut parked) = (false, false);
        while let Some(c) = rest.pop_front() {
            self.last_found.set(None);
            match self.execute(&c, &ctx, host) {
                Ok(o) => {
                    out.push_str(&o.stdout);
                    if o.status != 0 {
                        status = o.status;
                    }
                }
                Err(e) => {
                    self.q.block = None;
                    self.q.insert = None;
                    error = Some(e);
                    break;
                }
            }
            self.command_ran(&c, &ctx);
            if let Some(list) = self.q.insert.take() {
                push_front(&mut rest, list);
            }
            let Some(b) = self.q.block.take() else { continue };
            if b.background {
                self.q.parked.push(Run { ctx, rest: VecDeque::new(), out: String::new(), status: 0, key: b.key, then: b.then, waited: false, muted });
                continue;
            }
            if !nested {
                // The hooks of what ran so far run now, not when the wait
                // is over; their output goes where the commands' goes.
                self.hooks.muted = was_muted;
                self.run_pending_hooks(ctx.now, Some(&mut out), host);
                self.hooks.muted = muted;
            }
            // A waited list keeps its output for whoever waits; an inner
            // list gives its caller what it has and finishes on its own.
            let kept = if waited { std::mem::take(&mut out) } else { String::new() };
            self.q.parked.push(Run { ctx, rest, out: kept, status, key: b.key, then: b.then, waited, muted });
            pending = waited;
            parked = true;
            break;
        }
        self.q.depth -= 1;
        self.hooks.muted = was_muted;
        if !nested && !parked {
            self.run_pending_hooks(ctx.now, Some(&mut out), host);
        }
        self.drain_ready(host);
        Outcome { out, status, error, pending, parked }
    }

    /// Carries on with the lists whose wait is over. Only the outermost
    /// caller does: a list woken from inside a command starts after it.
    fn drain_ready(&mut self, host: &mut dyn Host) {
        if self.q.draining || self.q.depth > 0 {
            return;
        }
        self.q.draining = true;
        while let Some(run) = self.q.ready.pop_front() {
            let (client, was_waited) = (run.ctx.client, run.waited);
            let work = Work { rest: run.rest, ctx: run.ctx, out: run.out, status: run.status, waited: run.waited, muted: run.muted };
            let o = self.drive(work, host);
            // Parked again: nothing to report yet, unless nobody waits
            // and it printed something on the way.
            if o.pending || (o.parked && o.out.is_empty()) {
                continue;
            }
            self.q.finished.push(Finished { client, waited: was_waited, stdout: o.out, status: o.status, error: o.error });
        }
        self.q.draining = false;
    }

    /// Runs one command list of a hook: its output and the error that
    /// ended it. If it has to wait, the rest finishes on its own.
    pub(crate) fn run_hook_list(&mut self, list: &CommandList, ctx: &CmdContext, host: &mut dyn Host) -> (String, Option<CmdError>) {
        let work = Work { rest: list.0.iter().cloned().collect(), ctx: *ctx, out: String::new(), status: 0, waited: false, muted: true };
        let o = self.drive(work, host);
        (o.out, o.error)
    }

    pub(crate) fn push_finished(&mut self, f: Finished) {
        self.q.finished.push(f);
    }

    /// The outcomes of parked command lists that have finished since the
    /// last call, oldest first.
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.q.finished)
    }

    /// Whether [`Model::take_finished`] has anything.
    pub fn has_finished(&self) -> bool {
        !self.q.finished.is_empty()
    }

    /// How many command lists are parked (for tests and statistics).
    pub fn parked(&self) -> usize {
        self.q.parked.len()
    }

    fn next_job(&mut self) -> u64 {
        self.q.next_id += 1;
        self.q.next_id
    }

    fn check_park(&self) -> Result<(), CmdError> {
        if self.q.parked.len() >= MAX_PARKED {
            return Err(CmdError(format!("too many waiting commands ({MAX_PARKED})")));
        }
        Ok(())
    }

    /// A job or timer started through [`Host::start_job`] /
    /// [`Host::start_timer`] has ended. `output` is what it printed (the
    /// server bounds it). Unknown ids are ignored.
    pub fn job_done(&mut self, id: u64, status: JobStatus, output: &[u8], now: WallClock, host: &mut dyn Host) {
        if self.format_job_done(id, output, host) {
            return;
        }
        let Some(i) = self.q.parked.iter().position(|r| r.key == WaitKey::Job(id)) else { return };
        let mut run = self.q.parked.remove(i);
        run.ctx.now = now;
        let mut error = None;
        match std::mem::replace(&mut run.then, Then::Nothing) {
            Then::Nothing => {}
            Then::Shell { command } => {
                let text = String::from_utf8_lossy(output);
                run.out.push_str(&text);
                if !text.is_empty() && !text.ends_with('\n') {
                    run.out.push('\n');
                }
                // tmux's wording, and its exit status for the client.
                match status {
                    JobStatus::Exited(0) => {}
                    JobStatus::Exited(n) => {
                        run.out.push_str(&format!("'{command}' returned {n}\n"));
                        run.status = n;
                    }
                    JobStatus::Signaled(s) => {
                        run.out.push_str(&format!("'{command}' terminated by signal {s}\n"));
                        run.status = 128 + s;
                    }
                }
            }
            Then::StartShell(mut spec) => {
                spec.id = self.next_job();
                match host.start_job(&spec) {
                    Ok(()) => {
                        run.key = WaitKey::Job(spec.id);
                        run.then = Then::Shell { command: spec.command };
                        self.q.parked.push(run);
                        return;
                    }
                    Err(e) => error = Some(CmdError(e)),
                }
            }
            Then::Commands(list) => push_front(&mut run.rest, list),
            Then::Branch { yes, no } => {
                let pick = if status == JobStatus::Exited(0) { Some(yes) } else { no };
                if let Some(arg) = pick {
                    match commands_from_args(&[arg]) {
                        Ok(list) => push_front(&mut run.rest, list),
                        Err(e) => error = Some(e),
                    }
                }
            }
        }
        if error.is_some() {
            self.q.finished.push(Finished { client: run.ctx.client, waited: run.waited, stdout: run.out, status: run.status, error });
            return;
        }
        self.q.ready.push_back(run);
        self.drain_ready(host);
    }

    /// A client is gone: its `wait-for` commands stop waiting (tmux
    /// removes them too). Lists waiting for a job finish without it.
    pub(crate) fn forget_waiter(&mut self, client: ClientId) {
        self.q.parked.retain(|r| r.ctx.client != Some(client) || matches!(r.key, WaitKey::Job(_)));
    }

    /// The target of `run-shell` and `if-shell`: the pane `-t` names, or
    /// the current one if there is any (they also run with no session).
    fn job_target(&self, a: &Args, ctx: &CmdContext) -> Result<Option<Found>, CmdError> {
        match self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default()) {
            Ok(f) => Ok(Some(f)),
            Err(e) if a.get('t').is_some() => Err(e),
            Err(_) => Ok(None),
        }
    }

    fn job_expand(&self, f: Option<&Found>, ctx: &CmdContext, host: &dyn Host, s: &str) -> String {
        format::expand(s, &self.fctx(host, f.map(|f| f.s), f.and_then(|f| f.w), f.and_then(|f| f.p), ctx.client, ctx.now))
    }

    fn job_spec(&mut self, command: String, cwd: String, f: Option<&Found>, stderr: bool) -> JobSpec {
        let session = f.map(|f| f.s);
        JobSpec {
            id: self.next_job(),
            command,
            cwd,
            session,
            pane: f.and_then(|f| f.p),
            stderr,
            env: self.job_env(session),
            tmux_compat: self.gopt("tmux-compat").as_flag(),
        }
    }

    pub(crate) fn cmd_run_shell(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let delay = a.get('d').map(parse_delay).transpose()?;
        let f = self.job_target(a, ctx)?;
        let background = a.has('b');
        self.check_park()?;
        let Some(command) = a.value(0) else {
            // Only a delay (or nothing at all, which tmux also accepts).
            if let Some(ms) = delay {
                let id = self.next_job();
                host.start_timer(id, ms).map_err(CmdError)?;
                self.q.block = Some(Block { key: WaitKey::Job(id), then: Then::Nothing, background });
            }
            return Ok(CmdOutput::default());
        };
        if a.has('C') {
            let list = parse_string(command)?;
            match delay {
                Some(ms) => {
                    let id = self.next_job();
                    host.start_timer(id, ms).map_err(CmdError)?;
                    self.q.block = Some(Block { key: WaitKey::Job(id), then: Then::Commands(list), background });
                }
                None => self.q.insert = Some(list),
            }
            return Ok(CmdOutput::default());
        }
        let command = self.job_expand(f.as_ref(), ctx, &*host, command);
        let cwd = self.job_cwd(a, f.as_ref(), ctx, &*host);
        let spec = self.job_spec(command, cwd, f.as_ref(), a.has('E'));
        let then = match delay {
            Some(ms) => {
                host.start_timer(spec.id, ms).map_err(CmdError)?;
                Then::StartShell(spec.clone())
            }
            None => {
                host.start_job(&spec).map_err(CmdError)?;
                Then::Shell { command: spec.command.clone() }
            }
        };
        self.q.block = Some(Block { key: WaitKey::Job(spec.id), then, background });
        Ok(CmdOutput::default())
    }

    pub(crate) fn cmd_if_shell(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let f = self.job_target(a, ctx)?;
        let condition = self.job_expand(f.as_ref(), ctx, &*host, a.value(0).unwrap_or(""));
        let yes = a.values()[1].clone();
        let no = a.values().get(2).cloned();
        if a.has('F') {
            let pick = if format::is_true(&condition) { Some(yes) } else { no };
            if let Some(arg) = pick {
                self.q.insert = Some(commands_from_args(&[arg])?);
            }
            return Ok(CmdOutput::default());
        }
        self.check_park()?;
        let cwd = self.job_cwd(a, f.as_ref(), ctx, &*host);
        let spec = self.job_spec(condition, cwd, f.as_ref(), false);
        host.start_job(&spec).map_err(CmdError)?;
        self.q.block = Some(Block { key: WaitKey::Job(spec.id), then: Then::Branch { yes, no }, background: a.has('b') });
        Ok(CmdOutput::default())
    }

    fn channel(&mut self, name: &str) -> Result<&mut Channel, CmdError> {
        if !self.q.channels.contains_key(name) && self.q.channels.len() >= MAX_CHANNELS {
            return Err(CmdError(format!("too many channels ({MAX_CHANNELS})")));
        }
        Ok(self.q.channels.entry(name.to_string()).or_default())
    }

    /// Forget a channel nothing is recorded for.
    fn tidy_channel(&mut self, name: &str) {
        if self.q.channels.get(name).is_some_and(|c| !c.locked && !c.woken) {
            self.q.channels.remove(name);
        }
    }

    pub(crate) fn cmd_wait_for(&mut self, a: &Args, ctx: &CmdContext) -> R {
        let name = a.value(0).unwrap_or("");
        if a.has('S') {
            let key = WaitKey::Channel(name.to_string());
            let mut woke = false;
            let mut i = 0;
            while i < self.q.parked.len() {
                if self.q.parked[i].key == key {
                    let mut run = self.q.parked.remove(i);
                    run.ctx.now = ctx.now;
                    self.q.ready.push_back(run);
                    woke = true;
                } else {
                    i += 1;
                }
            }
            if !woke {
                self.channel(name)?.woken = true;
            }
            return Ok(CmdOutput::default());
        }
        if a.has('L') {
            self.check_park()?;
            let ch = self.channel(name)?;
            if ch.locked {
                self.q.block = Some(Block { key: WaitKey::Lock(name.to_string()), then: Then::Nothing, background: false });
            } else {
                ch.locked = true;
            }
            return Ok(CmdOutput::default());
        }
        if a.has('U') {
            if !self.q.channels.get(name).is_some_and(|c| c.locked) {
                return Err(CmdError(format!("channel {name} not locked")));
            }
            let key = WaitKey::Lock(name.to_string());
            // The lock goes to whoever has waited longest, or is free.
            if let Some(i) = self.q.parked.iter().position(|r| r.key == key) {
                let mut run = self.q.parked.remove(i);
                run.ctx.now = ctx.now;
                self.q.ready.push_back(run);
            } else {
                self.q.channels.get_mut(name).expect("checked").locked = false;
                self.tidy_channel(name);
            }
            return Ok(CmdOutput::default());
        }
        if self.q.channels.get(name).is_some_and(|c| c.woken) {
            self.q.channels.get_mut(name).expect("checked").woken = false;
            self.tidy_channel(name);
            return Ok(CmdOutput::default());
        }
        self.check_park()?;
        self.q.block = Some(Block { key: WaitKey::Channel(name.to_string()), then: Then::Nothing, background: false });
        Ok(CmdOutput::default())
    }

    // ----- #() ---------------------------------------------------------

    /// How often formats have expanded a `#()` so far. A status line whose
    /// rendering moves this uses shell commands and should be redrawn
    /// every `status-interval`.
    pub fn format_job_uses(&self) -> u64 {
        self.fjobs.borrow().uses
    }

    /// Start the `#()` commands that formats asked for since the last
    /// call. Call after expanding formats that are shown (the status
    /// line) and after running commands.
    pub fn start_format_jobs(&mut self, now: WallClock, host: &mut dyn Host) {
        if !self.fjobs.borrow().jobs.iter().any(|j| j.wanted) {
            return;
        }
        let wanted: Vec<(String, Option<SessionId>)> =
            self.fjobs.borrow().jobs.iter().filter(|j| j.wanted).map(|j| (j.command.clone(), j.session)).collect();
        for (command, session) in wanted {
            let session = session.filter(|s| self.sessions.contains_key(s));
            let cwd = session.and_then(|s| self.sessions.get(&s)).map(|s| s.cwd.clone()).unwrap_or_default();
            let spec = JobSpec {
                id: self.next_job(),
                command,
                cwd,
                session,
                pane: None,
                stderr: false,
                env: self.job_env(session),
                tmux_compat: self.gopt("tmux-compat").as_flag(),
            };
            let started = host.start_job(&spec).is_ok();
            let mut fj = self.fjobs.borrow_mut();
            let Some(j) = fj.jobs.iter_mut().find(|j| j.command == spec.command) else { continue };
            j.wanted = false;
            // A command that could not start is tried again after the
            // interval, like one that ran.
            j.started = now.unix_secs;
            if started {
                j.running = Some(spec.id);
            }
        }
    }

    /// If `id` is a `#()` command: keep the last line it printed and
    /// redraw the status lines.
    fn format_job_done(&mut self, id: u64, output: &[u8], host: &mut dyn Host) -> bool {
        let mut fj = self.fjobs.borrow_mut();
        let Some(j) = fj.jobs.iter_mut().find(|j| j.running == Some(id)) else { return false };
        j.running = None;
        let text = String::from_utf8_lossy(output);
        let line = text.lines().rev().find(|l| !l.is_empty()).unwrap_or("");
        let line: String = line.chars().filter(|c| !c.is_control()).collect();
        let mut end = line.len().min(MAX_FORMAT_JOB_OUTPUT);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        let changed = j.out != line[..end];
        if changed {
            j.out = line[..end].to_string();
        }
        drop(fj);
        if changed {
            for s in self.sessions.keys() {
                host.redraw(Redraw::Status(*s));
            }
        }
        true
    }
}
