"""Adapter invariants; no inference, real agents, or global configuration changes."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("bridge", ROOT / "src/codex_shared.py")
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)
SESSION = "00000000-0000-0000-0000-000000000001"
CHILD = "00000000-0000-0000-0000-000000000002"


def environment(workspace="wTest"):
    return dict(zip(bridge.KEYS, ["/bin/herdr", "1", workspace + ":p1", "/tmp/herdr.sock",
                                "/tmp/repo", workspace + ":t1", workspace]))


def record(directory, sid=SESSION, root=SESSION, context=None):
    bridge.private_json(directory / (sid + ".json"), {
        "version": 1, "session_id": sid, "root_session_id": root,
        "environment": context or environment(), "endpoint": "unix:///tmp/codex.sock",
    })


class Adapter(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="composer-adapter-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / "contexts"

    def test_private_allowlist_and_identity(self):
        record(self.directory)
        path = self.directory / (SESSION + ".json")
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.directory.stat().st_mode & 0o777, 0o700)
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            bridge.record_for(self.directory, SESSION)
        path.chmod(0o600)
        with self.assertRaises(ValueError):
            bridge.routing(dict(environment(), OPENAI_API_KEY="never-persist-this"))
        self.assertIsNone(bridge.context_for(self.directory, {"session_id": "../../secret"}))
        symlink = self.directory / "link.json"
        symlink.symlink_to(path)
        with self.assertRaises(ValueError):
            bridge.read_private(symlink)

    def test_independent_sessions_resume_and_children(self):
        record(self.directory)
        record(self.directory, CHILD, CHILD, environment("wOther"))
        with patch.object(bridge, "owner_context", return_value=None):
            self.assertEqual(bridge.context_for(self.directory, {"session_id": SESSION}), environment())
            self.assertEqual(bridge.context_for(self.directory, {"session_id": CHILD}), environment("wOther"))
        fresh = environment("wResumed")
        with patch.object(bridge, "owner_context", return_value=fresh):
            bridge.context_for(self.directory, {"session_id": SESSION,
                               "hook_event_name": "SubagentStart", "agent_id": CHILD})
        child = bridge.record_for(self.directory, CHILD)
        self.assertEqual(child["root_session_id"], SESSION)
        self.assertEqual(child["environment"], fresh)
        with patch.object(bridge, "owner_context", side_effect=ValueError("ambiguous")):
            with self.assertRaises(ValueError):
                bridge.context_for(self.directory, {"session_id": SESSION})

    def test_native_child_before_parent_hook(self):
        record(self.directory)
        bridge.private_json(self.directory / "server.json", {"endpoint": "unix:///tmp/codex.sock"})
        class ParentRPC:
            def __init__(self, *args, **kwargs): pass
            def call(self, method, params):
                self_test.assertEqual(method, "thread/read")
                return {"thread": {"parentThreadId": SESSION}}
            def close(self): pass
        self_test = self
        with patch.object(bridge, "RPC", ParentRPC), patch.object(bridge, "owner_context", return_value=None):
            context = bridge.context_for(self.directory, {"session_id": CHILD, "agent_id": CHILD})
        self.assertEqual(context, environment())
        self.assertEqual(bridge.record_for(self.directory, CHILD)["root_session_id"], SESSION)

    def test_hook_and_notify_preserve_bytes_arguments_and_status(self):
        record(self.directory)
        capture = Path(self.temporary.name) / "capture.py"
        capture.write_text("import json,os,sys\nprint(json.dumps({'pane':os.environ.get('HERDR_PANE_ID'),"
                           "'thread':os.environ.get('CODEX_THREAD_ID'),'args':sys.argv[1:]}))\n"
                           "sys.stdout.flush()\nsys.stdout.buffer.write(sys.stdin.buffer.read())\n"
                           "sys.stderr.write('original-stderr\\n')\nsys.exit(7)\n")
        native = [sys.executable, str(capture), "literal ' argument", "$(do-not-run)"]
        cmd = shlex.join(native)
        raw = (" {\"session_id\":\"" + SESSION + "\",\"task\":\"日本語\"} \n").encode()
        env = dict(os.environ, HERDR_PANE_ID="daemon-stale-pane", CODEX_THREAD_ID="stale-thread")
        binary = ROOT / "target/debug/herdr-composer"
        common = [str(binary), "__codex-hook", "--contexts", str(self.directory), "--", cmd]
        result = subprocess.run(common, input=raw, env=env, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 7, result.stderr)
        first, tail = result.stdout.split(b"\n", 1)
        self.assertEqual(json.loads(first)["pane"], "wTest:p1")
        self.assertEqual(json.loads(first)["thread"], SESSION)
        self.assertEqual(json.loads(first)["args"], native[2:])
        self.assertEqual(tail, raw)
        self.assertEqual(result.stderr, b"original-stderr\n")
        # Ordinary embedded sessions keep their inherited environment and hook input.
        result = subprocess.run(common, input=b"unregistered input\n", env=env, capture_output=True, timeout=10)
        first, tail = result.stdout.split(b"\n", 1)
        self.assertEqual(json.loads(first)["pane"], "daemon-stale-pane")
        self.assertEqual(json.loads(first)["thread"], "stale-thread")
        self.assertEqual(tail, b"unregistered input\n")
        payload = json.dumps({"thread-id": SESSION, "type": "agent-turn-complete"})
        result = subprocess.run([str(binary), "__codex-notify", "--contexts", str(self.directory),
                                 "--", *native, payload], input=b"", env=env, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 7, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value["args"], native[2:] + [payload])
        self.assertEqual(value["pane"], "wTest:p1")

    def test_prepare_no_inference_and_partial_failure(self):
        calls = []
        mismatch = False
        partial = False
        class FakeRPC:
            def __init__(self, *args): pass
            def close(self): pass
            def call(self, method, params):
                calls.append((method, params))
                if method == "hooks/list":
                    return {"data": [{"hooks": [{"key": "user-hook", "currentHash": "changed" if mismatch else "hash",
                                                  "handlerType": "command", "enabled": True, "trustStatus": "trusted"}]}]}
                if method == "config/read": return {"config": {"features": {"hooks": True}}}
                if method == "thread/start": return {"thread": {"id": SESSION}}
                if method == "thread/section/move" and partial: raise RuntimeError("placement failed")
                return {}
        bridge.private_json(self.directory / "setup.json", {"endpoint": "unix:///tmp/codex.sock",
                                                            "hooks": {"user-hook": "hash"}})
        value = {"endpoint": "unix:///tmp/codex.sock", "contexts_dir": str(self.directory), "cwd": "/tmp/repo",
                 "environment": environment(), "model": "selected-model", "effort": "high", "speed": "fast",
                 "launch_id": "durable-launch", "name": "Composer test"}
        with patch.object(bridge, "RPC", FakeRPC):
            self.assertEqual(bridge.prepare(value), {"thread_id": SESSION, "error": None})
            self.assertEqual([m for m, _ in calls], ["hooks/list", "config/read", "thread/start", "thread/section/move", "thread/name/set"])
            start = calls[2][1]
            self.assertEqual(start["model"], "selected-model")
            self.assertEqual(start["cwd"], "/tmp/repo")
            self.assertEqual(start["config"], dict({"model_reasoning_effort": "high", "service_tier": "fast"},
                                               **{"shell_environment_policy.set." + k: v for k, v in environment().items()}))
            self.assertNotIn("approvalPolicy", start)
            self.assertNotIn("sandbox", start)
            calls.clear(); partial = True
            result = bridge.prepare(value)
            self.assertEqual(result["thread_id"], SESSION)
            self.assertEqual(result["error"], "placement failed")
            self.assertIsNotNone(bridge.record_for(self.directory, SESSION))
            self.assertNotIn("thread/name/set", [m for m, _ in calls])
            calls.clear(); mismatch = True
            self.assertIsNone(bridge.prepare(value)["thread_id"])
            self.assertNotIn("thread/start", [m for m, _ in calls])

    def test_setup_preserves_trust_and_is_repeatable(self):
        spec = importlib.util.spec_from_file_location("setup", ROOT / "scripts/setup-shared-codex.py")
        setup = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(setup)
        home = Path(self.temporary.name).resolve() / "codex"
        home.mkdir()
        hooks_path = home / "hooks.json"
        live_path = home / "config.toml"
        original = "printf '%s' 'literal $(do-not-run)'"
        hooks_path.write_text(json.dumps({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": original}]}]}}))
        def discovered():
            state = tomllib.loads(live_path.read_text()).get("hooks", {}).get("state", {})
            result = []
            for event, matchers in json.loads(hooks_path.read_text())["hooks"].items():
                key_event = {"SessionStart": "session_start", "SubagentStart": "subagent_start"}[event]
                for i, matcher in enumerate(matchers):
                    for j, handler in enumerate(matcher["hooks"]):
                        key = f"{hooks_path}:{key_event}:{i}:{j}"
                        fingerprint = hashlib.sha256(handler["command"].encode()).hexdigest()
                        result.append(dict(key=key, currentHash=fingerprint, sourcePath=str(hooks_path),
                                           handlerType="command", enabled=True, command=handler["command"],
                                           trustStatus="trusted" if state.get(key, {}).get("trusted_hash") == fingerprint else "untrusted"))
            return result
        original_key = str(hooks_path) + ":session_start:0:0"
        live_path.write_text(setup.toml_text({"model": "keep-model", "notify": ["/bin/true", "literal argument"], "features": {"hooks": True},
                                              "hooks": {"state": {original_key: {"trusted_hash": hashlib.sha256(original.encode()).hexdigest()}}}}))
        policy = home / "config.shared.toml"
        policy.write_text('# Preserve this comment\nnotify = ["/bin/true", "literal argument"]\n\n[features]\nhooks = true\n')
        class SetupRPC:
            def __init__(self, *args): pass
            def close(self): pass
            def call(self, method, params):
                if method == "hooks/list": return {"data": [{"hooks": discovered()}]}
                if method == "config/read": return {"config": tomllib.loads(live_path.read_text())}
                raise AssertionError(method)
        args = ["setup", "--socket", "unix:///tmp/server.sock", "--contexts", str(self.directory),
                "--binary", str(ROOT / "target/debug/herdr-composer"), "--codex-home", str(home),
                "--notify-policy", str(policy)]
        with patch.object(sys, "argv", args), patch.object(setup.bridge, "RPC", SetupRPC), patch("builtins.print"):
            setup.main()
            first = hooks_path.read_bytes(), live_path.read_bytes()
            setup.main()
            self.assertEqual((hooks_path.read_bytes(), live_path.read_bytes()), first)
            parsed = shlex.split(json.loads(hooks_path.read_text())["hooks"]["SessionStart"][0]["hooks"][0]["command"])
            self.assertEqual(parsed[-1], original)
            self.assertEqual(tomllib.loads(live_path.read_text())["model"], "keep-model")
            self.assertTrue(policy.read_text().startswith('# Preserve this comment\n'))
            self.assertEqual(tomllib.loads(policy.read_text())["notify"], tomllib.loads(live_path.read_text())["notify"])
            self.assertTrue(all(h["trustStatus"] == "trusted" for h in discovered()))
            # A stable launcher can replace an exact, already-trusted binary
            # without nesting adapters or changing the underlying commands.
            args += ["--replace-binary", str(ROOT / "target/debug/herdr-composer"), "--binary", sys.executable]
            setup.main()
            migrated = shlex.split(json.loads(hooks_path.read_text())["hooks"]["SessionStart"][0]["hooks"][0]["command"])
            self.assertEqual(migrated[0], str(Path(sys.executable).resolve()))
            self.assertEqual(migrated[-1], original)
            self.assertEqual(tomllib.loads(live_path.read_text())["notify"][5:], ["/bin/true", "literal argument"])
            self.assertTrue(all(h["trustStatus"] == "trusted" for h in discovered()))
            # Editing an original hook never earns automatic trust from setup.
            hooks = json.loads(hooks_path.read_text())
            hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"] = "untrusted edit"
            hooks_path.write_text(json.dumps(hooks))
            before = hooks_path.read_bytes(), live_path.read_bytes()
            with self.assertRaises(ValueError): setup.main()
            self.assertEqual((hooks_path.read_bytes(), live_path.read_bytes()), before)


if __name__ == "__main__":
    unittest.main()
