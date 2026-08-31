"""Check the optional sow wrapper without launching an agent."""
import json, os, pathlib, subprocess, tempfile
root = pathlib.Path(__file__).resolve().parents[1]
wrapper = root / 'bin/sow'
with tempfile.TemporaryDirectory(prefix='composer-sow-') as temporary:
    work = pathlib.Path(temporary)
    plugin = work / 'plugin with spaces'
    (plugin / 'bin').mkdir(parents=True)
    executable = plugin / 'bin/herdr-composer'
    executable.write_text('#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps({"argv":sys.argv[1:],"stdin":sys.stdin.read()}))\n')
    executable.chmod(0o755)
    herdr = work / 'herdr'
    herdr.write_text('#!/usr/bin/env python3\nimport json,os\nprint(json.dumps({"result":{"plugins":[{"plugin_id":"composer","enabled":True,"plugin_root":os.environ["TEST_PLUGIN"]}]}}))\n')
    herdr.chmod(0o755)
    env = dict(os.environ, HERDR_BIN_PATH=str(herdr), TEST_PLUGIN=str(plugin))
    task = 'Fix `auth`\n$(touch /tmp/never-from-sow)\n日本語  \n'
    result = subprocess.run([str(wrapper), '--no-focus', '-b', 'dan/fix-auth', '--codex', '--model', 'fixture-model', '--speed', 'normal', '--effort', 'medium', '--repo', '/path with spaces', '--here', '-'], input=task, env=env, text=True, capture_output=True, check=True)
    actual = json.loads(result.stdout)
    assert actual['stdin'] == task
    assert actual['argv'] == ['launch', '--provider', 'worktrunk', '--launch-mode', 'worktree', '--no-focus', '--branch', 'dan/fix-auth', '--agent', 'codex', '--model', 'fixture-model', '--speed', 'normal', '--effort', 'medium', '--repo', '/path with spaces', '--base', 'current', '-']
    result = subprocess.run([str(wrapper), '--', '--literal', 'task'], input='', env=env, text=True, capture_output=True, check=True)
    assert json.loads(result.stdout)['argv'][-2:] == ['--', '--literal task']
    for args in [['-b'], ['--unknown'], ['-', 'extra']]:
        result = subprocess.run([str(wrapper), *args], input='', env=env, text=True, capture_output=True)
        assert result.returncode == 2, (args, result)
print('sow wrapper passed: existing flags, literal arguments, stdin, invalid options.')
