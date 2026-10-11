#!/usr/bin/env python3
"""A synthetic PTY consumer. No provider CLI, transcript or native session ID."""
import json
import os
from pathlib import Path
import signal
import sys

folder = Path(sys.argv[1])
folder.mkdir(exist_ok=True)
(folder / "pid.json").write_text(json.dumps({"pid": os.getpid(),
    "start_ticks": Path(f"/proc/{os.getpid()}/stat").read_text().split(") ", 1)[1].split()[19]}))
signal.signal(signal.SIGHUP, lambda *_: (folder / "hup").write_text("HUP\n"))
print("READY", flush=True)
with (folder / "received").open("a", buffering=1) as log:
    for line in sys.stdin:
        log.write(line)
        print("got:" + line.rstrip("\n"), flush=True)
