#!/usr/bin/env python3
"""Remote Linux-only old-to-candidate handoff with 174 real synthetic PTYs.

Uses the same public API, marker and process checks as tests/live_handoff.rs.
Every run requires a fresh directory, with no production snapshot or provider home.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import struct
import subprocess
import sys

PANES = 174


def wait_marker(path, predicate):
    # Wait on filesystem events rather than inserting a timing assumption.
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), "inotify_init1")
    try:
        # IN_MODIFY matters: the stand-in flushes but keeps its receipt log open.
        if libc.inotify_add_watch(fd, os.fsencode(path.parent), 0x2 | 0x100 | 0x8 | 0x80) < 0:
            raise OSError(ctypes.get_errno(), "inotify_add_watch")
        while True:
            if path.exists():
                text = path.read_text()
                if predicate(text):
                    return text
            os.read(fd, 65536)
    finally:
        os.close(fd)


def request(path, method, params=None, allow_error=False):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.connect(str(path))
        stream.sendall(json.dumps({"id": method, "method": method,
                                  "params": params or {}}).encode() + b"\n")
        with stream.makefile("rb") as reader:
            response = json.loads(reader.readline())
    if not allow_error and "error" in response:
        raise AssertionError(response)
    return response


def peer_pid(path):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.connect(str(path))
        return struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]


def process_identity(pid):
    # Start ticks protect these checks from unrelated PID reuse.
    return Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()[19]


def pane_identity(result):
    panes = result["result"]["panes"]
    return sorted((p["pane_id"], p["tab_id"], p["workspace_id"], p["terminal_id"])
                  for p in panes)


def assert_output_markers(text, tokens, pane):
    lines = [line.strip() for line in text.splitlines()]
    markers = ["got:" + token for token in tokens]
    for marker in markers:
        assert lines.count(marker) == 1, f"pane {pane}: missing or duplicated output {marker!r}"
    positions = [lines.index(marker) for marker in markers]
    assert positions == sorted(positions), f"pane {pane}: output reordered"


def verify_pane_output(api, pane, tokens):
    # The supported wait API observes parsed terminal output, with no client-side
    # delay or deadline. Waiting for the final line establishes an output barrier;
    # pane.read must then contain every unique line, including the earlier ones.
    request(api, "pane.wait_for_output", {
        "pane_id": pane, "source": "recent_unwrapped", "lines": 100,
        "match": {"type": "substring", "value": "got:" + tokens[-1]},
        "strip_ansi": True,
    })
    read = request(api, "pane.read", {
        "pane_id": pane, "source": "recent_unwrapped", "lines": 100,
        "format": "text", "strip_ansi": True,
    })["result"]["read"]
    assert read["pane_id"] == pane, "output read resolved a different pane"
    assert_output_markers(read["text"], tokens, pane)
    return read


def client_reconnect(path):
    # Frozen endpoint generation 1 framing, copied from tests/support/mod.rs.
    def varint(number):
        if number <= 250:
            return bytes([number])
        if number <= 65535:
            return b"\xfb" + struct.pack("<H", number)
        return b"\xfc" + struct.pack("<I", number)

    def string(value):
        value = value.encode()
        return varint(len(value)) + value

    def read_number(payload, offset):
        tag = payload[offset]
        if tag <= 250:
            return tag, offset + 1
        width, fmt = {251: (2, "<H"), 252: (4, "<I")}[tag]
        return struct.unpack_from(fmt, payload, offset + 1)[0], offset + 1 + width

    def read_string(payload, offset):
        length, offset = read_number(payload, offset)
        return payload[offset:offset + length].decode(), offset + length

    def exact(stream, count):
        result = b""
        while len(result) < count:
            chunk = stream.recv(count - len(result))
            if not chunk:
                raise EOFError("client disconnected during handshake")
            result += chunk
        return result

    hello = {"generation": 1, "cell_width_px": 8, "cell_height_px": 16,
             "surface_size": {"cols": 483, "rows": 128}, "pixel_mouse": False,
             "direct_graphics": False, "endpoint_keybindings": False, "mouse_capture": False,
             "snapshot_codecs": ["shell.snapshot.v1"], "surface_codecs": ["shell.surface.v1"],
             "input_codecs": ["shell.input.semantic.v1"], "blob_codecs": ["shell.blob.v1"]}
    payload = varint(20) + string("endpoint.hello.v1") + string(json.dumps(hello))
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
        client.sendall(struct.pack("<I", len(payload)) + payload)
        length = struct.unpack("<I", exact(client, 4))[0]
        assert 0 < length <= 2 * 1024 * 1024
        response = exact(client, length)
        variant, offset = read_number(response, 0)
        assert variant == 20
        kind, offset = read_string(response, offset)
        assert kind == "endpoint.welcome.v1"
        text, _ = read_string(response, offset)
        welcome = json.loads(text)
        assert welcome["generation"] == 1 and not welcome.get("error"), welcome


def run(old, candidate, root, failure, suppress_output_pane=None):
    if suppress_output_pane is not None and not 0 <= suppress_output_pane < PANES:
        raise ValueError("suppressed-output pane index must be within the synthetic pane set")
    root = root.absolute()
    for path in [root, *root.parents]:
        if path.is_symlink():
            raise ValueError("rehearsal root must not traverse symlinks")
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    config = root / "c"
    data = config / "herdr/sessions/fast"
    if len(os.fsencode(data / "herdr-handoff-2147483647.sock")) > 107:
        raise ValueError("use a shorter rehearsal root for Unix socket paths")
    data.mkdir(parents=True)
    (config / "herdr/config.toml").write_text(
        "onboarding = false\n[update]\nversion_check = false\nmanifest_check = false\n")
    for name in ("h", "s", "r", "d", "k"):
        (root / name).mkdir()
    env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "h"), "SHELL": "/bin/sh",
           "TERM": "xterm-256color", "XDG_CONFIG_HOME": str(config),
           "XDG_STATE_HOME": str(root / "s"), "XDG_RUNTIME_DIR": str(root / "r"),
           "XDG_DATA_HOME": str(root / "d"), "XDG_CACHE_HOME": str(root / "k"),
           "HERDR_SESSION": "fast"}
    # after_restored is an existing supported internal test injection, not a new protocol.
    if failure == "after-restored":
        env["HERDR_TEST_HANDOFF_IMPORT_FAIL"] = "after_restored"
    api = data / "herdr.sock"
    server_pid = None
    candidate_killed = False
    with (root / "server.log").open("wb") as log:
        server = subprocess.Popen([str(old), "--session", "fast", "server"], env=env,
                                  stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    try:
        wait_socket(api)
        server_pid = peer_pid(api)
        before_server = process_identity(server_pid)
        created = request(api, "workspace.create", {"cwd": str(root), "focus": True})["result"]
        workspace = created["workspace"]["workspace_id"]
        panes = [created["root_pane"]["pane_id"]]
        for _ in range(1, PANES):
            panes.append(request(api, "tab.create", {"workspace_id": workspace,
                          "cwd": str(root), "focus": False})["result"]["root_pane"]["pane_id"])
        children = []
        standin = Path(__file__).with_name("standin.py").resolve()
        for index, pane in enumerate(panes):
            folder = root / f"p{index}"
            folder.mkdir()
            command_args = [sys.executable, str(standin), str(folder)]
            if index == suppress_output_pane:
                # The receipt is still written, and the final output line still
                # establishes the barrier. A successful rehearsal must reject this.
                command_args += ["--suppress-output", f"after-{index}-0"]
            command = shlex.join(command_args)
            request(api, "pane.send_input", {"pane_id": pane, "text": command, "keys": ["Enter"]})
            text = wait_marker(folder / "pid.json", lambda s: s.endswith("}"))
            children.append(json.loads(text))
        before = pane_identity(request(api, "pane.list"))
        assert len(before) == PANES
        client_reconnect(data / "herdr-client.sock")
        output_reads = []
        for phase in ("before", "after"):
            if phase == "after":
                params = {"import_exe": str(candidate)}
                if failure == "protocol":
                    params["expected_protocol"] = 999999
                elif failure == "missing-executable":
                    params["import_exe"] = str(root / "absent")
                response = request(api, "server.live_handoff", params, allow_error=True)
                (root / "handoff-response.json").write_text(json.dumps(response, indent=2))
                assert ("error" in response) == (failure not in {"none", "post-commit"}), response
                wait_socket(api)
                current = peer_pid(api)
                if failure in {"none", "post-commit"}:
                    assert current != server_pid, "candidate did not take socket ownership"
                    assert Path(f"/proc/{current}/exe").resolve() == candidate
                else:
                    assert current == server_pid and process_identity(current) == before_server
                assert pane_identity(request(api, "pane.list")) == before
                client_reconnect(data / "herdr-client.sock")
            for index, (pane, child) in enumerate(zip(panes, children)):
                assert process_identity(child["pid"]) == child["start_ticks"]
                assert not (root / f"p{index}/hup").exists()
                tokens = [f"{phase}-{index}-{n}" for n in range(3)]
                request(api, "pane.send_input", {"pane_id": pane,
                        "text": "\n".join(tokens) + "\n"})
                received = root / f"p{index}/received"
                wait_marker(received, lambda s: tokens[-1] + "\n" in s)
                expected = [f"before-{index}-{n}" for n in range(3)]
                if phase == "after":
                    expected += tokens
                assert received.read_text().splitlines() == expected, "input lost, reordered or duplicated"
                output_reads.append({"phase": phase, "pane_id": pane,
                                     "read": verify_pane_output(api, pane, tokens)})
        (root / "terminal-output.json").write_text(json.dumps(output_reads, indent=2))
        if failure == "post-commit":
            victim = peer_pid(api)
            assert Path(f"/proc/{victim}/exe").resolve() == candidate
            victim_ticks = process_identity(victim)
            assert process_identity(victim) == victim_ticks
            # This is the fresh rehearsal's candidate, never a discovered live PID.
            os.kill(victim, signal.SIGKILL)
            candidate_killed = True
            # pidfd reports exit without sleeping or guessing how long it takes.
            import select
            # If already reaped, /proc absence itself records exit.
            try:
                fd = os.pidfd_open(victim)
            except ProcessLookupError:
                fd = None
            if fd is not None:
                try:
                    select.select([fd], [], [])
                finally:
                    os.close(fd)
            observations = []
            for index, child in enumerate(children):
                try:
                    survives = process_identity(child["pid"]) == child["start_ticks"]
                except FileNotFoundError:
                    survives = False
                observations.append({"pid": child["pid"], "present_at_observation": survives,
                                     "hup_at_observation": (root / f"p{index}/hup").exists()})
            (root / "post-commit.json").write_text(json.dumps({"killed_candidate": victim,
                "old_source_exited": server.poll() is not None, "children": observations,
                "recovery": "not attempted; binary rollback does not restore lost PTYs"}, indent=2))
        (root / "result.json").write_text(json.dumps({"panes": PANES, "failure": failure,
            "old_server_pid": server_pid, "new_server_pid": current,
            "identities": before, "pty_children": children, "result": "passed"}, indent=2))
    finally:
        # Stop only the owned fresh socket. Never discover or stop any other server.
        if api.exists() and not candidate_killed:
            request(api, "server.stop")
        server.wait()


def wait_socket(path):
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), "inotify_init1")
    try:
        if libc.inotify_add_watch(fd, os.fsencode(path.parent), 0x100 | 0x80) < 0:
            raise OSError(ctypes.get_errno(), "inotify_add_watch")
        while not path.exists():
            os.read(fd, 65536)
    finally:
        os.close(fd)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--failure", choices=["none", "protocol", "missing-executable", "after-restored", "post-commit"], default="none")
    parser.add_argument("--suppress-output-pane", type=int,
                        help="negative control: omit one post-handoff got: line; must fail")
    args = parser.parse_args()
    run(args.old.resolve(strict=True), args.candidate.resolve(strict=True), args.root,
        args.failure, args.suppress_output_pane)
