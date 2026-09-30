"""Stock Codex shared-session transport and hook routing for Composer."""
import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import stat
import struct
import subprocess
import sys
import time

KEYS = (
    "HERDR_BIN_PATH", "HERDR_ENV", "HERDR_PANE_ID", "HERDR_SOCKET_PATH",
    "HERDR_STARTUP_CWD", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID",
)
SID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\Z")
MAX_INPUT = 1024 * 1024


class RPC:
    def __init__(self, endpoint, timeout=10):
        if not endpoint.startswith("unix:///"):
            raise ValueError("shared Codex requires an absolute Unix socket")
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(timeout)
        self.socket.connect(endpoint.removeprefix("unix://"))
        self.buffer = bytearray()
        self.next_id = 1
        key = base64.b64encode(os.urandom(16)).decode()
        self.socket.sendall(("GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
                             "Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
                             "Sec-WebSocket-Key: " + key + "\r\n\r\n").encode())
        while b"\r\n\r\n" not in self.buffer:
            data = self.socket.recv(65536)
            if not data:
                raise EOFError("shared Codex disconnected during handshake")
            self.buffer.extend(data)
            if len(self.buffer) > 65536:
                raise ValueError("invalid shared Codex handshake")
        end = self.buffer.index(b"\r\n\r\n")
        header = bytes(self.buffer[:end]).decode("ascii")
        del self.buffer[:end + 4]
        expected = base64.b64encode(hashlib.sha1(
            (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        fields = dict(line.split(":", 1) for line in header.split("\r\n")[1:] if ":" in line)
        fields = {key.lower(): value.strip() for key, value in fields.items()}
        if " 101 " not in header.split("\r\n")[0] or fields.get("sec-websocket-accept") != expected:
            raise ValueError("shared Codex rejected the WebSocket handshake")
        self.call("initialize", {"clientInfo": {"name": "herdr_composer", "version": "1"},
                                 "capabilities": {"experimentalApi": True}})
        self.send({"method": "initialized", "params": {}})

    def close(self):
        self.socket.close()

    def send(self, value, opcode=1):
        data = json.dumps(value).encode() if opcode == 1 else value
        size = len(data)
        header = bytes([0x80 | opcode, 0x80 | size]) if size < 126 else (
            bytes([0x80 | opcode, 0x80 | 126]) + struct.pack("!H", size) if size < 65536 else
            bytes([0x80 | opcode, 0x80 | 127]) + struct.pack("!Q", size))
        mask = os.urandom(4)
        self.socket.sendall(header + mask + bytes(v ^ mask[i % 4] for i, v in enumerate(data)))

    def take(self, size):
        while len(self.buffer) < size:
            data = self.socket.recv(65536)
            if not data:
                raise EOFError("shared Codex disconnected")
            self.buffer.extend(data)
        result = bytes(self.buffer[:size])
        del self.buffer[:size]
        return result

    def read(self):
        message = bytearray()
        while True:
            a, b = self.take(2)
            size = b & 127
            if size == 126:
                size = struct.unpack("!H", self.take(2))[0]
            elif size == 127:
                size = struct.unpack("!Q", self.take(8))[0]
            if len(message) + size > 16 * MAX_INPUT:
                raise ValueError("shared Codex response is too large")
            mask = self.take(4) if b & 128 else None
            data = self.take(size)
            if mask:
                data = bytes(v ^ mask[i % 4] for i, v in enumerate(data))
            opcode = a & 15
            if opcode == 9:
                self.send(data, 10)
                continue
            if opcode == 8:
                raise EOFError("shared Codex closed the connection")
            if opcode in (0, 1):
                message.extend(data)
                if a & 128:
                    return json.loads(message)

    def call(self, method, params):
        ident = self.next_id
        self.next_id += 1
        self.send({"id": ident, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            response = self.read()
            if response.get("id") == ident:
                if "error" in response:
                    raise RuntimeError(response["error"].get("message", "shared Codex request failed"))
                return response.get("result")
        raise TimeoutError("shared Codex request timed out")


def private_json(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_name(path.name + "." + os.urandom(8).hex() + ".tmp")
    try:
        with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as stream:
            json.dump(value, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def read_private(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError("shared Codex context must be a private file owned by this user")
    if info.st_size > MAX_INPUT:
        raise ValueError("shared Codex context is too large")
    return json.loads(path.read_bytes())


def routing(value):
    if not isinstance(value, dict) or set(value) != set(KEYS) or not all(isinstance(v, str) for v in value.values()):
        raise ValueError("shared Codex context must contain only Herdr routing fields")
    workspace = value["HERDR_WORKSPACE_ID"]
    if (value["HERDR_ENV"] != "1" or not workspace
            or not value["HERDR_PANE_ID"].startswith(workspace + ":p")
            or not value["HERDR_TAB_ID"].startswith(workspace + ":t")
            or any(not Path(value[key]).is_absolute() for key in
                   ("HERDR_SOCKET_PATH", "HERDR_BIN_PATH", "HERDR_STARTUP_CWD"))):
        raise ValueError("invalid shared Codex pane context")
    return value


def prepare(value):
    thread_id = None
    rpc = None
    try:
        directory = Path(value["contexts_dir"])
        setup = read_private(directory / "setup.json")
        if setup["endpoint"] != value["endpoint"]:
            raise ValueError("shared Codex hook setup uses a different server")
        environment = routing(value["environment"])
        rpc = RPC(value["endpoint"])
        entries = rpc.call("hooks/list", {"cwds": [value["cwd"]]})["data"]
        if any(entry.get("errors") or entry.get("warnings") for entry in entries):
            raise ValueError("shared Codex hook discovery reported errors or warnings")
        hooks = {hook["key"]: hook for entry in entries for hook in entry["hooks"]
                 if hook["handlerType"] == "command" and hook["enabled"]}
        if set(hooks) != set(setup["hooks"]):
            raise ValueError("shared Codex hooks changed; reconfigure the adapter before launching")
        for key, fingerprint in setup["hooks"].items():
            hook = hooks.get(key)
            if hook is None or hook["currentHash"] != fingerprint or hook["trustStatus"] != "trusted":
                raise ValueError("shared Codex hook configuration changed; reconfigure the adapter before launching")
        effective = rpc.call("config/read", {"cwd": value["cwd"], "includeLayers": False})["config"]
        if effective.get("features", {}).get("hooks") is not True:
            raise ValueError("shared Codex requires enabled hooks for pane routing")
        if effective.get("notify") != setup.get("notify"):
            raise ValueError("shared Codex notify changed; reconfigure the adapter before launching")
        # Dotted overrides preserve the user's other shell policy settings.
        config = {"shell_environment_policy.set." + key: val for key, val in environment.items()}
        if value.get("effort"):
            config["model_reasoning_effort"] = value["effort"]
        if value.get("speed"):
            config["service_tier"] = "fast" if value["speed"] == "fast" else "default"
        params = {"cwd": value["cwd"], "config": config}
        if value.get("model"):
            params["model"] = value["model"]
        result = rpc.call("thread/start", params)
        thread_id = result["thread"]["id"]
        if not SID.fullmatch(thread_id):
            raise ValueError("shared Codex returned an invalid session ID")
        record = {"version": 1, "session_id": thread_id, "root_session_id": thread_id,
                  "endpoint": value["endpoint"], "launch_id": value["launch_id"],
                  "environment": environment}
        private_json(directory / (thread_id + ".json"), record)
        private_json(directory / "server.json", {"endpoint": value["endpoint"]})
        # Explicit placement materializes an empty rollout without running a
        # shell command, starting inference, or injecting conversation history.
        rpc.call("thread/section/move", {"threadId": thread_id, "sectionId": None})
        rpc.call("thread/name/set", {"threadId": thread_id, "name": value["name"]})
        return {"thread_id": thread_id, "error": None}
    except Exception as error:
        return {"thread_id": thread_id, "error": str(error)}
    finally:
        if rpc:
            rpc.close()


def process_context(pid, session_id):
    if sys.platform == "darwin":
        libc = ctypes.CDLL(None, use_errno=True)
        mib = (ctypes.c_int * 3)(1, 49, pid)
        size = ctypes.c_size_t()
        if libc.sysctl(mib, 3, None, ctypes.byref(size), None, 0) != 0:
            return None
        buffer = ctypes.create_string_buffer(size.value)
        if libc.sysctl(mib, 3, buffer, ctypes.byref(size), None, 0) != 0:
            return None
        raw = buffer.raw[:size.value]
        argc = struct.unpack_from("i", raw)[0]
        position = raw.index(b"\0", 4) + 1
        while position < len(raw) and raw[position] == 0:
            position += 1
        argv = []
        for _ in range(argc):
            end = raw.index(b"\0", position)
            argv.append(os.fsdecode(raw[position:end]))
            position = end + 1
        values = raw[position:].split(b"\0")
    elif sys.platform.startswith("linux"):
        directory = Path("/proc") / str(pid)
        if directory.stat().st_uid != os.getuid():
            return None
        argv = [os.fsdecode(v) for v in (directory / "cmdline").read_bytes().split(b"\0") if v]
        values = (directory / "environ").read_bytes().split(b"\0")
    else:
        return None
    if "resume" not in argv or session_id not in argv[argv.index("resume") + 1:]:
        return None
    # Discard every other environment value, including all credentials.
    environment = {}
    for value in values:
        if b"=" in value:
            key, val = value.split(b"=", 1)
            if os.fsdecode(key) in KEYS:
                environment[os.fsdecode(key)] = os.fsdecode(val)
    if environment.get("HERDR_ENV") != "1":
        return None
    return routing(environment)


def owner_context(session_id):
    rows = subprocess.run(["/bin/ps", "-U", str(os.getuid()), "-o", "pid=,comm="],
                          text=True, capture_output=True, check=True, timeout=3).stdout.splitlines()
    matches = []
    for row in rows:
        pieces = row.strip().split(None, 1)
        if len(pieces) == 2 and Path(pieces[1]).name == "codex":
            try:
                value = process_context(int(pieces[0]), session_id)
            except (OSError, ValueError, IndexError):
                continue
            if value is not None:
                matches.append(value)
    if len(matches) > 1:
        raise ValueError("multiple live Codex clients own this session; refusing ambiguous hook routing")
    return matches[0] if matches else None


def record_for(directory, session_id):
    path = directory / (session_id + ".json")
    if not path.exists():
        return None
    value = read_private(path)
    if value.get("version") != 1 or value.get("session_id") != session_id or not SID.fullmatch(value.get("root_session_id", "")):
        raise ValueError("invalid shared Codex context record")
    routing(value["environment"])
    return value


def context_for(directory, event):
    session_id = event.get("session_id") or event.get("thread-id")
    if not isinstance(session_id, str) or not SID.fullmatch(session_id):
        return None
    record = record_for(directory, session_id)
    if record is None and event.get("agent_id") and (directory / "server.json").exists():
        # Native children dispatch SubagentStart in their own session. Follow
        # only server-supplied parent identities to find a registered root.
        endpoint = read_private(directory / "server.json")["endpoint"]
        rpc = None
        try:
            rpc = RPC(endpoint, timeout=.5)
            parent = session_id
            for _ in range(8):
                thread = rpc.call("thread/read", {"threadId": parent})["thread"]
                parent = thread.get("parentThreadId") or thread.get("forkedFromId")
                if not parent or not SID.fullmatch(parent):
                    break
                record = record_for(directory, parent)
                if record:
                    record = dict(record, session_id=session_id)
                    private_json(directory / (session_id + ".json"), record)
                    break
        except (OSError, EOFError, RuntimeError):
            pass
        finally:
            if rpc:
                rpc.close()
    if record is None:
        return None
    environment = owner_context(record["root_session_id"]) or record["environment"]
    if event.get("hook_event_name") == "SubagentStart":
        child = event.get("agent_id")
        if isinstance(child, str) and SID.fullmatch(child):
            private_json(directory / (child + ".json"), dict(record, session_id=child, environment=environment))
    return environment


def hook(directory, command, raw):
    try:
        event = json.loads(raw)
    except ValueError:
        event = {}
    environment = os.environ.copy()
    context = context_for(directory, event) if isinstance(event, dict) else None
    if context:
        environment.update(context)
        environment["CODEX_THREAD_ID"] = event.get("session_id") or event.get("thread-id")
    shell = environment.get("SHELL") or "/bin/sh"
    return subprocess.run([shell, "-c", command], input=raw, env=environment).returncode


def main():
    mode = sys.argv[1]
    if mode == "prepare":
        result = prepare(json.loads(sys.stdin.buffer.read(MAX_INPUT + 1)))
        print(json.dumps(result))
        return 1 if result["error"] else 0
    directory = Path(sys.argv[2])
    if mode == "hook":
        raw = sys.stdin.buffer.read(MAX_INPUT + 1)
        if len(raw) > MAX_INPUT:
            raise ValueError("Codex hook input is too large")
        return hook(directory, sys.argv[3], raw)
    if mode == "notify":
        event = json.loads(sys.argv[-1])
        environment = os.environ.copy()
        context = context_for(directory, event)
        if context:
            environment.update(context)
            environment["CODEX_THREAD_ID"] = event["thread-id"]
        return subprocess.run(sys.argv[3:], env=environment).returncode
    raise ValueError("unknown shared Codex adapter operation")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print("Composer Codex hook: " + str(error), file=sys.stderr)
        sys.exit(1)
