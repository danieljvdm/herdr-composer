"""Stock Codex + disposable Herdr + localhost model fixture; no paid inference.

Requires local socket/process permission. Owns and stops only its named session.
"""
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/herdr-composer"
CODEX = shutil.which("codex")
HERDR = shutil.which("herdr")
spec = importlib.util.spec_from_file_location("bridge", ROOT / "src/codex_shared.py")
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)
root = Path(tempfile.mkdtemp(prefix="composer-shared-", dir="/tmp")).resolve()
home = root / "codex"
home.mkdir()
endpoint = root / "codex.sock"
contexts = root / "contexts"
config = root / "composer"
config.mkdir()
hooks_output = root / "hooks.jsonl"
shell_output = root / "shell.jsonl"
notify_output = root / "notify.jsonl"
requests = []
session = "composer-shared-" + str(os.getpid())
environment = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COMPOSER_")) and k != "CODEX_THREAD_ID"}
environment.update(CODEX_HOME=str(home), COMPOSER_CONFIG_DIR=str(config),
                   COMPOSER_STATE_DIR=str(root / "state"), TERM="xterm-256color")
# Exercise the installed Worktrunk without running the user's global hooks.
tools_dir = root / "bin"
tools_dir.mkdir()
wt_config = root / "wt.toml"
wt_config.write_text("")
wt = shutil.which("wt")
if not wt: raise RuntimeError("the live test requires installed Worktrunk")
wt_wrapper = tools_dir / "wt"
wt_wrapper.write_text("#!/bin/sh\nexec " + shlex.join([wt, "--config", str(wt_config)]) + ' "$@"\n')
wt_wrapper.chmod(0o755)
environment["PATH"] = str(tools_dir) + os.pathsep + environment["PATH"]
server = None
herdr = None
rpc = None


def command(argv, **kwargs):
    result = subprocess.run(argv, text=True, capture_output=True, timeout=40,
                            env=environment, **kwargs)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    return result.stdout


def lines(path):
    return [json.loads(v) for v in path.read_text().splitlines()] if path.exists() else []


def wait(predicate, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.1)
    raise TimeoutError("fixture timed out; inspect " + str(root))


