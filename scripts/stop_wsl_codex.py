import argparse
import json
import os
import re
import select
import signal
import sys
import time
from pathlib import Path

def trusted_executable(executable, auth_home):
    name = Path(executable).name.removesuffix(" (deleted)")
    return name == "codex" or (bool(re.fullmatch(r"codex-[a-z0-9_-]+", name)) and not name.startswith("codex-switcher"))


def inspect_process(pid):
    directory = Path("/proc") / str(pid)
    fields = (directory / "stat").read_text().rsplit(")", 1)[1].split()
    return {
        "pid": pid,
        "name": (directory / "comm").read_text().strip(),
        "parent": int(fields[1]),
        "start": fields[19],
        "uid": directory.stat().st_uid,
        "exe": os.readlink(directory / "exe"),
    }


def all_processes():
    result = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            record = inspect_process(int(entry.name))
            result[record["pid"]] = record
        except (FileNotFoundError, ProcessLookupError):
            continue
        except PermissionError:
            name = (entry / "comm").read_text().strip()
            if is_codex({"name": name}):
                raise RuntimeError("Cannot inspect a Codex process")
    return result


def is_codex(record):
    return record["name"] == "codex" or (record["name"].startswith("codex-") and not record["name"].startswith("codex-switcher"))


def belongs_to(pid, roots, processes):
    seen = set()
    while pid in processes and pid not in seen:
        if pid in roots:
            return True
        seen.add(pid)
        pid = processes[pid]["parent"]
    return False


def snapshot_targets(auth_home):
    processes = all_processes()
    targets = {}
    for pid, record in processes.items():
        if not is_codex(record) or record["uid"] != os.getuid():
            continue
        environment = (Path("/proc") / str(pid) / "environ").read_bytes().split(b"\0")
        homes = [value[len(b"CODEX_HOME="):].decode() for value in environment if value.startswith(b"CODEX_HOME=")]
        user_homes = [value[len(b"HOME="):].decode() for value in environment if value.startswith(b"HOME=")]
        process_home = homes[0] if homes else str(Path(user_homes[0]) / ".codex") if user_homes else str(Path.home() / ".codex")
        if os.path.realpath(process_home) != auth_home:
            continue
        if not trusted_executable(record["exe"], auth_home):
            raise RuntimeError("Unrecognized executable for the selected Codex home")
        targets[pid] = record
    roots = set()
    for record in targets.values():
        parent = processes.get(record["parent"])
        if parent and Path(parent["exe"]).name in ("ChatGPT", "Codex", "codex-desktop") and (Path(parent["exe"]).parent / "resources/codex").is_file() and parent["uid"] == os.getuid():
            roots.add(parent["pid"])
    for pid, record in processes.items():
        if any(record["exe"] == processes[root]["exe"] for root in roots) and record["uid"] == os.getuid() and belongs_to(pid, roots, processes):
            targets[pid] = record
    return targets, roots


def brief(record):
    return {key: record[key] for key in ("pid", "name", "parent")}


def exited(handle):
    poller = select.poll()
    poller.register(handle, select.POLLIN)
    return bool(poller.poll(0))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--home", required=True)
    parser.add_argument("--stop", action="store_true")
    arguments = parser.parse_args()
    if not arguments.home.startswith("/"):
        raise RuntimeError("Use an absolute Codex home")
    arguments.home = os.path.realpath(arguments.home)
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        raise RuntimeError("Safe process handles are unavailable")
    targets, roots = snapshot_targets(arguments.home)
    if not arguments.stop:
        print(json.dumps({"status": "preview", "targets": [brief(record) for record in targets.values()], "blockingProcesses": [brief(record) for record in all_processes().values() if is_codex(record)]}))
        return 0
    handles = {}
    try:
        for pid, expected in targets.items():
            try:
                handle = os.pidfd_open(pid)
            except ProcessLookupError:
                continue
            handles[pid] = handle
            try:
                current = inspect_process(pid)
            except (FileNotFoundError, ProcessLookupError):
                if exited(handle):
                    continue
                raise
            if current != expected:
                raise RuntimeError("Process identity changed before termination")
        print(json.dumps({"status": "terminating", "targets": [brief(record) for record in targets.values()]}), flush=True)
        for batch in (roots, set(handles) - roots):
            for pid in batch:
                handle = handles.get(pid)
                if handle is None or exited(handle):
                    continue
                try:
                    signal.pidfd_send_signal(handle, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            if batch == roots:
                time.sleep(1)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and any(not exited(handle) for handle in handles.values()):
            time.sleep(0.1)
        remaining = [pid for pid, handle in handles.items() if not exited(handle)]
        blockers = [brief(record) for record in all_processes().values() if is_codex(record)]
        success = not remaining and not blockers
        print(json.dumps({"status": "stopped" if success else "blocked", "remainingTargetPids": remaining, "blockingProcesses": blockers, "credentialsChanged": False}), flush=True)
        return 0 if success else 3
    finally:
        for handle in handles.values():
            os.close(handle)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(json.dumps({"status": "failed", "errorType": type(error).__name__, "credentialsChanged": False}), flush=True)
        sys.exit(1)
