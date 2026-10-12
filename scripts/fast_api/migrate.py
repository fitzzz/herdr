#!/usr/bin/env python3
"""Explicit one-shot Linux migration from the pinned unmodified old server.

This is separate from the isolated fast-API launcher. It retains inherited
session/provider paths and never starts, discovers or retries a source server.
Terminal identity is preserved; completion history crosses a fresh epoch.
Use only at an authorized boundary with consumers prepared for that epoch.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import stat
import struct
import sys
import tempfile
import uuid

MAX_JSON = 32 * 1024 * 1024


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON object key: " + key)
        result[key] = value
    return result


def decode(data):
    return json.loads(data, object_pairs_hook=unique_object)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"


def sha(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def file_stamp(metadata):
    return {"device": metadata.st_dev, "inode": metadata.st_ino, "uid": metadata.st_uid}


def socket_stamp(path):
    metadata = os.lstat(path)
    if not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise ValueError("source socket type or owner differs")
    return file_stamp(metadata)


def process_stamp(pid):
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        raise ValueError("source process PID is absent")
    base = Path("/proc") / str(pid)

    def start():
        fields = (base / "stat").read_text().rsplit(")", 1)[1].split()
        if fields[0] in ("Z", "X", "x"):
            raise ValueError("source process has exited")
        return int(fields[19])

    ticks = start()
    result = {"pid": pid, "uid": os.stat(base).st_uid, "start_ticks": ticks,
              "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
              "pid_namespace": os.readlink(base / "ns/pid"),
              "mount_namespace": os.readlink(base / "ns/mnt")}
    if start() != ticks:
        raise ValueError("source process identity changed")
    return result


def source_context(pid):
    wanted = {"HOME", "XDG_CONFIG_HOME", "HERDR_CONFIG_PATH", "HERDR_SESSION",
              "HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH"}
    selectors = {}
    for entry in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0"):
        key, separator, value = entry.partition(b"=")
        if separator and key.decode(errors="replace") in wanted:
            selectors[key.decode()] = os.fsdecode(value)
    home = selectors.get("HOME")
    config_base = selectors.get("XDG_CONFIG_HOME") or (str(Path(home) / ".config") if home else "/tmp")
    config_dir = Path(config_base) / "herdr"
    session = selectors.get("HERDR_SESSION") or None
    # /proc/environ is the launch environment. Herdr can select a session from
    # argv after launch; reconstruct that documented override explicitly.
    arguments = [os.fsdecode(value) for value in Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0") if value]
    explicit_session = False
    for index, argument in enumerate(arguments):
        if argument == "--":
            break
        if argument == "--session" and index + 1 < len(arguments):
            session, explicit_session = arguments[index + 1], True
        elif argument.startswith("--session="):
            session, explicit_session = argument.split("=", 1)[1], True
    if session == "default":
        session = None
    data_dir = config_dir / "sessions" / session if session else config_dir
    api = str(data_dir / "herdr.sock") if explicit_session else selectors.get("HERDR_SOCKET_PATH") or str(data_dir / "herdr.sock")
    if explicit_session:
        client = str(data_dir / "herdr-client.sock")
    elif selectors.get("HERDR_SOCKET_PATH"):
        api_path = Path(api)
        client = str(api_path.with_name(api_path.stem + "-client.sock"))
    else:
        client = selectors.get("HERDR_CLIENT_SOCKET_PATH") or str(data_dir / "herdr-client.sock")
    return {"session": session, "config_path": str(selectors.get("HERDR_CONFIG_PATH") or config_dir / "config.toml"),
            "data_dir": str(data_dir),
            "api_socket": api, "client_socket": client}


class SourceAPI:
    def __init__(self, path, source_sha256, expected_context, timeout=None):
        self.path = str(path)
        self.expected_hash = source_sha256
        self.context = expected_context
        self.timeout = timeout
        self.source = None
        self.socket_identity = socket_stamp(path)
        self.counter = 0

    def authenticate(self, stream):
        pid, uid, _gid = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        identity = process_stamp(pid)
        own = process_stamp(os.getpid())
        if uid != os.geteuid() or identity["uid"] != uid or socket_stamp(self.path) != self.socket_identity:
            raise ValueError("source socket/peer ownership differs")
        if any(identity[key] != own[key] for key in ("boot_id", "pid_namespace", "mount_namespace")):
            raise ValueError("source namespace differs")
        if sha(f"/proc/{pid}/exe") != self.expected_hash or source_context(pid) != self.context:
            raise ValueError("source artifact/session context differs")
        if self.source is None:
            self.source = identity
        if identity != self.source or process_stamp(pid) != identity:
            raise ValueError("source process identity changed")

    def request(self, method, params, sent=None):
        self.counter += 1
        request_id = f"migration:{self.counter}"
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            stream.settimeout(self.timeout)
            stream.connect(self.path)
            self.authenticate(stream)
            payload = encoded({"id": request_id, "method": method, "params": params})
            # Record dispatch before sendall: an interrupted write has unknown
            # outcome and must never be retried with this one-use attempt.
            if sent:
                sent()
            stream.sendall(payload)
            with stream.makefile("rb") as reader:
                line = reader.readline(MAX_JSON + 1)
            if not line or len(line) > MAX_JSON or not line.endswith(b"\n"):
                raise ValueError("source response is missing, oversized or incomplete")
            response = decode(line)
            if response.get("id") != request_id or "error" in response or "result" not in response:
                raise ValueError("source API rejected request: " + json.dumps(response))
            return response["result"]


def identity_table(panes, agents):
    names = {}
    for agent in agents:
        terminal = agent["terminal_id"]
        if terminal in names or agent.get("launch_pending"):
            raise ValueError("agent capture is duplicated or launch-pending")
        names[terminal] = agent
    result = {}
    terminals = set()
    for pane in panes:
        address, terminal = pane["pane_id"], pane["terminal_id"]
        if not address or not terminal or address in result or terminal in terminals or pane.get("restore_error"):
            raise ValueError("pane capture is incomplete, duplicated or unavailable")
        agent = names.pop(terminal, None)
        if agent is not None and any(agent.get(key) != pane.get(key) for key in ("pane_id", "workspace_id", "tab_id", "agent_session")):
            raise ValueError("pane/agent identity association differs")
        result[address] = {key: pane[key] for key in ("pane_id", "terminal_id", "workspace_id", "tab_id")}
        result[address].update(agent_name=agent.get("name") if agent else None,
                               agent_session=pane.get("agent_session"))
        terminals.add(terminal)
    if names:
        raise ValueError("unaccounted agents in capture")
    return result


def membership(snapshot):
    workspaces = [workspace["workspace_id"] for workspace in snapshot["workspaces"]]
    tabs = {tab["tab_id"]: tab["workspace_id"] for tab in snapshot["tabs"]}
    if len(set(workspaces)) != len(workspaces) or len(tabs) != len(snapshot["tabs"]):
        raise ValueError("workspace/tab capture is duplicated")
    layouts = {}
    leaves = set()
    for layout in snapshot["layouts"]:
        tab = layout["tab_id"]
        ids = [pane["pane_id"] for pane in layout["panes"]]
        if tab in layouts or len(set(ids)) != len(ids) or leaves.intersection(ids) or tabs.get(tab) != layout["workspace_id"]:
            raise ValueError("layout capture is incomplete or duplicated")
        layouts[tab] = sorted(ids)
        leaves.update(ids)
    if set(layouts) != set(tabs) or leaves != {pane["pane_id"] for pane in snapshot["panes"]}:
        raise ValueError("layout/pane capture coverage differs")
    for pane in snapshot["panes"]:
        if tabs.get(pane["tab_id"]) != pane["workspace_id"] or pane["workspace_id"] not in workspaces or pane["pane_id"] not in layouts[pane["tab_id"]]:
            raise ValueError("pane membership capture differs")
    return {"workspaces": sorted(workspaces), "tabs": tabs, "layouts": layouts}


def capture(api):
    ping = api.request("ping", {})
    pane_a = api.request("pane.list", {})["panes"]
    snap_a = api.request("session.snapshot", {})["snapshot"]
    table = identity_table(pane_a, snap_a["agents"])
    if table != identity_table(snap_a["panes"], snap_a["agents"]):
        raise ValueError("initial pane/snapshot capture differs")
    children = set()
    for address, pane in table.items():
        info = api.request("pane.process_info", {"pane_id": address})["process_info"]
        if info["pane_id"] != address:
            raise ValueError("process capture address differs")
        child = process_stamp(info.get("shell_pid"))
        if child["pid"] in children or child["uid"] != api.source["uid"]:
            raise ValueError("shell process capture is duplicated or foreign")
        pane["child"] = child
        children.add(child["pid"])
    snap_b = api.request("session.snapshot", {})["snapshot"]
    pane_b = api.request("pane.list", {})["panes"]
    bare_table = {key: {name: value for name, value in pane.items() if name != "child"} for key, pane in table.items()}
    if bare_table != identity_table(pane_b, snap_b["agents"]) or bare_table != identity_table(snap_b["panes"], snap_b["agents"]) or membership(snap_a) != membership(snap_b):
        raise ValueError("identity changed while bracketing capture")
    for pane in table.values():
        if process_stamp(pane["child"]["pid"]) != pane["child"]:
            raise ValueError("shell process changed after capture")
    return ping, list(table.values()), snap_b["agents"]


def exclusive_write(directory_fd, name, data, mode=0o400):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 mode, dir_fd=directory_fd)
    with os.fdopen(fd, "wb") as target:
        target.write(data)
        target.flush()
        os.fsync(target.fileno())


def checked_open(directory_fd, name):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory_fd)
    metadata = os.fstat(fd)
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o400:
        os.close(fd)
        raise ValueError("attempt file type/owner/mode differs")
    return fd


def read_fd(fd):
    with os.fdopen(os.dup(fd), "rb") as source:
        data = source.read(MAX_JSON + 1)
    # dup shares the open-file offset; rewind the original for the importer.
    os.lseek(fd, 0, os.SEEK_SET)
    if len(data) > MAX_JSON:
        raise ValueError("attempt file oversized")
    return data


def shim_main(attempt_path, receipt_hash, candidate, candidate_hash, operation_id, arguments):
    directory_fd = os.open(attempt_path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    metadata = os.fstat(directory_fd)
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise ValueError("attempt directory owner/mode differs")
    try:
        if len(arguments) != 4 or arguments[:2] != ["server", "--handoff-import"]:
            raise ValueError("unexpected handoff shim arguments")
        receipt_fd = checked_open(directory_fd, "receipt.json")
        data = read_fd(receipt_fd)
        receipt = decode(data)
        source = receipt["source"]
        if hashlib.sha256(data).hexdigest() != receipt_hash or receipt["operation_id"] != operation_id or receipt["candidate_sha256"] != candidate_hash:
            raise ValueError("attempt receipt digest/binding differs")
        if process_stamp(os.getppid()) != source or sha(f"/proc/{source['pid']}/exe") != receipt["source_sha256"]:
            raise ValueError("shim was not invoked by captured exporter")
        handoff_socket, token = arguments[2:]
        if handoff_socket != str(Path(receipt["data_dir"]) / f"herdr-handoff-{source['pid']}.sock") or not token or sha(candidate) != candidate_hash:
            raise ValueError("shim socket/token/candidate differs")
        claim = {"operation_id": operation_id, "receipt_sha256": receipt_hash,
                 "receipt_stamp": file_stamp(os.fstat(receipt_fd)), "directory_stamp": file_stamp(metadata),
                 "importer": process_stamp(os.getpid()), "source": source, "candidate_sha256": candidate_hash,
                 "handoff_socket": handoff_socket, "handoff_socket_stamp": socket_stamp(handoff_socket), "token": token}
        exclusive_write(directory_fd, "claimed.json", encoded(claim))
        claim_fd = checked_open(directory_fd, "claimed.json")
        for fd in (receipt_fd, claim_fd, directory_fd):
            os.set_inheritable(fd, True)
        os.execve(candidate, [candidate, *arguments, "--handoff-identity-fds", str(receipt_fd), str(claim_fd), str(directory_fd)], os.environ.copy())
    except Exception as error:
        exclusive_write(directory_fd, "shim-error.json", encoded({"operation_id": operation_id,
                         "phase": "shim", "error": str(error), "ownership_transfer": "precommit",
                         "handoff_request_sent": True, "clients_disconnected": True}))
        raise


def make_attempt(root, receipt, candidate):
    operation_id = receipt["operation_id"]
    attempt = Path(tempfile.mkdtemp(prefix="migration-" + operation_id + "-", dir=root))
    directory_fd = os.open(attempt, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        receipt_bytes = encoded(receipt)
        receipt_hash = hashlib.sha256(receipt_bytes).hexdigest()
        exclusive_write(directory_fd, "receipt.json", receipt_bytes)
        source = Path(__file__).read_text()
        entry = f"shim_main({str(attempt)!r}, {receipt_hash!r}, {str(candidate)!r}, {receipt['candidate_sha256']!r}, {operation_id!r}, sys.argv[1:])"
        source = source.replace('if __name__ == "__main__":\n    main()', 'if __name__ == "__main__":\n    ' + entry)
        # Freeze the complete shim source, not an import of a mutable helper.
        source = source.replace("#!/usr/bin/env python3", "#!" + sys.executable, 1)
        exclusive_write(directory_fd, "import-shim", source.encode(), 0o500)
        exclusive_write(directory_fd, "dispatch.json", encoded({"operation_id": operation_id,
                         "receipt_sha256": receipt_hash, "shim_sha256": sha(attempt / "import-shim"),
                         "candidate_sha256": receipt["candidate_sha256"]}))
    finally:
        os.close(directory_fd)
    return attempt


def migrate(options):
    if sys.platform != "linux":
        raise ValueError("explicit terminal identity migration requires Linux")
    candidate = options.candidate.resolve(strict=True)
    if sha(candidate) != options.candidate_sha256:
        raise ValueError("candidate artifact differs")
    context = {"session": options.session, "config_path": str(options.config_path),
               "data_dir": str(options.data_dir), "api_socket": str(options.api_socket),
               "client_socket": str(options.client_socket)}
    api = SourceAPI(options.api_socket, options.source_sha256, context, options.request_timeout)
    ping, panes, agents = capture(api)
    if (ping["version"], ping["protocol"]) != (options.source_version, options.source_protocol):
        raise ValueError("source version/protocol differs")
    operation_id = uuid.uuid4().hex
    receipt = dict(version=1, operation_id=operation_id, observation_epoch=operation_id,
                   source=api.source, source_sha256=options.source_sha256,
                   source_version=ping["version"], source_protocol=ping["protocol"],
                   candidate_sha256=options.candidate_sha256, candidate_version=options.candidate_version,
                   candidate_protocol=options.candidate_protocol, **context,
                   data_dir_stamp=file_stamp(os.stat(options.data_dir)),
                   api_socket_stamp=socket_stamp(options.api_socket),
                   client_socket_stamp=socket_stamp(options.client_socket), panes=panes, captured_agents=agents)
    attempt = make_attempt(options.attempt_root, receipt, candidate)
    directory_fd = os.open(attempt, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        def sent():
            exclusive_write(directory_fd, "request-sent.json", encoded({"operation_id": operation_id,
                            "handoff_request_sent": True, "ownership_transfer": "unknown"}))
        print(json.dumps({"operation_id": operation_id, "attempt": str(attempt)}), flush=True)
        response = api.request("server.live_handoff", {"import_exe": str(attempt / "import-shim"),
                               "expected_version": options.candidate_version,
                               "expected_protocol": options.candidate_protocol}, sent=sent)
        exclusive_write(directory_fd, "result.json", encoded({"operation_id": operation_id, "response": response,
                        "handoff_request_sent": True, "ownership_transfer": "reported_success"}))
        return attempt
    except Exception as error:
        exclusive_write(directory_fd, "coordinator-error.json", encoded({"operation_id": operation_id,
                         "error": str(error), "handoff_request_sent": (attempt / "request-sent.json").exists(),
                         "ownership_transfer": "unknown"}))
        raise
    finally:
        os.close(directory_fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("api-socket", "client-socket", "data-dir", "config-path", "candidate", "attempt-root"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("source-sha256", "source-version", "candidate-sha256", "candidate-version"):
        parser.add_argument("--" + name, required=True)
    for name in ("source-protocol", "candidate-protocol"):
        parser.add_argument("--" + name, type=int, required=True)
    parser.add_argument("--session")
    parser.add_argument("--request-timeout", type=float, help="Optional operator-supplied request limit; no default timeout")
    options = parser.parse_args()
    for name in ("api_socket", "client_socket", "data_dir", "config_path", "candidate", "attempt_root"):
        if not getattr(options, name).is_absolute():
            parser.error(name + " must be an explicit absolute path")
    migrate(options)


if __name__ == "__main__":
    main()