shell_capture = "import json,os;from pathlib import Path;v={k:os.environ.get(k) for k in " + repr(bridge.KEYS) + "};v['session']=os.environ.get('CODEX_THREAD_ID');f=Path(" + repr(str(shell_output)) + ").open('a');f.write(json.dumps(v)+'\\n');f.close();print('COMPOSER_SHELL_OK')"
tool_command = shlex.join([shutil.which("python3"), "-c", shell_capture])


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *args): pass

    def do_POST(self):
        value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        requests.append({key: value.get(key) for key in ("model", "reasoning", "service_tier")})
        ident = "response-" + str(len(requests))
        tools = []
        definitions = list(value.get("tools", []))
        # Responses Lite places definitions in additional_tools input items.
        for entry in value.get("input", []):
            if entry.get("type") == "additional_tools":
                definitions.extend(entry["tools"])
        for tool in definitions:
            if tool.get("type") == "namespace":
                tools.extend((entry.get("name"), tool["name"]) for entry in tool["tools"])
            else:
                tools.append((tool.get("name"), None))
        names = {name for name, _ in tools}
        shell_name = next((name for name in ("exec_command", "shell_command", "shell") if name in names), None)
        if not shell_name:
            raise AssertionError("fixture requires a shell tool; offered names: " + repr(sorted(str(n) for n in names)))
        # Exercise the stock model shell tool and its PreToolUse hook once.
        if not any(item.get("type") == "function_call_output" for item in value.get("input", [])):
            args = {"cmd": tool_command} if shell_name == "exec_command" else (
                {"command": tool_command} if shell_name == "shell_command" else {"command": ["/bin/bash", "-c", tool_command]})
            item = {"id": "item-" + ident, "type": "function_call", "name": shell_name,
                    "call_id": "call-" + ident, "arguments": json.dumps(args)}
            namespace = next(namespace for name, namespace in tools if name == shell_name)
            if namespace: item["namespace"] = namespace
            events = [{"type": "response.output_item.added", "item": item},
                      {"type": "response.output_item.done", "item": item}]
        else:
            item = {"id": "item-" + ident, "type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "COMPOSER_SHARED_OK"}]}
            events = [{"type": "response.output_item.added", "item": item},
                      {"type": "response.output_text.delta", "delta": "COMPOSER_SHARED_OK"},
                      {"type": "response.output_item.done", "item": item}]
        events.insert(0, {"type": "response.created", "response": {"id": ident}})
        events.append({"type": "response.completed", "response": {"id": ident, "usage": {
            "input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}})
        data = "".join("event: " + e["type"] + "\ndata: " + json.dumps(e) + "\n\n" for e in events).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
threading.Thread(target=provider.serve_forever, daemon=True).start()
print("Fixture:", root, "stock Codex:", command([CODEX, "--version"]).strip(), flush=True)
capture = root / "hook.py"
capture.write_text("import os,json,sys,subprocess\nfrom pathlib import Path\ne=json.load(sys.stdin)\n"
                   "v={k:os.environ.get(k) for k in " + repr(bridge.KEYS) + "}\n"
                   "v.update(session=e.get('session_id'),event=e.get('hook_event_name'),thread=os.environ.get('CODEX_THREAD_ID'))\n"
                   "with Path(" + repr(str(hooks_output)) + ").open('a') as f:f.write(json.dumps(v)+'\\n')\n"
                   "if e.get('hook_event_name')=='SessionStart':\n"
                   " subprocess.run(['bash',str(Path.home()/'.codex/herdr-agent-state.sh'),'session'],input=json.dumps(e),text=True,check=True)\n")
notify = root / "notify.py"
notify.write_text("import json,os,sys\nfrom pathlib import Path\n"
                  "with Path(" + repr(str(notify_output)) + ").open('a') as f:f.write(json.dumps({'pane':os.environ.get('HERDR_PANE_ID'),'payload':json.loads(sys.argv[-1])})+'\\n')\n")
prefix = [str(BINARY), "__codex-hook", "--contexts", str(contexts), "--"]
wrapped = shlex.join(prefix + [shlex.join([shutil.which("python3"), str(capture)])])
(home / "hooks.json").write_text(json.dumps({"hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": wrapped}]}],
    "PreToolUse": [{"matcher": "^Bash$", "hooks": [{"type": "command", "command": wrapped}]}],
    "SubagentStart": [{"hooks": [{"type": "command", "command": shlex.join(prefix + [":"])}]}],
}}))
notify_args = [str(BINARY), "__codex-notify", "--contexts", str(contexts), "--", shutil.which("python3"), str(notify)]
codex_config = home / "config.toml"
codex_config.write_text('model="gpt-6-sol"\nmodel_provider="fixture"\nweb_search="disabled"\n'
                       'notify=' + json.dumps(notify_args) + '\napproval_policy="never"\nsandbox_mode="danger-full-access"\n'
                       '[model_providers.fixture]\nname="Local fixture only"\nbase_url="http://127.0.0.1:' + str(provider.server_port) + '/v1"\n'
                       'wire_api="responses"\nrequires_openai_auth=false\nrequest_max_retries=0\nstream_max_retries=0\n'
                       '[features]\nhooks=true\nplugins=false\nshell_snapshot=true\n'
                       '[tui]\nstatus_line=["model-with-reasoning","run-state","current-dir"]\n'
                       '[projects.' + json.dumps(str(root)) + ']\ntrust_level="trusted"\n')
# Use a private catalog to keep discovery local as well.
if (Path.home() / ".codex/models_cache.json").exists():
    models = json.loads((Path.home() / ".codex/models_cache.json").read_text())["models"]
    # The mock emits direct function calls. Production keeps its real catalog
    # and tool mode; this private catalog selects the corresponding test wire format.
    for model in models: model["tool_mode"] = "direct"
    model_catalog = home / "catalog.json"
    model_catalog.write_text(json.dumps({"models": models}))
    codex_config.write_text('model_catalog_json=' + json.dumps(str(model_catalog)) + '\n' + codex_config.read_text())
config.joinpath("config.toml").write_text('[defaults]\nagent="codex"\nfocus=false\n'
                                        '[agents.codex]\ncatalog="curated"\n'
                                        '[[agents.codex.models]]\nid="gpt-6-sol"\nefforts=["high","low"]\nspeeds=["fast","normal"]\n'
                                        '[[agents.codex.models]]\nid="gpt-6-luna"\nefforts=["high","low"]\nspeeds=["fast","normal"]\n'
                                        '[codex.shared]\nsocket=' + json.dumps("unix://" + str(endpoint)) + '\ncontexts_dir=' + json.dumps(str(contexts)) + '\n')
