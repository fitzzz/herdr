#!/usr/bin/env python3
"""Opt-in Linux responsiveness measurements on an isolated remote allocation.

No build, download, installed Herdr discovery, or connection to an existing session.
See responsiveness_bench.md for boundaries and the baseline/candidate procedure.
"""

import argparse
from collections import Counter
import fcntl
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import platform
import pty
import re
import selectors
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import uuid


PINNED_BASE = "7b116c05bfda646af39d2524c54e70c751f57ee8"
ECHO = re.compile(r"ECHO(\d{8})")
HISTORY = re.compile(r"HISTORY(\d{6})")


def distribution(values):
    """Nearest-rank percentiles; empty observations are absent, never zero."""
    values = sorted(values)
    if not values:
        return {"count": 0, "p50": None, "p95": None, "p99": None, "max": None}
    return {"count": len(values), **{
        label: values[max(0, math.ceil(len(values) * quantile) - 1)]
        for label, quantile in (("p50", .50), ("p95", .95), ("p99", .99), ("max", 1))
    }}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def isolated_environment(root, session):
    # Deliberately do not copy credentials, provider homes, SSH agent routing,
    # Herdr overrides, tracing variables, or the user's shell startup environment.
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "HOME": str(root / "home"), "SHELL": "/bin/sh", "TERM": "xterm-256color",
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_DATA_HOME": str(root / "data"), "XDG_CACHE_HOME": str(root / "cache"),
        "XDG_STATE_HOME": str(root / "state"), "XDG_RUNTIME_DIR": str(root / "run"),
        "TMPDIR": str(root / "tmp"), "HERDR_SESSION": session,
        "HERDR_DISABLE_SOUND": "1", "HERDR_BENCH_ROOT": str(root),
    }


def owned_socket_path(root, session):
    if not re.fullmatch(r"latency-[0-9a-f]{12}", session):
        raise ValueError("only generated benchmark session names are allowed")
    path = root / "config" / "herdr" / "sessions" / session / "herdr.sock"
    if len(os.fsencode(path)) >= 108:
        raise ValueError("isolated socket exceeds Linux sockaddr_un capacity")
    return path


