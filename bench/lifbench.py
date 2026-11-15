#!/usr/bin/env python3
"""lif benchmark harness: compares terminal multiplexers on one machine.

Usage:
  python3 bench/lifbench.py run [--tools none,tmux,zellij,lif] [--quick] [--only m1,m2]
  python3 bench/lifbench.py report RESULT.json... [-o OUT.md]

Every multiplexer runs in a private namespace (own socket dir, HOME, config)
and is driven through a client attached to a PTY owned by lifbench-driver
(bench/driver). See bench/README.md for the methodology.
"""

import argparse
import datetime
import json
import os
import platform
import random
import shutil
import signal
import statistics
import subprocess
import sys
import time
import traceback

BENCH = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(BENCH)
TMP = os.path.join(BENCH, "tmp")
# ".noindex" keeps Spotlight from indexing 240 MB of test data and the
# session dirs churned during a run.
DATA = os.path.join(TMP, "data.noindex")
RUNS = os.path.join(TMP, "r.noindex")
RESULTS = os.path.join(BENCH, "results")
TARGET = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target-bench"))
DRIVER = os.path.join(TARGET, "release", "lifbench-driver")
COLS, ROWS = 200, 50
MB = 1_000_000

# Keys typed into a pane for latency samples: printable, not used by the
# flood ("y"), not in either tool's status bar, rotated so that a stale
# redraw of an earlier sample cannot be mistaken for the current echo.
ECHO_CHARS = b"{}~^`"
ESC_MARK = b"~"
# attach-detach sends the detach keys once the client has been silent this
# long after its first output, i.e. it has drawn the session. The hyperfine
# time includes this wait.
ATTACH_QUIET_MS = 20


def log(*a):
    print("[lifbench]", *a, file=sys.stderr, flush=True)


def sh(argv, env=None, check=True, timeout=60):
    r = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"{argv!r} failed ({r.returncode}): {r.stderr.strip()}")
    return r


def pid_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def ps_table():
    """pid -> (ppid, rss_kb, cputime, command)."""
    out = sh(["ps", "-axww", "-o", "pid=,ppid=,rss=,time=,command="]).stdout
    t = {}
    for line in out.splitlines():
        parts = line.split(None, 4)
        if len(parts) < 5:
            continue
        t[int(parts[0])] = (int(parts[1]), int(parts[2]), parts[3], parts[4])
    return t


def cputime_s(s):
    # ps TIME is [dd-][hh:]mm:ss.cc
    days = 0
    if "-" in s:
        d, s = s.split("-", 1)
        days = int(d)
    secs = 0.0
    for p in s.split(":"):
        secs = secs * 60 + float(p)
    return days * 86400 + secs


def rusage(pids):
    if not pids:
        return []
    out = sh([DRIVER, "rusage"] + [str(p) for p in pids]).stdout
    return [json.loads(l) for l in out.splitlines() if l.strip()]


def stats(values):
    v = sorted(values)
    if not v:
        return {"n": 0}

    def pct(p):
        if len(v) == 1:
            return v[0]
        k = (len(v) - 1) * p / 100
        f = int(k)
        c = min(f + 1, len(v) - 1)
        return v[f] + (v[c] - v[f]) * (k - f)

    return {
        "n": len(v), "median": pct(50), "min": v[0], "max": v[-1],
        "p25": pct(25), "p75": pct(75), "p95": pct(95), "p99": pct(99),
        "mean": statistics.fmean(v),
        "stdev": statistics.stdev(v) if len(v) > 1 else 0.0,
    }


# --------------------------------------------------------------------------
# Test data (deterministic: fixed seed; regenerated only if missing)


def gen_data():
    os.makedirs(DATA, exist_ok=True)
    files = {
        "ascii": (os.path.join(DATA, "ascii-100MB.txt"), gen_ascii),
        "sgr": (os.path.join(DATA, "sgr-utf8-100MB.txt"), gen_sgr),
        "longline": (os.path.join(DATA, "longline-20MB.txt"), gen_longline),
        "history": (os.path.join(DATA, "history-100k-lines.txt"), gen_history),
    }
    out = {}
    for name, (path, fn) in files.items():
        if not os.path.exists(path):
            log("generating", path)
            tmp = path + ".part"
            with open(tmp, "wb") as f:
                fn(f, random.Random(0x11F))
            os.replace(tmp, path)
        out[name] = path
    return out


def _ascii_line(rng):
    n = rng.randint(80, 120)
    return bytes(rng.randint(0x21, 0x7E) if rng.random() > 0.15 else 0x20 for _ in range(n)) + b"\n"


def gen_ascii(f, rng):
    pool = [_ascii_line(rng) for _ in range(8192)]
    total = 0
    while total < 100 * MB:
        line = pool[rng.randrange(len(pool))]
        if total + len(line) > 100 * MB:
            line = b"x" * (100 * MB - total - 1) + b"\n"
        f.write(line)
        total += len(line)


WORDS = ["alpha", "beta", "çalışma", "über", "naïve", "für", "ışık", "→", "█▓▒░", "λx.x",
         "日本語", "漢字", "emoji", "≠", "±0.5", "error:", "warning", "ok", "ğüşöç", "“quoted”"]


def _sgr_line(rng, maxw=120):
    parts, width = [], 0
    while True:
        w = rng.choice(WORDS)
        cw = sum(2 if ord(ch) >= 0x2E80 else 1 for ch in w) + 1
        if width + cw > maxw:
            break
        width += cw
        style = rng.random()
        if style < 0.4:
            parts.append(f"\x1b[38;5;{rng.randrange(256)}m{w}\x1b[0m ")
        elif style < 0.6:
            parts.append(f"\x1b[1;3{rng.randrange(8)};4{rng.randrange(8)}m{w}\x1b[0m ")
        elif style < 0.75:
            parts.append(f"\x1b[38;2;{rng.randrange(256)};{rng.randrange(256)};{rng.randrange(256)}m{w}\x1b[0m ")
        else:
            parts.append(w + " ")
        if width > 80 and rng.random() < 0.3:
            break
    return ("".join(parts).rstrip() + "\n").encode()


def gen_sgr(f, rng):
    pool = [_sgr_line(rng) for _ in range(8192)]
    total = 0
    while total < 100 * MB:
        line = pool[rng.randrange(len(pool))]
        if total + len(line) > 100 * MB:
            # Pad with ASCII so the file is exactly 100 MB and never ends
            # inside an escape sequence or a UTF-8 character.
            line = b"x" * (100 * MB - total - 1) + b"\n"
        f.write(line)
        total += len(line)


def gen_longline(f, rng):
    chunk = bytes(rng.randint(0x21, 0x7E) for _ in range(1_000_000))
    for _ in range(20):
        f.write(chunk)
    # No trailing newline: one 20 MB logical line.


def gen_history(f, rng):
    pool = [_sgr_line(rng, 100) for _ in range(4096)]
    for i in range(100_000):
        f.write(b"%06d " % i + pool[rng.randrange(len(pool))])


# --------------------------------------------------------------------------
# Adapters: how to run each tool in a private namespace.