herdr_config = root / "herdr.toml"
herdr_config.write_text('onboarding=false\n[terminal]\ndefault_shell="/bin/bash"\nshell_mode="non_login"\n[update]\nversion_check=false\n')
environment["HERDR_CONFIG_PATH"] = str(herdr_config)
try:
    # Only these disposable services are ever stopped/restarted by the test.
    server = subprocess.Popen([CODEX, "app-server", "--listen", "unix://" + str(endpoint)], env=environment,
                              cwd=root, stdout=subprocess.DEVNULL, stderr=(root / "codex.log").open("a"))
    wait(endpoint.exists)
    rpc = bridge.RPC("unix://" + str(endpoint))
    metadata = rpc.call("hooks/list", {"cwds": [str(root)]})["data"][0]["hooks"]
    with codex_config.open("a") as stream:
        for hook in metadata:
            stream.write('\n[hooks.state.' + json.dumps(hook["key"]) + ']\ntrusted_hash=' + json.dumps(hook["currentHash"]) + '\n')
    # Confirm fresh config is read without restarting the daemon.
    metadata = rpc.call("hooks/list", {"cwds": [str(root)]})["data"][0]["hooks"]
    assert all(h["trustStatus"] == "trusted" for h in metadata), metadata
    print(command([shutil.which("python3"), str(ROOT / "scripts/setup-shared-codex.py"),
                   "--socket", "unix://" + str(endpoint), "--contexts", str(contexts),
                   "--binary", str(BINARY), "--codex-home", str(home)]), flush=True)
    herdr = subprocess.Popen([HERDR, "--session", session, "server"], env=environment, cwd=root,
                             stdout=subprocess.DEVNULL, stderr=(root / "herdr.log").open("a"))
    def find_socket():
        result = subprocess.run([HERDR, "session", "list", "--json"], env=environment,
                                text=True, capture_output=True, timeout=5)
        if result.returncode: return None
        value = json.loads(result.stdout)
        entries = value if isinstance(value, list) else value.get("sessions", value.get("result", {}).get("sessions", []))
        entry = next((s for s in entries if s.get("name") == session), None)
        if not entry: return None
        return entry.get("socket_path") or entry.get("api_socket_path")
    environment["HERDR_SOCKET_PATH"] = wait(find_socket)
    environment.update(HERDR_ENV="1", HERDR_BIN_PATH=HERDR)
    repo = root / "repo"
    command(["git", "init", "-b", "main", str(repo)])
    command(["git", "-C", str(repo), "-c", "user.name=Fixture", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "initial"])
    source = json.loads(command([HERDR, "workspace", "create", "--cwd", str(repo), "--no-focus"]))["result"]
    environment.update(HERDR_WORKSPACE_ID=source["workspace"]["workspace_id"],
                       HERDR_PANE_ID=source["root_pane"]["pane_id"], HERDR_TAB_ID=source["tab"]["tab_id"], HERDR_STARTUP_CWD=str(repo))
    active = []
    for mode, workspace_provider, model, effort, speed in [
            ("tab", "herdr", "gpt-6-sol", "high", "fast"),
            ("worktree", "herdr", "gpt-6-luna", "low", "normal"),
            ("worktree", "worktrunk", "gpt-6-sol", "high", "normal")]:
        args = [str(BINARY), "launch", "--launch-mode", mode, "--model", model, "--effort", effort, "--speed", speed, "--no-focus"]
        if mode == "worktree": args += ["--provider", workspace_provider, "--branch", "fixture-shared-" + workspace_provider + "-" + str(os.getpid()), "--base", "main"]
        print(command(args + ["Local fixture task: run the supplied shell check, then reply COMPOSER_SHARED_OK."], cwd=repo), flush=True)
        path = max((root / "state/sessions").glob("*.json"), key=lambda p: p.stat().st_mtime_ns)
        def delivered():
            record = json.loads(path.read_text())
            if record["error"]: raise RuntimeError(json.dumps(record))
            receipt = record.get("receipt")
            if receipt and receipt.get("pane") and record["delivery"] == "NotSent":
                screen = command([HERDR, "pane", "read", receipt["pane"], "--source", "visible"])
                if "Trust this folder?" in screen and "1. Trust and continue" in screen:
                    common = command(["git", "-C", receipt["checkout"], "rev-parse", "--git-common-dir"]).strip()
                    assert Path(receipt["checkout"], common).resolve() == repo / ".git"
                    time.sleep(.5)
                    assert json.loads(path.read_text())["delivery"] == "NotSent"
                    command([HERDR, "agent", "send-keys", receipt["pane"], "enter"])
                    print("Approved owned empty fixture checkout; task remained NotSent.", flush=True)
            return record if record["step"] == "delivered" else None
        record = wait(delivered, 60)
        active.append(record)
        pane = record["receipt"]["pane"]
        sid = record["codex_thread"]
        assert record["delivery"] == "Confirmed" and sid
        command([HERDR, "pane", "wait-output", pane, "--regex", r"^\s*[•]?\s*COMPOSER_SHARED_OK\s*$", "--timeout", "30000"])
        wait(lambda: any(row.get("session") == sid for row in lines(hooks_output)))
        wait(lambda: any(row["payload"].get("thread-id") == sid for row in lines(notify_output)))
        thread = rpc.call("thread/read", {"threadId": sid})["thread"]
        # Check live session config and actual provider input, not just launch argv.
        assert thread["cwd"] == record["receipt"]["checkout"], thread
        pane_info = json.loads(command([HERDR, "pane", "list"]))["result"]["panes"]
        live_pane = next(p for p in pane_info if p["pane_id"] == pane)
        assert sid in json.dumps(live_pane), live_pane
        assert any(r["model"] == model and r["reasoning"]["effort"] == effort
                   and (r["service_tier"] == "priority" if speed == "fast" else r["service_tier"] in (None, "default")) for r in requests), requests
        # Also check the loaded shell policy after the preparation connection closed.
        before = len(lines(shell_output))
        rpc.call("thread/shellCommand", {"threadId": sid, "command": tool_command, "timeoutMs": 5000})
        wait(lambda: len(lines(shell_output)) > before)
        assert lines(shell_output)[-1]["HERDR_PANE_ID"] == pane, lines(shell_output)
        assert all(e["HERDR_PANE_ID"] == pane and e["thread"] == sid
                   for e in lines(hooks_output) if e["session"] == sid), lines(hooks_output)
        assert all(e["pane"] == pane for e in lines(notify_output) if e["payload"].get("thread-id") == sid)
        screen = command([HERDR, "pane", "read", pane, "--source", "recent-unwrapped"])
        assert "Running without the shared background server" not in screen
        print("PASS", json.dumps({"mode": mode, "provider": workspace_provider, "model": model, "effort": effort, "speed": speed, "pane": pane, "session": sid}), flush=True)
    assert any(e["event"] == "PreToolUse" for e in lines(hooks_output)), "mock shell tool did not exercise PreToolUse"
    for record in active:
        command([str(BINARY), "remove", "--session", record["id"]], cwd=repo)
    print("PASS three simultaneous shared Composer clients, both worktree providers, hooks, notify, shell tools, delivery and cleanup", flush=True)
finally:
    if rpc: rpc.close()
    if herdr:
        result = subprocess.run([HERDR, "session", "stop", session, "--json"], env=environment, capture_output=True, timeout=15)
        if result.returncode: herdr.terminate()
        try: herdr.wait(timeout=10)
        except subprocess.TimeoutExpired: herdr.kill(); herdr.wait()
        subprocess.run([HERDR, "session", "delete", session, "--json"], env=environment, capture_output=True, timeout=15)
    if server:
        server.terminate()
        try: server.wait(timeout=10)
        except subprocess.TimeoutExpired: server.kill(); server.wait()
    repo = root / "repo"
    if repo.exists():
        entries = subprocess.check_output(["git", "-C", str(repo), "worktree", "list", "--porcelain"], text=True)
        for line in entries.splitlines():
            if line.startswith("worktree ") and Path(line[9:]) != repo:
                subprocess.run(["git", "-C", str(repo), "worktree", "remove", line[9:]], check=True, capture_output=True)
    provider.shutdown()
    provider.server_close()