class JsonPeer:
    """One FIFO JSON connection. Each workload thread owns its own peer."""

    def __init__(self, path, expected_pid):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            self.sock.connect(str(path))
            pid, uid, _ = struct.unpack("3i", self.sock.getsockopt(
                socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
            if pid != expected_pid or uid != os.getuid():
                raise RuntimeError(f"socket peer is not the owned server: {pid}/{uid}")
            self.stream = self.sock.makefile("rb")
        except BaseException:
            self.sock.close()
            raise
        self.serial = 0
        self.requests = 0
        self.bytes_received = 0

    def send(self, method, params):
        self.serial += 1
        request_id = f"bench-{self.serial}"
        data = json.dumps({"id": request_id, "method": method, "params": params})
        self.sock.sendall(data.encode() + b"\n")
        self.requests += 1
        return request_id

    def receive(self):
        line = self.stream.readline()
        self.bytes_received += len(line)
        if not line:
            raise EOFError("owned API connection closed")
        return json.loads(line)

    def request(self, method, **params):
        request_id = self.send(method, params)
        response = self.receive()
        if response.get("id") != request_id:
            raise RuntimeError(f"response order/identity mismatch: {response}")
        if "error" in response:
            raise RuntimeError(f"{method}: {response['error']}")
        return response["result"]

    def close(self):
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.stream.close()
        self.sock.close()


class Samples:
    """Bounded by the explicit run length and prescribed input frequency."""

    def __init__(self):
        self.lock = threading.Lock()
        self.sent = {}
        self.observed = {"direct": {}, "tui": {}, "child_write": {}}
        self.arrival_order = {name: [] for name in self.observed}
        self.scrolls = []
        self.child_duplicate_receipts = 0

    def send(self, sequence, path, scheduled_ns):
        with self.lock:
            self.sent[sequence] = {
                "sequence": sequence, "path": path, "scheduled_ns": scheduled_ns,
                "sent_ns": time.monotonic_ns(),
            }

    def observe(self, name, sequences, observed_ns):
        with self.lock:
            for sequence in sequences:
                if name == "child_write" and sequence in self.observed[name]:
                    self.child_duplicate_receipts += 1
                if sequence in self.sent and sequence not in self.observed[name]:
                    self.observed[name][sequence] = observed_ns
                    self.arrival_order[name].append(sequence)

    def send_complete(self, sequence):
        with self.lock:
            self.sent[sequence]["send_complete_ns"] = time.monotonic_ns()

    def report(self):
        with self.lock:
            result = {}
            for observer, observations in self.observed.items():
                for path in ("direct", "tui"):
                    expected = {seq: sample for seq, sample in self.sent.items()
                                if sample["path"] == path}
                    latencies = [(observations[seq] - sample["sent_ns"]) / 1e6
                                 for seq, sample in expected.items() if seq in observations]
                    result[f"{path}_to_{observer}_ms"] = {
                        **distribution(latencies), "sent": len(expected),
                        "unobserved_at_end": len(expected.keys() - observations.keys()),
                    }
                order = self.arrival_order[observer]
                result[f"{observer}_order_regressions"] = sum(
                    later < earlier for earlier, later in zip(order, order[1:]))
            result["input_schedule_lateness_ms"] = distribution([
                (sample["sent_ns"] - sample["scheduled_ns"]) / 1e6
                for sample in self.sent.values()])
            result["scroll_ms"] = distribution([
                (sample["observed_ns"] - sample["sent_ns"]) / 1e6
                for sample in self.scrolls if sample.get("observed_ns") is not None])
            result["scroll_sent"] = len(self.scrolls)
            result["scroll_unobserved_at_end"] = sum(
                sample.get("observed_ns") is None for sample in self.scrolls)
            result["child_duplicate_receipts"] = self.child_duplicate_receipts
            for path in ("direct", "tui"):
                result[f"{path}_scroll_ms"] = distribution([
                    (sample["observed_ns"] - sample["sent_ns"]) / 1e6
                    for sample in self.scrolls if sample["path"] == path and
                    sample.get("observed_ns") is not None])
            return result

    def rows(self):
        with self.lock:
            return [{**sample, **{f"{name}_ns": observations.get(sequence)
                                  for name, observations in self.observed.items()}}
                    for sequence, sample in self.sent.items()]


class TerminalCells:
    """Apply ANSI cells, publishing synchronized updates only at their commit."""

    def __init__(self, cols, rows):
        import pyte
        self.commits = []
        self.sync_commits = 0
        owner = self

        class Screen(pyte.Screen):
            def __init__(self):
                self.synchronized = False
                super().__init__(cols, rows)

            def set_mode(self, *modes, **kwargs):
                super().set_mode(*modes, **kwargs)
                if kwargs.get("private") and 2026 in modes:
                    self.synchronized = True

            def reset_mode(self, *modes, **kwargs):
                super().reset_mode(*modes, **kwargs)
                if kwargs.get("private") and 2026 in modes:
                    self.synchronized = False
                    owner.sync_commits += 1
                    owner.publish()

        self.screen = Screen()
        self.stream = pyte.ByteStream(self.screen)

    def publish(self):
        self.commits.append(("\n".join(self.screen.display), time.monotonic_ns()))

    def feed(self, data):
        self.commits = []
        self.stream.feed(data)
        if not self.screen.synchronized:
            self.publish()
        return self.commits


class Tui:
    def __init__(self, binary, env, session, cols, rows, samples, changed, stopped,
                 parse_screen=True):
        self.samples = samples
        self.changed = changed
        self.stopped = stopped
        self.parse_screen = parse_screen
        if parse_screen:
            self.cells = TerminalCells(cols, rows)
        self.lock = threading.Lock()
        self.text = ""
        self.bytes_received = 0
        self.read_chunks = 0
        self.screen_updates = 0
        self.parse_ns = 0
        self.query_tail = b""
        pid, fd = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            os.chdir(env["HERDR_BENCH_ROOT"])
            os.execve(str(binary), [str(binary), "--session", session, "client"], env)
        self.pid, self.fd = pid, fd
        os.set_blocking(fd, False)
        self.cols, self.rows = cols, rows

    def read(self):
        try:
            data = os.read(self.fd, 65536)
        except BlockingIOError:
            return
        if not data:
            raise EOFError("owned TUI closed")
        self.bytes_received += len(data)
        self.read_chunks += 1
        # Supply replies from the simulated outer terminal, never from a user terminal.
        queries = self.query_tail + data
        for pattern, reply in ((b"\x1b[6n", b"\x1b[1;1R"),
                               (b"\x1b[c", b"\x1b[?1;2c"),
                               (b"\x1b[>c", b"\x1b[>0;0;0c"),
                               (b"\x1b[?u", b"\x1b[?0u"),
                               (b"\x1b[?2026$p", b"\x1b[?2026;1$y"),
                               (b"\x1b]10;?", b"\x1b]10;rgb:eeee/eeee/eeee\x1b\\"),
                               (b"\x1b]11;?", b"\x1b]11;rgb:0000/0000/0000\x1b\\")):
            # Match only queries completed by this read, including fragmented queries.
            for match in re.finditer(re.escape(pattern), queries):
                if match.end() > len(self.query_tail):
                    self.write(reply)
        self.query_tail = queries[-16:]
        if self.parse_screen:
            start = time.monotonic_ns()
            with self.lock:
                commits = self.cells.feed(data)
                for text, observed_ns in commits:
                    if text != self.text:
                        self.screen_updates += 1
                    self.text = text
                    self.samples.observe("tui", (int(x) for x in ECHO.findall(text)), observed_ns)
            self.parse_ns += time.monotonic_ns() - start
            if commits:
                self.changed.set()

    def visible(self):
        with self.lock:
            return self.text

    def write(self, data):
        # These messages are small; partial writes still must preserve byte order.
        view = memoryview(data)
        while view and not self.stopped.is_set():
            try:
                count = os.write(self.fd, view)
                view = view[count:]
            except BlockingIOError:
                with selectors.DefaultSelector() as selector:
                    selector.register(self.fd, selectors.EVENT_WRITE)
                    selector.select(.1)
        if view:
            raise InterruptedError("owned TUI input cancelled")

    def close(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        os.waitpid(self.pid, 0)
        os.close(self.fd)


def producer():
    """An inert interactive stand-in: real raw PTY, ordered echo, static history."""
    root = Path(os.environ["HERDR_BENCH_ROOT"])
    pane = os.environ.get("HERDR_PANE_ID", "unknown")
    options = json.loads((root / "producer.json").read_text())
    original = termios.tcgetattr(0)
    raw = termios.tcgetattr(0)
    raw[3] &= ~(termios.ECHO | termios.ICANON)
    raw[6][termios.VMIN], raw[6][termios.VTIME] = 1, 0
    termios.tcsetattr(0, termios.TCSANOW, raw)
    audit = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
    audit.setblocking(False)
    ledger = (root / ("audit-" + hashlib.sha256(pane.encode()).hexdigest()[:16] + ".jsonl")).open(
        "w", buffering=1) if options["audit"] else None
    buffer = b""
    sequence = 0
    next_output = time.monotonic()
    emitting = False
    history = "".join(f"HISTORY{i:06d}\r\n" for i in range(options["rows"] * 4))
    os.write(1, (history + "SCROLLBOTTOM00000000\r\nBENCHREADY\r\n").encode())
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(0, selectors.EVENT_READ)
            while True:
                period = 1 / options["output_hz"]
                timeout = max(0, next_output - time.monotonic()) if emitting else None
                for _, _ in selector.select(timeout):
                    data = os.read(0, 4096)
                    if not data:
                        return
                    buffer += data.replace(b"\r", b"\n")
                    while b"\n" in buffer:
                        line, buffer = buffer.split(b"\n", 1)
                        match = re.fullmatch(rb"I(\d{8})", line)
                        if match:
                            value = int(match[1])
                            os.write(1, f"ECHO{value:08d}\r\n".encode())
                            if options["audit"]:
                                receipt = json.dumps({"pane": pane, "sequence": value,
                                                      "ns": time.monotonic_ns()})
                                ledger.write(receipt + "\n")
                                try:
                                    audit.sendto(receipt.encode(), str(root / "audit.sock"))
                                except (BlockingIOError, FileNotFoundError, ConnectionRefusedError):
                                    # Evidence loss is visible as missing receipts; never stall input.
                                    pass
                        elif re.fullmatch(rb"H\d{8}", line):
                            os.write(1, (history + "SCROLLBOTTOM" + line[1:].decode() +
                                         "\r\nBENCHREADY\r\n").encode())
                        elif line == b"G":
                            visible = (root / "visible-pane").read_text()
                            emitting = (options["output"] == "visible" and pane == visible) or (
                                options["output"] == "hidden" and pane != visible)
                            next_output = time.monotonic()
                        elif line == b"S":
                            emitting = False
                        elif line and ledger:
                            ledger.write(json.dumps({"pane": pane, "malformed_input": True,
                                                     "ns": time.monotonic_ns()}) + "\n")
                now = time.monotonic()
                if emitting and now >= next_output:
                    sequence += 1
                    os.write(1, f"LOAD{sequence:08d}\r\n".encode())
                    # Same monotonic paced-output policy as release_perf_producer.pl.
                    # After falling behind, do not produce an unbounded catch-up burst.
                    next_output += period
                    if next_output < now - period:
                        next_output = now + period
    finally:
        termios.tcsetattr(0, termios.TCSANOW, original)
        audit.close()
        if ledger:
            ledger.close()


def proc_sample(pid):
    """Linux counters: runqueue wait is explicitly not application queue age."""
    path = Path(f"/proc/{pid}")
    fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
    status = (path / "status").read_text()
    sched = (path / "schedstat").read_text().split()
    return {"pid": pid, "ns": time.monotonic_ns(),
            "user_ticks": int(fields[11]), "system_ticks": int(fields[12]),
            "start_ticks": int(fields[19]),
            "rss_kib": int(re.search(r"VmRSS:\s+(\d+)", status)[1]),
            "runtime_ns": int(sched[0]), "runqueue_wait_ns": int(sched[1]),
            "voluntary_switches": int(re.search(
                r"\nvoluntary_ctxt_switches:\s+(\d+)", status)[1])}


class Case:
    def __init__(self, args):
        self.args = args
        self.out = args.output.resolve()
        self.out.mkdir(parents=True, exist_ok=False)
        self.root = Path(tempfile.mkdtemp(prefix="hdl-", dir="/var/tmp"))
        self.owner = uuid.uuid4().hex
        (self.root / "owner").write_text(self.owner)
        self.root_identity = self.root.stat().st_ino
        self.session = f"latency-{uuid.uuid4().hex[:12]}"
        self.env = isolated_environment(self.root, self.session)
        self.socket_path = owned_socket_path(self.root, self.session)
        self.stop = threading.Event()
        self.samples = Samples()
        self.peers, self.tuis, self.threads = [], [], []
        self.errors, self.resources, self.api_samples = [], [], []
        self.subscription_stats = []
        self.server = None
        self.audit = None
        self.visible_changed = threading.Event()
        self.direct_text = ""
        self.direct_lock = threading.Lock()
        self.burst_counts = {}
        self.cleanup_result = {}
        self.child_ledger = []
        self.phase = "setup"
        self.binary_sha = sha256(args.binary)

    def peer(self):
        peer = JsonPeer(self.socket_path, self.server.pid)
        self.peers.append(peer)
        return peer

    def thread(self, label, fn):
        def guarded():
            try:
                fn()
            except BaseException as error:
                if not self.stop.is_set():
                    self.errors.append({"thread": label, "error": repr(error)})
                    self.stop.set()
                    self.visible_changed.set()
        thread = threading.Thread(target=guarded, name=label, daemon=True)
        self.threads.append(thread)
        thread.start()

    def start(self):
        for key in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME",
                    "XDG_STATE_HOME", "XDG_RUNTIME_DIR", "TMPDIR"):
            Path(self.env[key]).mkdir(mode=0o700)
        self.binary_version = subprocess.run(
            [str(self.args.binary), "--version"], env=self.env, cwd=self.root,
            check=True, capture_output=True, text=True).stdout.strip()
        script = Path(__file__).resolve()
        shell = self.root / "stand-in"
        shell.write_text("#!/bin/sh\nexec " + shlex.join(
            [sys.executable, str(script), "producer"]) + "\n")
        shell.chmod(0o700)
        (self.root / "producer.json").write_text(json.dumps({
            "rows": self.args.rows, "output_hz": self.args.output_hz,
            "output": self.args.workload, "audit": not self.args.no_audit}))
        config = self.root / "config" / "herdr"
        config.mkdir()
        (config / "config.toml").write_text(
            "onboarding = false\n[update]\nversion_check = false\nmanifest_check = false\n"
            f"[terminal]\ndefault_shell = {json.dumps(str(shell))}\nshell_mode = 'non_login'\n"
            f"[server]\nheadless_cols = {self.args.cols}\nheadless_rows = {self.args.rows}\n"
            "[ui.toast]\ndelivery = 'off'\n")
        self.server_log = open(self.out / "server.log", "wb")
        self.server = subprocess.Popen(
            [str(self.args.binary), "--session", self.session, "server"],
            env=self.env, cwd=self.root, stdin=subprocess.DEVNULL,
            stdout=self.server_log, stderr=subprocess.STDOUT, start_new_session=True)
        (self.out / "isolation.json").write_text(json.dumps({
            "root": str(self.root), "root_inode": self.root_identity, "owner": self.owner,
            "session": self.session, "api_socket": str(self.socket_path),
            "server_pid": self.server.pid, "server_start_ticks": proc_sample(self.server.pid)["start_ticks"],
            "environment": self.env,
        }, indent=2) + "\n")
        # Readiness polling is setup only, excluded from all measurement intervals.
        while not self.stop.is_set():
            if self.server.poll() is not None:
                raise RuntimeError("owned server exited during startup; inspect server.log")
            try:
                control = self.peer()
                break
            except (FileNotFoundError, ConnectionRefusedError):
                self.stop.wait(.05)
        else:
            raise InterruptedError("startup cancelled")
        self.control = control
        workspaces = control.request("workspace.list")["workspaces"]
        if workspaces:
            raise RuntimeError("fresh isolated server unexpectedly restored workspaces")
        created = control.request("workspace.create", cwd=str(self.root), focus=True,
                                  label="latency-bench")
        self.panes = [created["root_pane"]["pane_id"]]
        for index in range(1, self.args.panes):
            tab = control.request("tab.create", workspace_id=created["workspace"]["workspace_id"],
                                  cwd=str(self.root), focus=False, label=f"bench-{index}")
            self.panes.append(tab["root_pane"]["pane_id"])
        self.pane = self.panes[0]
        control.request("pane.focus", pane_id=self.pane)
        (self.root / "visible-pane").write_text(self.pane)
        for pane in self.panes:
            while "BENCHREADY" not in control.request(
                    "pane.read", pane_id=pane, source="recent", format="text")["read"]["text"]:
                if self.stop.wait(.05):
                    raise InterruptedError("stand-in setup cancelled")
        for pane in self.panes[:min(121, self.args.panes)]:
            control.request("pane.report_agent", pane_id=pane, source="bench", agent="codex",
                            state="idle", seq=1)
        listed = control.request("pane.list")["panes"]
        if len(listed) != self.args.panes or len(set(self.panes)) != self.args.panes or (
                {pane["pane_id"] for pane in listed} != set(self.panes)):
            raise RuntimeError("pane identity/cardinality changed during setup")
        self.identities = listed
        self.recognized_agents = sum(pane.get("agent") == "codex" for pane in listed)
        if self.recognized_agents != min(121, self.args.panes):
            raise RuntimeError("stand-in reports did not establish the requested recognized agents")
        if self.args.mode != "direct":
            tui = Tui(self.args.binary, self.env, self.session, self.args.cols, self.args.rows,
                      self.samples, self.visible_changed, self.stop, not self.args.raw_drain)
            self.tuis.append(tui)
        if self.args.slow_reader_hz:
            slow = Tui(self.args.binary, self.env, self.session, self.args.cols, self.args.rows,
                       self.samples, self.visible_changed, self.stop, False)
            self.tuis.append(slow)
            self.thread("slow-tui", lambda: self.tui_loop(slow, self.args.slow_reader_hz))
        # Both PTY forks precede reader threads; do not fork a multithreaded harness.
        if self.args.mode != "direct":
            self.thread("tui-observer", lambda: self.tui_loop(tui, 0))
            if not self.args.raw_drain:
                if not self.wait_visible(lambda text: "BENCHREADY" in text):
                    raise InterruptedError("TUI readiness cancelled")
        if not self.args.no_audit:
            self.audit = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
            self.audit.bind(str(self.root / "audit.sock"))
            self.audit.settimeout(.1)
            self.thread("child-write-observer", self.audit_loop)
        if self.args.mode != "tui":
            direct_peer = self.peer()
            self.thread("direct-observer", lambda: self.direct_loop(direct_peer))
        for index, group in enumerate(subscription_groups(self.panes, self.args.subscriptions,
                                                         self.args.connections)):
            peer = self.peer()
            request_id = peer.send("events.subscribe", {"subscriptions": group})
            started = peer.receive()
            if started.get("id") != request_id or started.get("result", {}).get(
                    "type") != "subscription_started":
                raise RuntimeError(f"subscription setup did not succeed: {started}")
            stats = {"connection": index, "subscriptions": len(group), "events": 0,
                     "errors": [], "bytes": 0, "first_events": []}
            self.subscription_stats.append(stats)
            self.thread(f"subscriber-{index}", lambda p=peer, s=stats: self.subscription_loop(p, s))
        for index in range(self.args.api_burst_clients):
            burst_peer = self.peer()
            self.thread(f"burst-{index}", lambda i=index, p=burst_peer: self.burst_loop(i, p))
        if not self.args.no_resource_sampling:
            self.thread("resources", self.resource_loop)
        for pane in self.panes:
            control.request("pane.send_input", pane_id=pane, text="G", keys=["Enter"])

    def tui_loop(self, tui, slow_hz):
        with selectors.DefaultSelector() as selector:
            selector.register(tui.fd, selectors.EVENT_READ)
            while not self.stop.is_set():
                if slow_hz and self.stop.wait(1 / slow_hz):
                    return
                if selector.select(.1):
                    tui.read()

    def audit_loop(self):
        while not self.stop.is_set():
            try:
                receipt = json.loads(self.audit.recv(4096))
            except socket.timeout:
                continue
            if receipt["pane"] == self.pane:
                self.samples.observe("child_write", [receipt["sequence"]], receipt["ns"])

    def direct_loop(self, peer):
        next_read = time.monotonic()
        while not self.stop.is_set():
            start = time.monotonic_ns()
            text = peer.request("pane.read", pane_id=self.pane,
                                source="visible", format="text")["read"]["text"]
            end = time.monotonic_ns()
            self.api_samples.append({"method": "pane.read", "start_ns": start, "end_ns": end})
            with self.direct_lock:
                self.direct_text = text
            self.visible_changed.set()
            self.samples.observe("direct", (int(x) for x in ECHO.findall(text)), end)
            next_read = max(next_read + 1 / self.args.read_hz, time.monotonic())
            self.stop.wait(max(0, next_read - time.monotonic()))

    def subscription_loop(self, peer, stats):
        while not self.stop.is_set():
            event = peer.receive()
            stats["events"] += 1
            stats["bytes"] = peer.bytes_received
            if len(stats["first_events"]) < 512:
                stats["first_events"].append({"observed_ns": time.monotonic_ns(), "event": event})
            if "error" in event:
                stats["errors"].append(event)
                self.errors.append({"subscription_error": event})
                return

    def burst_loop(self, index, peer):
        counts = self.burst_counts[index] = {}
        for method in itertools.cycle(("pane.get", "agent.list", "workspace.list")):
            if self.stop.is_set():
                return
            start = time.monotonic_ns()
            peer.request(method, **({"pane_id": self.pane} if method == "pane.get" else {}))
            counts[method] = counts.get(method, 0) + 1
            # Aggregate durations rather than unbounded per-request labels/logs.
            counts[method + "_total_ns"] = counts.get(method + "_total_ns", 0) + (
                time.monotonic_ns() - start)

    def resource_loop(self):
        while not self.stop.is_set():
            for role, pid in [("server", self.server.pid), ("harness", os.getpid())] + [
                    (f"tui-{i}", tui.pid) for i, tui in enumerate(self.tuis)]:
                try:
                    self.resources.append({"role": role, "phase": self.phase, **proc_sample(pid)})
                except FileNotFoundError:
                    self.errors.append({"process_exited": role, "pid": pid})
            self.stop.wait(1)

    def visible(self):
        if self.args.mode != "direct":
            return self.tuis[0].visible()
        with self.direct_lock:
            return self.direct_text

    def wait_visible(self, predicate, end=None):
        while not self.stop.is_set():
            self.visible_changed.clear()
            text = self.visible()
            if predicate(text):
                return text
            remaining = None if end is None else end - time.monotonic()
            if remaining is not None and remaining <= 0:
                return None
            self.visible_changed.wait(remaining)
        return None

    def run(self):
        self.start()
        self.phase = "input"
        start = time.monotonic_ns()
        end = start + int(self.args.seconds * 1e9)
        interval = int(1e9 / self.args.input_hz)
        sequence = 0
        for scheduled in range(start, end, interval):
            if self.stop.wait(max(0, (scheduled - time.monotonic_ns()) / 1e9)):
                return
            sequence += 1
            path = "direct" if self.args.mode == "direct" or (
                self.args.mode == "mixed" and sequence % 2 == 0) else "tui"
            self.samples.send(sequence, path, scheduled)
            if path == "direct":
                self.control.request("pane.send_input", pane_id=self.pane,
                                     text=f"I{sequence:08d}", keys=["Enter"])
            else:
                self.tuis[0].write(f"I{sequence:08d}\r".encode())
            self.samples.send_complete(sequence)
        self.stop.wait(max(0, (end - time.monotonic_ns()) / 1e9))
        self.input_window_ns = [start, time.monotonic_ns()]
        if self.stop.is_set() or self.args.raw_drain or self.args.scroll_samples == 0:
            return
        # Static history makes a changed viewport evidence of scroll delivery.
        # Foreground output is explicitly paused for this phase; no product delay changes.
        self.control.request("pane.send_input", pane_id=self.pane, text="S", keys=["Enter"])
        self.phase = "scroll"
        self.control.request("pane.send_input", pane_id=self.pane, text="H00000001", keys=["Enter"])
        self.control.request("pane.scroll", pane_id=self.pane, offset_from_bottom=0)
        if not self.wait_visible(lambda text: "SCROLLBOTTOM00000001" in text):
            return
        for index in range(self.args.scroll_samples):
            before = tuple(HISTORY.findall(self.visible()))
            sample = {"index": index, "before": before, "sent_ns": time.monotonic_ns(),
                      "observed_ns": None, "path": "direct"}
            self.samples.scrolls.append(sample)
            if self.args.mode == "direct" or (self.args.mode == "mixed" and index % 2 == 0):
                self.control.request("pane.scroll", pane_id=self.pane,
                                     offset_from_bottom=10 if index % 2 == 0 else 0)
            else:
                sample["path"] = "tui"
                text = self.tuis[0].visible().splitlines()
                locations = [(x + 2, y + 1) for y, line in enumerate(text)
                             if (x := line.find("HISTORY")) >= 0]
                if not locations:
                    raise RuntimeError("no visible history cells for a safe pane-local wheel input")
                x, y = locations[len(locations) // 2]
                button = 64 if index % 2 == 0 else 65
                self.tuis[0].write(f"\x1b[<{button};{x};{y}M".encode())
            # No latency acceptance threshold. The explicit observation window censors
            # an undelivered scroll rather than waiting indefinitely for lost evidence.
            observation_end = time.monotonic() + self.args.seconds
            direction = -1 if index % 2 == 0 else 1
            after_text = self.wait_visible(
                lambda text: scroll_observed(before, tuple(HISTORY.findall(text)), direction),
                observation_end)
            if after_text is not None:
                sample["observed_ns"] = time.monotonic_ns()
                sample["after"] = tuple(HISTORY.findall(after_text))
            if sample["observed_ns"] is None:
                break

    def cleanup(self):
        self.stop.set()
        self.visible_changed.set()
        for peer in self.peers:
            peer.close()
        for tui in self.tuis:
            tui.close()
        for thread in self.threads:
            thread.join()
        if self.audit:
            self.audit.close()
        if self.server:
            if self.server.poll() is None:
                self.server.send_signal(signal.SIGINT)
            self.cleanup_result["server_exit"] = self.server.wait()
            self.server_log.close()
        # Never remove a root whose ownership marker or inode has changed.
        if self.root.stat().st_ino != self.root_identity or (
                self.root / "owner").read_text() != self.owner:
            raise RuntimeError("benchmark cleanup ownership changed; state retained")
        for ledger in self.root.glob("audit-*.jsonl"):
            for line in ledger.read_text().splitlines():
                try:
                    self.child_ledger.append(json.loads(line))
                except json.JSONDecodeError:
                    self.errors.append({"partial_child_ledger": ledger.name})
        survivors = []
        for process in Path("/proc").iterdir():
            if process.name.isdigit() and int(process.name) != os.getpid():
                try:
                    env = (process / "environ").read_bytes().split(b"\0")
                    if f"HERDR_BENCH_ROOT={self.root}".encode() in env:
                        survivors.append(int(process.name))
                except (PermissionError, FileNotFoundError, ProcessLookupError):
                    pass
        self.cleanup_result["owned_survivors"] = survivors
        if survivors:
            self.cleanup_result["state_retained"] = str(self.root)
        else:
            shutil.rmtree(self.root)
            self.cleanup_result["state_removed"] = True

    def save(self):
        write_jsonl(self.out / "input.jsonl", self.samples.rows())
        write_jsonl(self.out / "scroll.jsonl", self.samples.scrolls)
        write_jsonl(self.out / "resources.jsonl", self.resources)
        write_jsonl(self.out / "observer-api.jsonl", self.api_samples)
        write_jsonl(self.out / "child-input.jsonl", self.child_ledger)
        final_hash = sha256(self.args.binary)
        if final_hash != self.binary_sha:
            self.errors.append({"binary_changed_during_case": final_hash})
        report = {
            "schema": 1, "pinned_base": PINNED_BASE,
            "binary": str(self.args.binary), "sha256": self.binary_sha,
            "version": getattr(self, "binary_version", None),
            "source_revision": self.args.source_revision,
            "variant": self.args.variant, "round": self.args.round,
            "host": platform.uname()._asdict(), "clock_ticks": os.sysconf("SC_CLK_TCK"),
            "python": sys.version, "python_executable": sys.executable,
            "affinity": sorted(os.sched_getaffinity(0)),
            "session": self.session, "isolated_root": str(self.root),
            "parameters": {key: str(value) if isinstance(value, Path) else value
                           for key, value in vars(self.args).items() if key != "func"},
            "identities": getattr(self, "identities", []),
            "recognized_agents_at_setup": getattr(self, "recognized_agents", None),
            "input_window_ns": getattr(self, "input_window_ns", None),
            "measurements": self.samples.report(),
            "input_processing": input_processing(self.samples.rows(), self.child_ledger,
                                                 getattr(self, "pane", None), not self.args.no_audit),
            "subscriptions": self.subscription_stats, "burst": self.burst_counts,
            "tui": [{"pid": tui.pid, "host_output_bytes": tui.bytes_received,
                     "read_chunks": tui.read_chunks, "cell_updates": tui.screen_updates,
                     "host_sync_commits": tui.cells.sync_commits if tui.parse_screen else None,
                     "observer_parse_ns": tui.parse_ns} for tui in self.tuis],
            "process_summary": resource_summary(self.resources),
            "errors": self.errors, "cleanup": self.cleanup_result,
            "limits": ["TUI cells are a simulated host terminal, not GPU presentation.",
                       "Direct observations include pane.read cost and polling quantization.",
                       "Host output chunks/cell changes are not server protocol frame counts.",
                       "Application queue age, PTY actor waits and blocked-time attribution are unmeasured.",
                       "Unobserved echo is not proof of dropped input; child receipts distinguish processing.",
                       "Audit receipts can be lost under datagram backpressure.",
                       "The child ledger records consumption; missing entries may still be in flight at cleanup.",
                       "Status-subscription payloads do not expose the internal event sequence.",
                       "No native provider/conversation is launched or recovered."],
        }
        (self.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")


def write_jsonl(path, rows):
    with path.open("w") as stream:
        for row in rows:
            stream.write(json.dumps(row) + "\n")


def scroll_observed(before, after, direction):
    """Ignore repaint-only changes and history movement in the wrong direction."""
    return bool(before and after and (int(after[-1]) - int(before[-1])) * direction > 0)


def resource_summary(samples):
    grouped = {}
    for sample in samples:
        grouped.setdefault((sample["role"], sample["phase"]), []).append(sample)
    summaries = []
    ticks = os.sysconf("SC_CLK_TCK")
    for (role, phase), observations in grouped.items():
        first, last = observations[0], observations[-1]
        elapsed = (last["ns"] - first["ns"]) / 1e9
        cpu = (last["user_ticks"] + last["system_ticks"] -
               first["user_ticks"] - first["system_ticks"]) / ticks
        summaries.append({"role": role, "phase": phase, "samples": len(observations),
                          "elapsed_seconds": elapsed,
                          "mean_cpu_percent": 100 * cpu / elapsed if elapsed else None,
                          "max_rss_kib": max(item["rss_kib"] for item in observations),
                          "runqueue_wait_ms": (last["runqueue_wait_ns"] -
                                               first["runqueue_wait_ns"]) / 1e6})
    return summaries


def input_processing(sent, ledger, pane, enabled):
    if not enabled:
        return {"measured": False}
    consumed = [row["sequence"] for row in ledger if row.get("pane") == pane and "sequence" in row]
    counts = Counter(consumed)
    expected = {row["sequence"] for row in sent}
    return {"measured": True, "sent": len(sent), "consumed": len(consumed),
            "not_consumed_by_cleanup": sorted(expected - counts.keys()),
            "unexpected_sequences": sorted(counts.keys() - expected),
            "duplicate_consumption": sum(max(0, count - 1) for count in counts.values()),
            "order_regressions": sum(later < earlier for earlier, later in zip(consumed, consumed[1:])),
            "malformed_input_records": sum(bool(row.get("malformed_input")) for row in ledger)}


def subscription_groups(panes, count, connections):
    if count == 0:
        return []
    groups = [[] for _ in range(min(count, connections))]
    for index in range(count):
        groups[index % len(groups)].append({"type": "pane.agent_status_changed",
                                          "pane_id": panes[index % len(panes)]})
    return groups


def run_case(args):
    if sys.platform != "linux" or not args.remote_isolated:
        raise RuntimeError("run requires Linux and --remote-isolated on the assigned remote lane")
    args.binary = args.binary.resolve(strict=True)
    if not os.access(args.binary, os.X_OK):
        raise ValueError("explicit binary is not executable")
    if args.raw_drain and args.mode == "direct":
        raise ValueError("raw-drain overhead control requires a TUI")
    if max(args.cols, args.rows) > 65535 or args.input_hz > 1e9 or (
            math.ceil(args.seconds * args.input_hz) > 99999999):
        raise ValueError("geometry, clock resolution, or sequence exceeds the harness encoding")
    if args.mode != "direct" and not args.raw_drain:
        import pyte
        del pyte
    case = Case(args)
    def stop(_signum, _frame):
        case.stop.set()
        case.visible_changed.set()
        # Interrupt a main-thread blocking API read; only owned peers are closed.
        for peer in list(case.peers):
            try:
                peer.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    try:
        case.run()
    except BaseException as error:
        case.errors.append({"run": repr(error)})
    finally:
        try:
            case.cleanup()
        except BaseException as error:
            case.errors.append({"cleanup": repr(error), "state_retained": str(case.root)})
        case.save()
    print(case.out / "report.json")
    return 1 if case.errors else 0


def matrix(_args):
    for panes, subscribers, mode, geometry, workload in itertools.product(
            (1, 15, 174), (0, 15, 121), ("direct", "tui", "mixed"),
            ((120, 40), (483, 128)), ("idle", "hidden", "visible")):
        print(json.dumps({"panes": panes, "subscriptions": subscribers,
                          "connections": 1, "mode": mode, "cols": geometry[0],
                          "rows": geometry[1], "workload": workload}))
    return 0


def compare(args):
    reports = [json.loads(path.read_text()) for path in args.reports]
    keys = ("panes", "subscriptions", "connections", "mode", "cols", "rows", "workload",
            "seconds", "input_hz", "read_hz", "output_hz", "api_burst_clients",
            "slow_reader_hz", "raw_drain", "no_audit", "no_resource_sampling", "scroll_samples")
    groups = {}
    for report in reports:
        key = tuple(report["parameters"][field] for field in keys)
        groups.setdefault(key, []).append(report)
    # Keep distributions per run; do not silently average percentiles or combine censored samples.
    for key, runs in groups.items():
        print(json.dumps({"parameters": dict(zip(keys, key)), "runs": [
            {"variant": run["variant"], "round": run["round"], "sha256": run["sha256"],
             "measurements": run["measurements"], "errors": run["errors"],
             "cleanup": run["cleanup"]} for run in runs]}))
    return 0


def positive(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be finite and positive")
    return number


def nonnegative(value):
    number = int(value)
    if number < 0:
        raise argparse.ArgumentTypeError("must be nonnegative")
    return number


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    sub = result.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    run.add_argument("--binary", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--variant", required=True)
    run.add_argument("--source-revision", required=True)
    run.add_argument("--round", type=positive_int, default=1)
    run.add_argument("--remote-isolated", action="store_true")
    run.add_argument("--panes", type=positive_int, choices=(1, 15, 174), required=True)
    run.add_argument("--subscriptions", type=nonnegative, required=True)
    run.add_argument("--connections", type=positive_int, default=1)
    run.add_argument("--mode", choices=("direct", "tui", "mixed"), required=True)
    run.add_argument("--cols", type=positive_int, default=120)
    run.add_argument("--rows", type=positive_int, default=40)
    run.add_argument("--workload", choices=("idle", "hidden", "visible"), required=True)
    run.add_argument("--seconds", type=positive, default=60)
    run.add_argument("--input-hz", type=positive, required=True)
    run.add_argument("--read-hz", type=positive, required=True)
    run.add_argument("--output-hz", type=positive, required=True)
    run.add_argument("--api-burst-clients", type=nonnegative, default=0)
    run.add_argument("--slow-reader-hz", type=positive, default=None)
    run.add_argument("--scroll-samples", type=nonnegative, default=20)
    run.add_argument("--raw-drain", action="store_true", help="observer overhead control; no TUI latency")
    run.add_argument("--no-audit", action="store_true", help="child receipt overhead control")
    run.add_argument("--no-resource-sampling", action="store_true", help="resource observer overhead control")
    run.set_defaults(func=run_case)
    sub.add_parser("producer").set_defaults(func=lambda _args: producer() or 0)
    sub.add_parser("matrix").set_defaults(func=matrix)
    comparison = sub.add_parser("compare")
    comparison.add_argument("reports", type=Path, nargs="+")
    comparison.set_defaults(func=compare)
    return result


if __name__ == "__main__":
    arguments = parser().parse_args()
    sys.exit(arguments.func(arguments))