def private_env(sock_dir):
    home = os.path.join(TMP, "home")
    return {
        "PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": home,
        "USER": os.environ.get("USER", "bench"),
        "LOGNAME": os.environ.get("USER", "bench"),
        "TMPDIR": sock_dir + "/",
        "TMUX_TMPDIR": sock_dir,
        "XDG_CONFIG_HOME": os.path.join(home, ".config"),
        "XDG_DATA_HOME": os.path.join(home, ".local", "share"),
        "XDG_CACHE_HOME": os.path.join(home, ".cache"),
        "XDG_RUNTIME_DIR": sock_dir,
        "SHELL": "/bin/sh",
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8",
        "PS1": "$ ",
    }


def kill_tracked(pids, matcher):
    """SIGTERM then SIGKILL pids we started, re-checking each is still ours
    (its command line names our private socket dir) to survive pid reuse."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        table = ps_table()
        live = [p for p in pids if p in table and matcher(table[p][3])]
        if not live:
            return
        log(f"sending {sig.name} to leftover pids {live}")
        for p in live:
            try:
                os.kill(p, sig)
            except ProcessLookupError:
                pass
        time.sleep(0.5)


def wait_gone(pids, timeout=5.0):
    t = time.time()
    while time.time() - t < timeout:
        if not any(pid_alive(p) for p in pids):
            return True
        time.sleep(0.02)
    return False


class Adapter:
    """One tool under test. Each session lives in its own work dir, whose
    s/ subdirectory is the socket dir (TMPDIR, TMUX_TMPDIR, ...)."""

    name = "?"
    has_server = True
    detach_keys = b""

    def __init__(self, work, opts):
        self.work = work
        self.opts = opts  # {"history": int, "escape_time": int}
        # Unix socket paths are limited to 104 bytes on macOS and zellij adds
        # ~45 under TMPDIR, so socket dirs live in a short separate tree.
        self.sock_dir = os.path.join(TMP, "s", os.path.basename(work))
        if len(self.sock_dir) + 45 > 104:
            raise RuntimeError(f"socket dir path too long for zellij: {self.sock_dir}")
        shutil.rmtree(self.sock_dir, ignore_errors=True)
        os.makedirs(self.sock_dir)
        self.pids = []  # every server pid seen; only these are ever killed

    def env(self):
        return private_env(self.sock_dir)

    @classmethod
    def binary_name(cls):
        raise NotImplementedError

    @classmethod
    def available(cls):
        return shutil.which(cls.binary_name()) is not None

    @classmethod
    def binary(cls):
        p = shutil.which(cls.binary_name())
        return os.path.realpath(p) if p else None

    @classmethod
    def version(cls):
        return "?"

    def start(self):
        """Create the session detached (no-op if the client creates it)."""

    def client_argv(self):
        """argv of the client, run on the driver PTY."""
        raise NotImplementedError

    def server_pids(self):
        raise NotImplementedError

    def split(self, cmd=None):
        """Add a pane running cmd (else a shell), leaving focus on the first."""
        raise NotImplementedError

    def pane_count(self):
        raise NotImplementedError

    def stop(self):
        raise NotImplementedError

    def config_text(self):
        return ""

    def history_lines(self):
        """Scrollback lines currently retained above the viewport, or None."""
        return None

    def window_count(self):
        """Windows (tabs) in the session, or None."""
        return None

    def hf_create_kill(self):
        """(prepare, command, cleanup) shell strings for hyperfine, or None."""
        return None


class NoneAdapter(Adapter):
    """No multiplexer: a login /bin/sh directly on the driver PTY."""

    name = "none"
    has_server = False

    @classmethod
    def binary_name(cls):
        return "/bin/sh"

    @classmethod
    def available(cls):
        return True

    @classmethod
    def version(cls):
        return "n/a (/bin/sh on the driver PTY)"

    def client_argv(self):
        return ["/bin/sh", "-l"]

    def server_pids(self):
        return []

    def pane_count(self):
        return 1

    def stop(self):
        pass


class TmuxAdapter(Adapter):
    name = "tmux"
    detach_keys = b"\x02d"  # C-b d
    session = "main"

    @classmethod
    def binary_name(cls):
        return "tmux"

    @classmethod
    def version(cls):
        return sh(["tmux", "-V"]).stdout.strip()

    def conf(self):
        lines = []
        if self.opts.get("history"):
            lines.append(f"set -g history-limit {self.opts['history']}")
        if self.opts.get("escape_time") is not None:
            lines.append(f"set -sg escape-time {self.opts['escape_time']}")
        if not lines:
            return "/dev/null"
        path = os.path.join(self.work, "tmux.conf")
        with open(path, "w") as f:
            f.write("\n".join(lines) + "\n")
        return path

    def config_text(self):
        c = self.conf()
        return "-f /dev/null (built-in defaults)" if c == "/dev/null" else open(c).read().strip()

    def base(self):
        return ["tmux", "-L", "lifbench", "-f", self.conf()]

    def start(self):
        sh(self.base() + ["new-session", "-d", "-s", self.session, "-x", str(COLS), "-y", str(ROWS)], env=self.env())
        self.pids = self.server_pids()

    def client_argv(self):
        return self.base() + ["attach", "-t", self.session]

    def server_pids(self):
        r = sh(self.base() + ["display-message", "-p", "-t", self.session, "#{pid}"], env=self.env(), check=False)
        try:
            return [int(r.stdout.strip())]
        except ValueError:
            return []

    def split(self, cmd=None):
        argv = self.base() + ["split-window", "-d", "-t", self.session]
        if cmd:
            argv.append(cmd)
        r = sh(argv, env=self.env(), check=False)
        # Re-tile after every split so that 50 panes fit in 200x50.
        sh(self.base() + ["select-layout", "-t", self.session, "tiled"], env=self.env(), check=False)
        return r.returncode == 0

    def pane_count(self):
        r = sh(self.base() + ["list-panes", "-s", "-t", self.session], env=self.env(), check=False)
        return len(r.stdout.splitlines())

    def window_count(self):
        r = sh(self.base() + ["list-windows", "-t", self.session], env=self.env(), check=False)
        return len(r.stdout.splitlines()) if r.returncode == 0 else None

    def history_lines(self):
        r = sh(self.base() + ["list-panes", "-t", self.session, "-F", "#{history_size}"], env=self.env(), check=False)
        try:
            return int(r.stdout.split()[0])
        except (ValueError, IndexError):
            return None

    def show_options(self):
        r = sh(self.base() + ["show-options", "-s", "escape-time", ";", "show-options", "-g", "history-limit",
                              ";", "show-options", "-g", "status-interval"], env=self.env(), check=False)
        return r.stdout.strip()

    def stop(self):
        sh(["tmux", "-L", "lifbench", "kill-server"], env=self.env(), check=False)
        if not wait_gone(self.pids):
            kill_tracked(self.pids, lambda c: "lifbench" in c and "tmux" in c)

    def hf_create_kill(self):
        b = "tmux -L lifbench-hf -f /dev/null"
        # kill-server returns before the server has exited; the (untimed)
        # prepare step waits for it so runs do not overlap.
        return (f"for i in $(seq 500); do {b} has-session 2>/dev/null || break; sleep 0.01; done",
                f"{b} new-session -d -s s -x {COLS} -y {ROWS} && {b} kill-server",
                f"{b} kill-server 2>/dev/null; true")


ZELLIJ_FLAGS = ["DesktopAppIntroduced", "WelcomeCardShown", "MissingServicesShown", "WizardDismissed",
                "ExitLineShown", "RemindersOff", "SetupApproved", "SetupNeverShow"]


def zellij_seed_home():
    """Mark zellij's first-run onboarding as already dismissed in the private
    HOME; otherwise a floating "Welcome to Zellij" plugin takes input focus."""
    d = os.path.join(TMP, "home", "Library", "Application Support", "org.Zellij-Contributors.Zellij")
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, "desktop_integration.json")
    doc = {}
    if os.path.exists(p):
        try:
            doc = json.load(open(p))
        except ValueError:
            doc = {}
    doc.setdefault("launcher", None)
    doc.setdefault("icons", [])
    doc.setdefault("binary_copy", None)
    doc.setdefault("windows", {"start_menu": None, "explorer_item": False, "jump_list": False, "added_to_path": None})
    doc.setdefault("repair", {"declined": [], "never": False})
    doc.setdefault("missing_services", [])
    doc["flags"] = ZELLIJ_FLAGS
    with open(p, "w") as f:
        json.dump(doc, f, indent=2)


def zellij_config(opts):
    t = "show_startup_tips false\nshow_release_notes false\n"
    if opts.get("history"):
        t += f"scroll_buffer_size {opts['history']}\n"
    return t


class ZellijAdapter(Adapter):
    name = "zellij"
    detach_keys = b"\x0fd"  # Ctrl-o (session mode), d (detach)
    session = "lifbench-main"

    @classmethod
    def binary_name(cls):
        return "zellij"

    @classmethod
    def version(cls):
        return sh(["zellij", "--version"]).stdout.strip()

    def __init__(self, work, opts):
        super().__init__(work, opts)
        self.floating = 0
        self.cfg_dir = os.path.join(work, "zcfg")
        os.makedirs(self.cfg_dir, exist_ok=True)
        with open(os.path.join(self.cfg_dir, "config.kdl"), "w") as f:
            f.write(self.config_text())
        zellij_seed_home()

    def config_text(self):
        return zellij_config(self.opts)

    def env(self):
        e = super().env()
        e["ZELLIJ_CONFIG_DIR"] = self.cfg_dir
        return e

    def z(self, *args):
        return sh(["zellij", "-s", self.session] + list(args), env=self.env(), check=False)

    def client_argv(self):
        # The attaching client creates the session at its own size (200x50),
        # which is how zellij is normally started.
        return ["zellij", "attach", "-c", self.session]

    def server_pids(self):
        pids = [p for p, (_, _, _, cmd) in ps_table().items()
                if "zellij" in cmd and "--server" in cmd and self.sock_dir in cmd]
        self.pids = sorted(set(self.pids + pids))
        return pids

    def split(self, cmd=None):
        if cmd:
            r = self.z("run", "--", "/bin/sh", "-c", cmd)
        else:
            r = self.z("action", "new-pane")
            if r.returncode != 0:
                # zellij refuses a 45th tiled pane at 200x50 ("No room for a
                # new pane", also with --stacked); floating panes have no
                # size limit and are full terminal panes all the same.
                r = self.z("action", "new-pane", "--floating")
                self.floating += r.returncode == 0
        # New panes take focus in zellij; give it back to the first pane.
        self.z("action", "focus-pane-id", "terminal_0")
        return r.returncode == 0

    def window_count(self):
        r = self.z("action", "query-tab-names")
        return len(r.stdout.splitlines()) if r.returncode == 0 else None

    def history_lines(self):
        """Lines above the viewport, from a full dump of the focused pane."""
        path = os.path.join(self.work, "dump.txt")
        r = self.z("action", "dump-screen", "--full", "--path", path)
        if r.returncode != 0 or not os.path.exists(path):
            return None
        with open(path, "rb") as f:
            n = f.read().count(b"\n")
        return max(0, n - ROWS)

    def pane_count(self):
        r = self.z("action", "list-panes")
        return sum(1 for l in r.stdout.splitlines() if l.startswith("terminal_"))

    def stop(self):
        self.server_pids()
        sh(["zellij", "kill-session", self.session], env=self.env(), check=False)
        if not wait_gone(self.pids):
            kill_tracked(self.pids, lambda c: self.sock_dir in c)
        sh(["zellij", "delete-session", "--force", self.session], env=self.env(), check=False)

    def hf_create_kill(self):
        # kill-session returns before the server has exited; the (untimed)
        # prepare step waits for that and removes the resurrection entry.
        return ("zellij delete-session --force lifbench-hf >/dev/null 2>&1;"
                " for i in $(seq 500); do zellij ls -n -s 2>/dev/null | grep -qx lifbench-hf || break; sleep 0.01;"
                " zellij delete-session --force lifbench-hf >/dev/null 2>&1; done; true",
                "zellij attach -b lifbench-hf && zellij kill-session lifbench-hf",
                "zellij kill-session lifbench-hf >/dev/null 2>&1; sleep 0.5;"
                " zellij delete-session --force lifbench-hf >/dev/null 2>&1; true")


class LifAdapter(Adapter):
    """lif (target/release/lif). Skipped while the release binary does not
    exist. lif 0.1.0 does not read a config file yet (lif-config is not linked
    into the binary), so options are set with set-option after the server
    starts; they only apply to panes created afterwards."""

    name = "lif"
    detach_keys = b"\x02d"  # C-b d
    session = "main"

    @classmethod
    def binary_name(cls):
        return os.path.join(ROOT, "target", "release", "lif")

    @classmethod
    def available(cls):
        return os.access(cls.binary_name(), os.X_OK)

    @classmethod
    def binary(cls):
        return cls.binary_name() if cls.available() else None

    @classmethod
    def version(cls):
        r = sh([cls.binary_name(), "-V"], check=False)
        return (r.stdout or r.stderr).strip() or "unknown"

    def sock(self):
        return os.path.join(self.sock_dir, "lif.sock")

    def base(self):
        return [self.binary_name(), "-S", self.sock()]

    def lif(self, *args, check=False):
        return sh(self.base() + list(args), env=self.env(), check=check)

    def config_text(self):
        if self.opts.get("escape_time") is not None:
            return f"set-option -s escape-time {self.opts['escape_time']}"
        if self.opts.get("kitty"):
            return "built-in defaults; the driver's terminal answers the kitty keyboard query"
        if self.opts.get("history"):
            # Scrollback is a byte budget. Rows are stored at full width, so
            # 100k lines at 200 columns need ~170 MB; 1 GiB keeps them all.
            return (f"set-option -g scrollback-bytes {LIF_HISTORY_BYTES}; "
                    f"set-option -g history-limit {self.opts['history']} (then a fresh window)")
        return "built-in defaults"

    def start(self):
        self.lif("new-session", "-d", "-s", self.session, "-x", str(COLS), "-y", str(ROWS), check=True)
        if self.opts.get("history"):
            self.lif("set-option", "-g", "scrollback-bytes", str(LIF_HISTORY_BYTES), check=True)
            self.lif("set-option", "-g", "history-limit", str(self.opts["history"]), check=True)
            # The first pane was created with the default budget; replace it.
            self.lif("new-window", "-t", self.session, check=True)
            self.lif("kill-window", "-t", f"{self.session}:0", check=True)
        if self.opts.get("escape_time") is not None:
            self.lif("set-option", "-s", "escape-time", str(self.opts["escape_time"]), check=True)
        self.server_pids()

    def client_argv(self):
        return self.base() + ["attach", "-t", self.session]

    def server_pids(self):
        # The server is a daemon forked from the first client: its command
        # line is that client's (it names our private socket) and its parent
        # is launchd. Attached clients and one-shot commands have other parents.
        pids = [p for p, (ppid, _, _, cmd) in ps_table().items() if self.sock() in cmd and ppid == 1]
        self.pids = sorted(set(self.pids + pids))
        return pids

    def split(self, cmd=None):
        argv = ["split-window", "-d", "-t", self.session]
        if cmd:
            argv.append(cmd)
        r = self.lif(*argv)
        self.lif("select-layout", "-t", self.session, "tiled")
        return r.returncode == 0

    def pane_count(self):
        r = self.lif("list-panes", "-s", "-t", self.session)
        return len(r.stdout.splitlines()) if r.returncode == 0 else -1

    def window_count(self):
        r = self.lif("list-windows", "-t", self.session)
        return len(r.stdout.splitlines()) if r.returncode == 0 else None

    def history_lines(self):
        r = self.lif("list-panes", "-t", self.session, "-F", "#{history_size}")
        try:
            return int(r.stdout.split()[0])
        except (ValueError, IndexError):
            return None

    def pane_pids(self):
        r = self.lif("list-panes", "-a", "-F", "#{pane_pid}")
        return [int(x) for x in r.stdout.split() if x.isdigit() and int(x) > 1] if r.returncode == 0 else []

    def stop(self):
        self.server_pids()
        panes = self.pane_pids()
        self.lif("kill-server")
        if not wait_gone(self.pids):
            kill_tracked(self.pids, lambda c: self.sock() in c)
        # A pane shell left behind would run (and may spin) through every
        # later measurement.
        if not wait_gone(panes, 2.0):
            left = [p for p in panes if pid_alive(p)]
            log(f"WARNING: pane processes outlived the lif server, killing {left}")
            for p in left:
                try:
                    os.killpg(p, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass

    def hf_create_kill(self):
        b = f"{self.binary_name()} -S {self.sock_dir}/hf.sock"
        return (f"for i in $(seq 500); do {b} has-session -t s 2>/dev/null || break; sleep 0.01; done",
                f"{b} new-session -d -s s -x {COLS} -y {ROWS} && {b} kill-server",
                f"{b} kill-server 2>/dev/null; true")


LIF_HISTORY_BYTES = 1 << 30


class LifBaseAdapter(LifAdapter):
    """Another lif build (default: bench/tmp/ab/lif-head, a copy of an older
    build, or $LIFBENCH_LIF_BASE), for before/after comparisons in one run."""

    name = "lifbase"

    @classmethod
    def binary_name(cls):
        return os.environ.get("LIFBENCH_LIF_BASE") or os.path.join(ROOT, "bench", "tmp", "ab", "lif-head")


ADAPTERS = {"none": NoneAdapter, "tmux": TmuxAdapter, "zellij": ZellijAdapter, "lif": LifAdapter,
            "lifbase": LifBaseAdapter}


# --------------------------------------------------------------------------
# A running session: adapter + driver holding an attached client.


class Driver:
    def __init__(self, argv, env, flags=()):
        self.p = subprocess.Popen([DRIVER, "hold", "--cols", str(COLS), "--rows", str(ROWS), *flags, "--"] + argv,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True, bufsize=1)
        hello = json.loads(self.p.stdout.readline())
        self.client_pid = hello["pid"]

    def cmd(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        out = self.p.stdout.readline()
        if not out:
            raise RuntimeError(f"driver died during {line.split()[0]}")
        return json.loads(out)

    def type(self, text):
        return self.cmd("type " + text.encode().hex())

    def close(self):
        try:
            self.cmd("quit 2000")
        except (RuntimeError, BrokenPipeError, ValueError):
            pass
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()


_seq = [0]


class Session:
    def __init__(self, tool, opts=None, ready_retry_ms=250):
        _seq[0] += 1
        self.work = os.path.join(RUNS, f"{tool[0]}{_seq[0]}")
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)
        self.ad = ADAPTERS[tool](self.work, opts or {})
        self.driver = None
        self.ready_retry_ms = ready_retry_ms
        self.ready_info = None

    def marker(self, name):
        return os.path.join(self.work, name)

    def __enter__(self):
        try:
            t0 = time.perf_counter()
            self.ad.start()
            # --kitty: the fake terminal speaks the kitty keyboard protocol.
            flags = ["--kitty"] if self.ad.opts.get("kitty") else []
            self.driver = Driver(self.ad.client_argv(), self.ad.env(), flags)
            r = self.driver.cmd(f"ready {self.marker('ready')} 30000 {self.ready_retry_ms}")
            if not r.get("ok"):
                raise RuntimeError(f"{self.ad.name}: session never became ready: {r}")
            self.ready_info = dict(r, since_start_ms=(time.perf_counter() - t0) * 1e3)
            self.ad.server_pids()
            self.driver.cmd("quiet 300 10000")
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *exc):
        if self.driver:
            self.driver.close()
        self.ad.stop()
        shutil.rmtree(self.work, ignore_errors=True)
        shutil.rmtree(self.ad.sock_dir, ignore_errors=True)
        return False

    def snapshot(self):
        """Memory and CPU counters of the server processes and the client."""
        spids = self.ad.server_pids()
        table = ps_table()
        su = rusage(spids)
        cu = rusage([self.driver.client_pid])
        cpid = self.driver.client_pid
        return {
            "server_pids": spids,
            "server": su,
            "server_rss": sum(u.get("rss", 0) for u in su),
            "server_footprint": sum(u.get("footprint", 0) for u in su),
            "server_ps_rss": sum(table[p][1] * 1024 for p in spids if p in table),
            "server_ps_cputime": sum(cputime_s(table[p][2]) for p in spids if p in table),
            "client": cu[0] if cu else None,
            "client_ps_rss": table[cpid][1] * 1024 if cpid in table else None,
            "client_ps_cputime": cputime_s(table[cpid][2]) if cpid in table else None,
        }


def settle(s, secs=2.0):
    s.driver.cmd("quiet 1000 20000")
    time.sleep(secs)


def typed(cmd):
    return (cmd + "\r").encode().hex()


# --------------------------------------------------------------------------
# Measurements


def m_throughput(tools, files, reps):
    """Repetitions are interleaved across tools (rep 1 of every tool, then
    rep 2, ...) so that drift in machine load hits all tools alike."""
    sizes = {k: os.path.getsize(v) for k, v in files.items()}
    out = {t: {} for t in tools}
    for kind in ("ascii", "sgr", "longline"):
        path = files[kind]
        runs = {t: [] for t in tools}
        timeout_ms = 300_000 if kind == "longline" else 600_000
        for i in range(reps):
            for tool in tools:
                if runs[tool] and not runs[tool][-1]["ok"]:
                    continue  # one timeout says enough; don't spend another 5 minutes
                runs[tool].append(throughput_once(tool, path, kind, sizes[kind], timeout_ms, i, out[tool]))
        for tool in tools:
            ok = [r for r in runs[tool] if r["ok"]]
            out[tool][kind] = {
                "file_bytes": sizes[kind],
                "runs": runs[tool],
                "secs": stats([r["secs"] for r in ok]),
                "mb_s": stats([r["mb_s"] for r in ok]),
                "client_bytes": stats([r["client_bytes"] for r in ok]),
                "drain_secs": stats([r["drain_secs"] for r in ok]),
                "failed": len(runs[tool]) - len(ok),
            }
    return out


def throughput_once(tool, path, kind, size, timeout_ms, i, out):
    with Session(tool) as s:
        mk = s.marker("done")
        r = s.driver.cmd(f"run {mk} {timeout_ms} 250 {typed(f'cat {path}; touch {mk}')}")
        rec = {"ok": r.get("ok", False), "loadavg": os.getloadavg()[0]}
        if r.get("ok"):
            rec.update(secs=r["secs"], mb_s=size / MB / r["secs"], drain_secs=r["drain_secs"],
                       client_bytes=r["bytes"], client_bytes_at_done=r["bytes_at_done"])
        else:
            rec.update(error=r.get("error"), secs=r.get("secs"), client_bytes=r.get("bytes"))
        if kind == "ascii" and i == 0 and s.ad.has_server and rec["ok"]:
            settle(s, 1.0)
            out["memory_after_ascii_cat"] = dict(s.snapshot(), retained_lines=s.ad.history_lines())
    log(f"{tool} throughput {kind} #{i}: " + ", ".join(
        f"{k}={v:.4g}" if isinstance(v, float) else f"{k}={v}" for k, v in rec.items()))
    return rec


def m_memory(tool, files, quick):
    if tool == "none":
        return {"skipped": "no server process"}
    out = {}
    with Session(tool) as s:
        settle(s)
        out["idle_1_pane"] = dict(s.snapshot(), panes=s.ad.pane_count())
    for n in ((10,) if quick else (10, 50)):
        with Session(tool) as s:
            failed = sum(0 if s.ad.split() else 1 for _ in range(n - 1))
            settle(s, 3.0)
            panes = s.ad.pane_count()
            out[f"idle_{n}_panes"] = dict(s.snapshot(), panes=panes, split_failures=failed,
                                           floating=getattr(s.ad, "floating", 0), windows=s.ad.window_count())
            log(f"{tool} memory: asked for {n} panes, have {panes}")
    with Session(tool, {"history": 200_000}) as s:
        mk = s.marker("done")
        r = s.driver.cmd(f"run {mk} 600000 500 {typed('cat ' + files['history'] + '; touch ' + mk)}")
        settle(s)
        out["history_100k_lines"] = dict(s.snapshot(), run=r, config=s.ad.config_text(),
                                         retained_lines=s.ad.history_lines())
    return out


def m_idle(tool, secs):
    with Session(tool) as s:
        settle(s, 3.0)
        a = s.snapshot()
        time.sleep(secs)
        b = s.snapshot()
        alive = s.driver.cmd("stats")

    def d(xs, ys, f):
        return sum(y.get(f, 0) for y in ys) - sum(x.get(f, 0) for x in xs)

    res = {"seconds": secs, "client_alive": alive.get("alive")}
    if a["server"]:
        res["server"] = {
            "cpu_ms": (d(a["server"], b["server"], "user_ns") + d(a["server"], b["server"], "sys_ns")) / 1e6,
            "idle_wakeups": d(a["server"], b["server"], "idle_wakeups"),
            "intr_wakeups": d(a["server"], b["server"], "intr_wakeups"),
            "ps_cputime_delta_s": round(b["server_ps_cputime"] - a["server_ps_cputime"], 2),
        }
    if a["client"] and b["client"]:
        res["client"] = {
            "cpu_ms": (d([a["client"]], [b["client"]], "user_ns") + d([a["client"]], [b["client"]], "sys_ns")) / 1e6,
            "idle_wakeups": d([a["client"]], [b["client"]], "idle_wakeups"),
            "intr_wakeups": d([a["client"]], [b["client"]], "intr_wakeups"),
            "ps_cputime_delta_s": round((b["client_ps_cputime"] or 0) - (a["client_ps_cputime"] or 0), 2),
        }
    return res


def hyperfine(cmd, env, runs, name):
    path = os.path.join(TMP, f"hf-{name}.json")
    prep, command, cleanup = cmd
    # --shell=default: commands use && (hyperfine 2 refuses shell syntax
    # otherwise); hyperfine subtracts its measured shell spawn time.
    argv = ["hyperfine", "--shell=default", "--export-json", path, "--runs", str(runs), "--warmup", "0",
            "--style", "none"]
    if prep:
        argv += ["--prepare", prep]
    if cleanup:
        argv += ["--cleanup", cleanup]
    argv.append(command)
    r = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=1800)
    if r.returncode != 0:
        return {"error": (r.stderr or r.stdout).strip()[-2000:], "command": command}
    res = json.load(open(path))["results"][0]
    if "times" in res:  # hyperfine 1.x
        times = res["times"]
    else:  # hyperfine 2.x
        times = [m["time_wall_clock"]["value"] for m in res["measurements"]]
        if any(m.get("exit_code", 0) != 0 for m in res["measurements"]):
            return {"error": "non-zero exit code in a run", "command": command}
    # No warmup on purpose: the first sample is the cold-ish one; the rest
    # are warm.
    return {"command": command, "first_s": times[0], "warm": stats(times[1:]), "times_s": times}


def m_startup(tool, quick):
    out = {}
    runs = 10 if quick else 31
    hf_dir = os.path.join(RUNS, f"h{tool[0]}")
    shutil.rmtree(hf_dir, ignore_errors=True)
    os.makedirs(hf_dir)
    ad = ADAPTERS[tool](hf_dir, {})
    ck = ad.hf_create_kill()
    if ck:
        out["create_kill"] = hyperfine(ck, ad.env(), runs, f"{tool}-ck")
    shutil.rmtree(hf_dir, ignore_errors=True)
    shutil.rmtree(ad.sock_dir, ignore_errors=True)
    # Time from "create session" until the pane's shell runs a typed command,
    # with a client attached: what a user waits for. Measured with the
    # driver, since completion is a file appearing, not a process exiting.
    usable = []
    for _ in range(5 if quick else 15):
        with Session(tool, ready_retry_ms=50) as s:
            usable.append(s.ready_info["since_start_ms"])
    out["new_session_to_command_ms"] = dict(stats(usable), samples=usable)
    if ADAPTERS[tool].detach_keys:
        with Session(tool) as s:
            cmd = " ".join([DRIVER, "attach-detach", "--keys", s.ad.detach_keys.hex(), "--quiet-ms",
                            str(ATTACH_QUIET_MS), "--timeout-ms", "10000", "--"]
                           + [quote(a) for a in s.ad.client_argv()])
            out["attach_detach"] = hyperfine((None, cmd, None), s.ad.env(), runs, f"{tool}-ad")
            # The session must have survived every detach.
            out["attach_detach"]["session_alive_after"] = bool(s.ad.server_pids())
    return out


def quote(a):
    return "'" + a.replace("'", "'\\''") + "'"


def lat_summary(r):
    s = stats(r.get("samples_us", []))
    return {"n": s.get("n", 0), "timeouts": r.get("timeouts"), "stray_hits": r.get("stray"),
            "client_bytes": r.get("bytes"), "error": r.get("error"), "samples_us": r.get("samples_us", []),
            "us": {k: s[k] for k in ("median", "p95", "p99", "max", "min", "p25", "p75")} if s.get("n") else None}


def echo_block(tool, n, flood):
    """One fresh session, pane running cat, optionally a second pane running
    yes; 20 discarded warm-up samples, then n samples."""
    with Session(tool) as s:
        s.driver.type("exec cat\r")
        s.driver.cmd("quiet 500 10000")
        ok = True
        if flood:
            ok = s.ad.split("exec yes")
            time.sleep(1.0)
        s.driver.cmd(f"latency echo 20 {ECHO_CHARS.hex()} 10 2000")
        r = s.driver.cmd(f"latency echo {n} {ECHO_CHARS.hex()} 10 2000")
        r["split_ok"] = ok
        r["loadavg"] = os.getloadavg()[0]
        r["sys_cpu_pct"] = system_cpu_pct()
        return r


def system_cpu_pct():
    """Sum of %CPU over all processes (ps' decaying average): a rough
    indicator of background activity, which moves keystroke latency."""
    out = sh(["ps", "-A", "-o", "%cpu="], check=False).stdout
    return round(sum(float(x) for x in out.split() if x.replace(".", "", 1).isdigit()), 1)


def m_latency(tools, n, rounds):
    """Echo latency in rounds: each round runs a block of n/rounds samples
    per tool (fresh session each), so all tools see the same machine state.
    Apple Silicon power states move single-keystroke latency by several x
    between runs; interleaving keeps the comparison fair within a run."""
    out = {t: {} for t in tools}
    per = max(1, n // rounds)
    for key, flood in (("echo_idle", False), ("echo_flood", True)):
        blocks = {t: [] for t in tools}
        for rnd in range(rounds):
            for t in tools:
                if flood and t == "none":
                    continue
                r = echo_block(t, per, flood)
                blocks[t].append(r)
                med = statistics.median(r["samples_us"]) if r.get("samples_us") else None
                log(f"{t} {key} round {rnd}: median {med} us, timeouts {r.get('timeouts')}, "
                    f"system CPU {r['sys_cpu_pct']}%")
        for t in tools:
            if not blocks[t]:
                continue
            merged = {
                "samples_us": [x for b in blocks[t] for x in b.get("samples_us", [])],
                "timeouts": sum(b.get("timeouts", 0) for b in blocks[t]),
                "stray": sum(b.get("stray", 0) for b in blocks[t]),
                "bytes": sum(b.get("bytes", 0) for b in blocks[t]),
            }
            out[t][key] = lat_summary(merged)
            out[t][key]["round_medians_us"] = [statistics.median(b["samples_us"]) if b.get("samples_us") else None
                                               for b in blocks[t]]
            out[t][key]["split_ok"] = all(b.get("split_ok", True) for b in blocks[t])
            out[t][key]["round_loadavg"] = [b.get("loadavg") for b in blocks[t]]
            out[t][key]["round_sys_cpu_pct"] = [b.get("sys_cpu_pct") for b in blocks[t]]
    for t in tools:
        out[t].update(m_esc(t, max(50, n // 3)))
    return out


def m_esc(tool, n):
    out = {}
    variants = [("esc_default", {})]
    if tool in ("tmux", "lif", "lifbase"):
        variants.append(("esc_time_0", {"escape_time": 0}))
    if tool in ("lif", "lifbase"):
        # A terminal with the kitty keyboard protocol: lif turns it on and
        # Escape arrives unambiguously (tmux 3.8 and zellij 0.46 do not
        # push kitty flags, so for them this is the legacy case again).
        variants.append(("esc_kitty", {"kitty": True}))
    for label, opts in variants:
        with Session(tool, opts) as s:
            mk = s.marker("probe")
            r = s.driver.cmd(f"run {mk} 10000 300 {typed('exec ' + DRIVER + ' escprobe ' + mk)}")
            if not r.get("ok"):
                out[label] = {"error": f"probe did not start: {r}"}
                continue
            s.driver.cmd("quiet 300 10000")
            out[label] = lat_summary(s.driver.cmd(f"latency esc {n} {ESC_MARK.hex()} 15 2000"))
            out[label]["config"] = s.ad.config_text()
            log(f"{tool} {label}: median {out[label]['us'] and out[label]['us']['median']} us")
    return out


def m_binary(tool):
    b = ADAPTERS[tool].binary()
    if tool == "none" or not b:
        return {"skipped": True}
    libs = [l.strip().split(" (")[0] for l in sh(["otool", "-L", b]).stdout.splitlines()[1:]]
    return {"path": b, "bytes": os.path.getsize(b), "dylibs": libs,
            "non_system_dylibs": [l for l in libs if not l.startswith(("/usr/lib/", "/System/"))]}


def machine_info():
    def sysctl(k):
        return sh(["sysctl", "-n", k], check=False).stdout.strip()

    sw = sh(["sw_vers"], check=False).stdout
    return {
        "model": sysctl("hw.model"),
        "chip": sysctl("machdep.cpu.brand_string"),
        "cores": sysctl("hw.ncpu"),
        "perflevels": [f"{sysctl(f'hw.perflevel{i}.physicalcpu')} {sysctl(f'hw.perflevel{i}.name')}"
                       for i in range(int(sysctl("hw.nperflevels") or 0))],
        "mem_bytes": int(sysctl("hw.memsize") or 0),
        "macos": " ".join(l.split(":", 1)[1].strip() for l in sw.splitlines() if ":" in l),
        "kernel": platform.release(),
        "loadavg_at_start": os.getloadavg(),
        "power": sh(["pmset", "-g", "batt"], check=False).stdout.strip().splitlines()[:1],
    }


METRICS = ["binary", "throughput", "memory", "idle", "startup", "latency"]


def tool_header(tool, args):
    cls = ADAPTERS[tool]
    res = {"tool": tool, "version": cls.version(), "date": datetime.datetime.now().isoformat(timespec="seconds"),
           "machine": machine_info(), "driver": DRIVER, "size": f"{COLS}x{ROWS}", "quick": args.quick,
           "reps": 2 if args.quick else args.reps, "metrics": {}, "elapsed_s": {}, "loadavg": {}}
    if tool == "tmux":
        with Session("tmux") as s:
            res["config"] = s.ad.config_text()
            res["effective_options"] = s.ad.show_options()
    elif tool == "zellij":
        res["config"] = zellij_config({}) + "(first-run onboarding flags pre-seeded as dismissed)"
    return res


def run_metric(m, tool, args, files):
    if m == "binary":
        return m_binary(tool)
    if m == "memory":
        return m_memory(tool, files, args.quick)
    if m == "idle":
        return m_idle(tool, 5 if args.quick else args.idle_secs)
    if m == "startup":
        return m_startup(tool, args.quick)
    raise ValueError(m)


def run_all(tools, args, files):
    """Metric-major order: every tool runs metric A before any runs metric B,
    so tools are compared under the same machine conditions."""
    docs = {t: tool_header(t, args) for t in tools}
    only = args.only.split(",") if args.only else METRICS
    for m in METRICS:
        if m not in only:
            continue
        if m in ("throughput", "latency"):
            t0, load0 = time.time(), os.getloadavg()[0]
            log(f"== {m} ({', '.join(tools)}, interleaved)")
            try:
                if m == "throughput":
                    per = m_throughput(tools, files, docs[tools[0]]["reps"])
                else:
                    # Keystroke latency depends on the CPU power state, which
                    # stays raised for tens of seconds after the heavy
                    # metrics; let the machine settle into its idle state, the
                    # state a user typing at a prompt is in.
                    rest = 2 if args.quick else args.latency_rest
                    log(f"resting {rest} s before latency")
                    time.sleep(rest)
                    per = m_latency(tools, 100 if args.quick else args.samples, 2 if args.quick else args.rounds)
            except Exception as e:
                per = {t: {"error": repr(e), "trace": traceback.format_exc()} for t in tools}
                log(f"{m} FAILED: {e!r}")
            for t in tools:
                docs[t]["metrics"][m] = per[t]
                docs[t]["elapsed_s"][m] = round(time.time() - t0, 1)
                docs[t]["loadavg"][m] = [load0, os.getloadavg()[0]]
            continue
        for t in tools:
            t1, l1 = time.time(), os.getloadavg()[0]
            log(f"== {t}: {m}")
            try:
                v = run_metric(m, t, args, files)
            except Exception as e:  # record it and go on with the other metrics
                v = {"error": repr(e), "trace": traceback.format_exc()}
                log(f"{t} {m} FAILED: {e!r}")
            docs[t]["metrics"][m] = v
            docs[t]["elapsed_s"][m] = round(time.time() - t1, 1)
            docs[t]["loadavg"][m] = [l1, os.getloadavg()[0]]
    return docs


# --------------------------------------------------------------------------
# Markdown report


def get(d, *path):
    for p in path:
        if not isinstance(d, dict) or p not in d:
            return None
        d = d[p]
    return d


def mib(b):
    return "-" if b is None else f"{b / 1048576:,.1f}"


def report(docs):
    docs = [d for d in docs if d]
    L = []
    tools = [d["tool"] if d["tool"] == "none" else f"{d['tool']} {d['version'].split()[-1]}" for d in docs]
    m0 = docs[0]["machine"]
    levels = f": {', '.join(m0['perflevels'])}" if m0.get("perflevels") else ""
    L.append(f"Machine: {m0['model']} ({m0['chip']}, {m0['cores']} cores{levels}, "
             f"{m0['mem_bytes'] // 2**30} GiB), {m0['macos']}. Client PTY {docs[0]['size']}. "
             f"Runs: {', '.join(d['date'] for d in docs)}.\n")

    def row(label, vals):
        L.append(f"| {label} | " + " | ".join(vals) + " |")

    def header(title, note=""):
        L.append(f"\n### {title}\n")
        if note:
            L.append(note + "\n")
        L.append("| | " + " | ".join(tools) + " |")
        L.append("|---|" + "---|" * len(tools))

    header("Throughput (MB/s: median [min-max], N runs)",
           "From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client "
           "received during the run. Tail: how long the client kept receiving output after the marker.")
    for kind, label in (("ascii", "100 MB ASCII"), ("sgr", "100 MB SGR + UTF-8"), ("longline", "20 MB single line")):
        vals, cb, dr = [], [], []
        for d in docs:
            t = get(d, "metrics", "throughput", kind)
            if not t:
                vals.append("-"); cb.append("-"); dr.append("-")
                continue
            s = t["mb_s"]
            if s.get("n"):
                fail = f", {t['failed']} failed" if t["failed"] else ""
                vals.append(f"{s['median']:,.1f} [{s['min']:,.1f}-{s['max']:,.1f}] N={s['n']}{fail}")
                cb.append(f"{t['client_bytes']['median'] / MB:,.1f} MB")
                dr.append(f"{t['drain_secs']['median'] * 1e3:,.0f} ms")
            else:
                r0 = t["runs"][0] if t["runs"] else {}
                vals.append(f"FAILED ({r0.get('error')} after {r0.get('secs') or 0:,.0f} s)")
                cb.append("-"); dr.append("-")
        row(label, vals)
        row(f"{label}: client bytes", cb)
        row(f"{label}: tail", dr)

    header("Memory (MiB: phys_footprint / RSS)",
           "Server: sum over all server-side processes of the tool. Pane shells are not counted.")
    for key, label in (("idle_1_pane", "server, idle, 1 pane"), ("idle_10_panes", "server, idle, 10 panes"),
                       ("idle_50_panes", "server, idle, 50 panes"),
                       ("history_100k_lines", "server, 100k colored lines in scrollback")):
        vals = []
        for d in docs:
            m = get(d, "metrics", "memory", key)
            if not m or "server_footprint" not in m:
                vals.append("-")
                continue
            extra = f" ({m['panes']} panes" if key.startswith("idle_") and key != "idle_1_pane" else ""
            if extra:
                extra += f", {m['floating']} floating)" if m.get("floating") else ")"
            vals.append(f"{mib(m['server_footprint'])} / {mib(m['server_rss'])}{extra}")
        row(label, vals)
    for a, b in ((1, 10), (10, 50), (1, 50)):
        vals = []
        for d in docs:
            ka = "idle_1_pane" if a == 1 else f"idle_{a}_panes"
            ma, mb = get(d, "metrics", "memory", ka), get(d, "metrics", "memory", f"idle_{b}_panes")
            if not ma or not mb or "server_footprint" not in ma or "server_footprint" not in mb:
                vals.append("-")
                continue
            na, nb = ma.get("panes") or a, mb.get("panes") or b
            fp = (mb["server_footprint"] - ma["server_footprint"]) / max(1, nb - na) / 1024
            rs = (mb["server_rss"] - ma["server_rss"]) / max(1, nb - na) / 1024
            vals.append(f"{fp:,.0f} / {rs:,.0f} KiB")
        row(f"server, cost per extra pane, {a} to {b} panes", vals)
    vals = []
    for d in docs:
        m = get(d, "metrics", "memory", "idle_50_panes")
        vals.append(f"{m.get('windows')} window(s)" if m and m.get("windows") is not None else "-")
    row("50 panes spread over", vals)
    vals = []
    for d in docs:
        m = get(d, "metrics", "memory", "history_100k_lines")
        vals.append(f"{m['retained_lines']:,}" if m and m.get("retained_lines") is not None else "-")
    row("lines retained in scrollback after the 100k-line cat", vals)
    vals = []
    for d in docs:
        m = get(d, "metrics", "throughput", "memory_after_ascii_cat")
        vals.append(f"{mib(m['server_footprint'])} / {mib(m['server_rss'])}" if m else "-")
    row("server, after 100 MB cat (default history)", vals)
    vals = []
    for d in docs:
        m = get(d, "metrics", "throughput", "memory_after_ascii_cat")
        vals.append(f"{m['retained_lines']:,}" if m and m.get("retained_lines") is not None else "-")
    row("lines retained after 100 MB cat (default history)", vals)
    vals = []
    for d in docs:
        c = get(d, "metrics", "memory", "idle_1_pane", "client")
        vals.append(f"{mib(c['footprint'])} / {mib(c['rss'])}" if c else "-")
    row("client, idle", vals)

    secs = next((get(d, "metrics", "idle", "seconds") for d in docs if get(d, "metrics", "idle", "seconds")), "?")
    header(f"Idle: one attached session, {secs} s window")
    for who in ("server", "client"):
        for field, label in (("cpu_ms", "CPU ms"), ("idle_wakeups", "idle wakeups"),
                             ("intr_wakeups", "interrupt wakeups")):
            vals = []
            for d in docs:
                v = get(d, "metrics", "idle", who, field)
                vals.append("-" if v is None else (f"{v:,.1f}" if field == "cpu_ms" else f"{v:,}"))
            row(f"{who}: {label}", vals)

    header("Startup (ms: median [p25-p75] of warm runs, then the first run)")

    def hf(d, k):
        h = get(d, "metrics", "startup", k)
        if not h:
            return "-"
        if "error" in h:
            return "error"
        w = h["warm"]
        return f"{w['median'] * 1e3:,.1f} [{w['p25'] * 1e3:,.1f}-{w['p75'] * 1e3:,.1f}] N={w['n']}; first {h['first_s'] * 1e3:,.1f}"

    row("create session + kill server (hyperfine)", [hf(d, "create_kill") for d in docs])
    row(f"attach, draw, detach (hyperfine; includes {ATTACH_QUIET_MS} ms quiet wait)", [hf(d, "attach_detach") for d in docs])
    vals = []
    for d in docs:
        s = get(d, "metrics", "startup", "new_session_to_command_ms")
        vals.append(f"{s['median']:,.1f} [{s['p25']:,.1f}-{s['p75']:,.1f}] N={s['n']}" if s and s.get("n") else "-")
    row("new session until a typed command runs (driver)", vals)

    header("Input latency (us: p50 / p95 / p99 / max)")
    for key, label in (("echo_idle", "key echo, idle"), ("echo_flood", "key echo, other pane runs `yes`"),
                       ("esc_default", "bare ESC, default config"), ("esc_time_0", "bare ESC, escape-time 0"),
                       ("esc_kitty", "bare ESC, terminal with kitty keyboard protocol")):
        vals = []
        for d in docs:
            l = get(d, "metrics", "latency", key)
            if not l or not l.get("us"):
                vals.append("-" if not l else "error")
                continue
            u = l["us"]
            extra = f", {l['timeouts']} timeouts" if l.get("timeouts") else ""
            extra += f", {l['stray_hits']} stray" if l.get("stray_hits") else ""
            vals.append(f"{u['median']:,.0f} / {u['p95']:,.0f} / {u['p99']:,.0f} / {u['max']:,.0f} (N={l['n']}{extra})")
        row(label, vals)
        if key.startswith("echo"):
            rm = [get(d, "metrics", "latency", key, "round_medians_us") for d in docs]
            row(f"{label}: median of each round", ["-" if not r else " ".join(f"{x:,.0f}" for x in r if x is not None)
                                                   for r in rm])

    header("Binary")
    row("size", [f"{get(d, 'metrics', 'binary', 'bytes') / 1e6:,.2f} MB" if get(d, "metrics", "binary", "bytes")
                 else "-" for d in docs])
    row("non-system dylibs", [", ".join(os.path.basename(x) for x in get(d, "metrics", "binary", "non_system_dylibs") or [])
                              or "-" for d in docs])
    row("dylibs total", [str(len(get(d, "metrics", "binary", "dylibs") or [])) for d in docs])

    errs = [(d["tool"], m, v["error"]) for d in docs for m, v in d["metrics"].items()
            if isinstance(v, dict) and "error" in v]
    if errs:
        L.append("\n### Errors\n")
        for t, m, e in errs:
            L.append(f"- {t} / {m}: {e}")
    return "\n".join(L) + "\n"


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--tools", default="none,tmux,zellij,lif")
    r.add_argument("--only", default="", help="comma list of: " + ",".join(METRICS))
    r.add_argument("--quick", action="store_true", help="few repetitions; for checking the harness itself")
    r.add_argument("--reps", type=int, default=5)
    r.add_argument("--samples", type=int, default=1000, help="echo latency samples per tool and scenario")
    r.add_argument("--rounds", type=int, default=5, help="latency blocks per tool, interleaved across tools")
    r.add_argument("--latency-rest", type=int, default=45, help="seconds of rest before the latency metric")
    r.add_argument("--idle-secs", type=int, default=30)
    r.add_argument("--tag", default="")
    p = sub.add_parser("report")
    p.add_argument("files", nargs="+")
    p.add_argument("-o", "--out")
    a = ap.parse_args()

    if a.cmd == "report":
        md = report([json.load(open(f)) for f in a.files])
        if a.out:
            open(a.out, "w").write(md)
        print(md)
        return

    if not os.access(DRIVER, os.X_OK):
        sys.exit(f"driver not built: {DRIVER} (bench/run.sh builds it)")
    for k in ("TMUX", "TMUX_PANE", "ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"):
        os.environ.pop(k, None)
    os.makedirs(RESULTS, exist_ok=True)
    files = gen_data()
    stamp = datetime.datetime.now().strftime("%Y-%m-%d-%H%M")
    tag = f"-{a.tag}" if a.tag else ""
    tools = []
    for t in a.tools.split(","):
        if t not in ADAPTERS:
            sys.exit(f"unknown tool {t}; known: {', '.join(ADAPTERS)}")
        if ADAPTERS[t].available():
            tools.append(t)
        else:
            log(f"skipping {t}: binary not found ({ADAPTERS[t].binary_name()})")
    docs = run_all(tools, a, files)
    written = []
    for tool in tools:
        path = os.path.join(RESULTS, f"{stamp}{tag}-{tool}.json")
        with open(path, "w") as f:
            json.dump(docs[tool], f, indent=1)
        written.append(path)
        log("wrote", path)
    if written:
        md = report([json.load(open(p)) for p in written])
        path = os.path.join(RESULTS, f"{stamp}{tag}-summary.md")
        with open(path, "w") as f:
            f.write(md)
        log("wrote", path)
        print(md)


if __name__ == "__main__":
    main()
