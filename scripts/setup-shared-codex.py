#!/usr/bin/env python3
"""Wrap existing trusted user hooks for stock Codex; never restart its daemon.

Requires Python 3.11+. Does not enable Composer; verify setup before opting in.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("bridge", ROOT / "src/codex_shared.py")
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


def toml_text(value):
    """Render Codex's machine-local state without introducing dependencies."""
    out = []
    def key(name): return name if re.fullmatch(r"[A-Za-z0-9_-]+", name) else json.dumps(name)
    def scalar(item):
        if isinstance(item, (str, bool, int, float)): return json.dumps(item, ensure_ascii=False)
        if isinstance(item, (datetime.datetime, datetime.date, datetime.time)): return item.isoformat()
        if isinstance(item, list): return "[" + ", ".join(scalar(v) for v in item) + "]"
        raise ValueError("unsupported TOML value")
    def table(item, prefix):
        for name, child in item.items():
            if not isinstance(child, dict) and not (isinstance(child, list) and child and all(isinstance(v, dict) for v in child)):
                out.append(key(name) + " = " + scalar(child))
        for name, child in item.items():
            path = prefix + [key(name)]
            if isinstance(child, dict):
                out.extend(["", "[" + ".".join(path) + "]"])
                table(child, path)
            elif isinstance(child, list) and child and all(isinstance(v, dict) for v in child):
                for element in child:
                    out.extend(["", "[[" + ".".join(path) + "]]"])
                    table(element, path)
    table(value, [])
    text = "\n".join(out) + "\n"
    assert tomllib.loads(text) == value
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True, help="Explicit unix:///path to a running stock Codex app-server")
    parser.add_argument("--contexts", required=True, type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "bin/herdr-composer")
    parser.add_argument("--replace-binary", type=Path, help="Migrate an already-trusted adapter at this exact old path")
    parser.add_argument("--codex-home", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")))
    parser.add_argument("--notify-policy", type=Path, help="Optional shared TOML source that owns notify")
    parser.add_argument("--apply-policy", type=Path, help="Optional existing command that renders shared policy after editing")
    args = parser.parse_args()
    directory = args.contexts.resolve()
    binary = args.binary.resolve()
    if not args.contexts.is_absolute() or not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("contexts must be absolute and binary must be an installed Composer executable")
    home = args.codex_home.resolve()
    live_path = home / "config.toml"
    hooks_path = home / "hooks.json"
    live = tomllib.loads(live_path.read_text())
    hooks = json.loads(hooks_path.read_text()) if hooks_path.exists() else {"hooks": {}}
    if any(live.get("hooks", {}).get(event) for event in hooks["hooks"]):
        raise ValueError("move inline command hooks to hooks.json before configuring shared Codex")
    rpc = bridge.RPC(args.socket)
    backups = {}
    try:
        def metadata():
            entries = rpc.call("hooks/list", {"cwds": [str(directory)]})["data"]
            if any(e.get("errors") or e.get("warnings") for e in entries):
                raise ValueError("resolve Codex hook discovery warnings before setup")
            return {h["key"]: h for entry in entries for h in entry["hooks"] if h["handlerType"] == "command"}
        old = metadata()
        effective = rpc.call("config/read", {"cwd": str(directory), "includeLayers": False})["config"]
        if effective.get("features", {}).get("hooks") is not True:
            raise ValueError("enable Codex features.hooks before configuring the adapter")
        if effective.get("notify") != live.get("notify"):
            raise ValueError("selected server and Codex home disagree on notify")
        if any(h["enabled"] and (h["sourcePath"] != str(hooks_path) or h["trustStatus"] != "trusted") for h in old.values()):
            raise ValueError("all enabled command hooks must be trusted user hooks from this Codex home")
        prefix = [str(binary), "__codex-hook", "--contexts", str(directory), "--"]
        previous_binary = str(args.replace_binary.resolve()) if args.replace_binary else None
        for event, matchers in hooks["hooks"].items():
            event_key = re.sub(r"(?<!^)(?=[A-Z])", "_", event).lower()
            for index, matcher in enumerate(matchers):
                for handler_index, handler in enumerate(matcher["hooks"]):
                    if handler["type"] != "command": continue
                    hook_key = f"{hooks_path}:{event_key}:{index}:{handler_index}"
                    if hook_key not in old: raise ValueError("server did not discover a user command hook")
                    original = handler["command"]
                    if original != old[hook_key]["command"]:
                        raise ValueError("user hooks changed during setup; retry before wrapping them")
                    parsed = shlex.split(original)
                    if parsed[:5] == prefix and len(parsed) == 6: continue
                    if previous_binary and parsed[:5] == [previous_binary, *prefix[1:]] and len(parsed) == 6:
                        original = parsed[-1]
                        parsed = shlex.split(original)
                    if "__codex-hook" in parsed:
                        raise ValueError("an adapter for a different binary or context directory is already installed")
                    handler["command"] = shlex.join(prefix + [original])
        child_command = shlex.join(prefix + [":"])
        child_hooks = hooks["hooks"].setdefault("SubagentStart", [])
        if not any(h.get("command") == child_command for matcher in child_hooks for h in matcher["hooks"]):
            child_hooks.append({"hooks": [{"type": "command", "command": child_command}]})
        notify = live.get("notify")
        if notify:
            notify_prefix = [str(binary), "__codex-notify", "--contexts", str(directory), "--"]
            if previous_binary and notify[:5] == [previous_binary, *notify_prefix[1:]]:
                notify = notify[5:]
            if notify[:5] != notify_prefix:
                if "__codex-notify" in notify: raise ValueError("a different notify adapter is already installed")
                notify = notify_prefix + notify
            live["notify"] = notify
        policy_text = None
        if args.notify_policy and notify:
            policy_text = args.notify_policy.read_text()
            if tomllib.loads(policy_text).get("notify") != effective.get("notify"):
                raise ValueError("notify policy and running configuration disagree")
            policy_text, count = re.subn(r"(?m)^notify\s*=\s*\[[^\n]*\]\s*$", "notify = " + json.dumps(notify), policy_text)
            if count != 1: raise ValueError("notify policy needs a single-line notify array")
            tomllib.loads(policy_text)
        paths = [live_path, hooks_path] + ([args.notify_policy] if policy_text is not None else [])
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        backup = directory / ("backup-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f"))
        backup.mkdir(mode=0o700, parents=True)
        for index, path in enumerate(paths):
            backups[path] = path.read_bytes() if path.exists() else None
            if backups[path] is not None:
                target = backup / (str(index) + "-" + path.name)
                target.write_bytes(backups[path]); target.chmod(0o600)
        hooks_path.write_text(json.dumps(hooks, indent=2) + "\n")
        current = metadata()
        state = live.setdefault("hooks", {}).setdefault("state", {})
        for hook_key, hook in current.items():
            previous = old.get(hook_key)
            known_child = hook["sourcePath"] == str(hooks_path) and hook["command"] == child_command
            if (previous and previous["trustStatus"] == "trusted") or known_child:
                state.setdefault(hook_key, {})["trusted_hash"] = hook["currentHash"]
        live_path.write_text(toml_text(live))
        if policy_text is not None: args.notify_policy.write_text(policy_text)
        if args.apply_policy:
            subprocess.run([str(args.apply_policy)], env=dict(os.environ, CODEX_HOME=str(home)), check=True)
        verified = metadata()
        active = {key: h for key, h in verified.items() if h["enabled"]}
        if not all(h["trustStatus"] == "trusted" and h["command"].startswith(shlex.join(prefix)) for h in active.values()):
            raise ValueError("Codex did not confirm trusted adapters")
        actual_notify = rpc.call("config/read", {"cwd": str(directory), "includeLayers": False})["config"].get("notify")
        if actual_notify != notify: raise ValueError("Codex did not confirm notify adapter")
        bridge.private_json(directory / "setup.json", {"endpoint": args.socket,
                            "hooks": {key: h["currentHash"] for key, h in active.items()}, "notify": notify})
        print("Verified shared hook adapter. Original configuration backup:", backup)
        print("Enable new Composer launches with:")
        print('[codex.shared]\nsocket = ' + json.dumps(args.socket) + '\ncontexts_dir = ' + json.dumps(str(directory)))
    except Exception:
        for path, original in backups.items():
            if original is None: path.unlink(missing_ok=True)
            else: path.write_bytes(original)
        raise
    finally:
        rpc.close()


if __name__ == "__main__":
    main()
