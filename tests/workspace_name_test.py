"""Exercise workspace naming with real session receipts and fake Herdr/Codex."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/herdr-composer'

with tempfile.TemporaryDirectory(prefix='composer-titles-') as directory:
    root = Path(directory)
    repo = root / 'repo'
    repo.mkdir()
    subprocess.run(['git', 'init', '-b', 'main', str(repo)], check=True, capture_output=True)
    subprocess.run(['git', '-C', str(repo), '-c', 'user.name=Test', '-c',
                    'user.email=test@example.invalid', 'commit', '--allow-empty', '-m', 'initial'],
                   check=True, capture_output=True)
    (root / 'bin').mkdir()
    for name in ['herdr', 'codex']:
        target = root / 'bin' / name
        shutil.copy(ROOT / 'tests/fixture_tool.py', target)
        target.chmod(0o755)
    for name in ['git', 'python3']:
        (root / 'bin' / name).symlink_to(shutil.which(name))
    (root / 'config').mkdir()
    (root / 'config/config.toml').write_text(
        '[defaults]\nagent="codex"\n[agents.codex]\ncatalog="curated"\n'
        '[workspace_naming]\nenabled=true\nmodel="fixture-luna"\n')
    env = dict(os.environ, PATH=str(root / 'bin'), COMPOSER_CONFIG_DIR=str(root / 'config'),
               COMPOSER_STATE_DIR=str(root / 'state'), HERDR_BIN_PATH=str(root / 'bin/herdr'),
               HERDR_SOCKET_PATH=str(root / 'socket'), FIXTURE_ROOT=str(root))
    for key in ['HERDR_PLUGIN_CONFIG_DIR', 'HERDR_PLUGIN_STATE_DIR', 'HERDR_WORKSPACE_ID']:
        env.pop(key, None)

    def run(args, extra=None, ok=True):
        result = subprocess.run([str(BINARY), *args], cwd=repo, env=dict(env, **(extra or {})),
                                text=True, capture_output=True, timeout=15)
        assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
        return result

    task = 'Fix the login redirect before loading the dashboard.'
    run(['launch', task])
    session = next((root / 'state/sessions').glob('*.json'))
    record = json.loads(session.read_text())
    run(['__run', record['id']])
    record = json.loads(session.read_text())
    original = json.loads((root / 'herdr.json').read_text())
    workspace = original['workspaces'][-1]
    titles = root / 'state/workspace-titles'
    created = {'type': 'workspace_created', 'workspace': workspace}
    activity = {'type': 'pane_agent_status_changed', 'workspace_id': workspace['workspace_id'],
                'pane_id': record['receipt']['pane'], 'agent_status': 'working'}
    title = {'FIXTURE_BRANCH_NAME': '{"title":"fix-login-redirect"}',
             'FIXTURE_TITLE_EXCERPT': 'Codex startup dialog'}

    def event(data, extra=None, ok=True):
        return run(['__workspace-title'], dict(title, HERDR_PLUGIN_EVENT_JSON=json.dumps({'data': data}),
                                              **(extra or {})), ok)

    def reset():
        (root / 'herdr.json').write_text(json.dumps(original))
        session.write_text(json.dumps(record))
        shutil.rmtree(titles, ignore_errors=True)
        (root / 'calls.jsonl').write_text('')

    def attempt():
        return json.loads(next(titles.glob('*.json')).read_text())

    def label():
        return json.loads((root / 'herdr.json').read_text())['workspaces'][-1]['label']

    def calls():
        return [json.loads(line) for line in (root / 'calls.jsonl').read_text().splitlines()]

    # Activity never enrolls existing workspaces, even with a Composer receipt.
    reset()
    event(activity)
    assert not titles.exists()
    assert not any(program == 'codex' for program, args in calls())

    # First activity names from the saved prompt, without waiting for terminal output.
    event(created)
    event(created)
    event(activity)
    assert json.loads((root / 'naming-input.json').read_text()) == {'task': task}
    assert label() == '(repo) fix-login-redirect' and attempt()['finished']
    assert not any(args[:2] == ['pane', 'read'] for program, args in calls())
    event(activity)
    assert len([1 for program, args in calls() if program == 'codex']) == 1
    assert json.loads(session.read_text()) == record

    # Transient inference and rename failures retry the same input on later events.
    for failure in ['FIXTURE_NAMING_FAIL', 'FIXTURE_RENAME_FAIL']:
        reset()
        event(created)
        event(activity, {failure: '1'}, ok=False)
        assert attempt()['calls'] == 1 and not attempt()['finished']
        event(activity)
        assert attempt()['calls'] == 2 and attempt()['finished']
        assert label() == '(repo) fix-login-redirect'

    # Persistent failures stop after three calls, including unchanged input.
    reset()
    event(created)
    for _ in range(3):
        event(activity, {'FIXTURE_NAMING_FAIL': '1'}, ok=False)
    event(activity)
    assert attempt()['calls'] == 3 and not attempt()['finished']
    assert label() == workspace['label']

    # A manual rename or a replaced agent during inference wins over the model.
    for change in ['FIXTURE_MANUAL_RENAME', 'FIXTURE_TITLE_PANE_CHANGED']:
        reset()
        event(created)
        event(activity, {change: '1'})
        assert label() == ('My chosen title' if change == 'FIXTURE_MANUAL_RENAME' else workspace['label'])
        assert not attempt()['finished']

    # Unrelated sockets, checkouts, panes and tab-mode receipts use the terminal fallback.
    for mismatch in ['socket', 'checkout', 'pane', 'launch_mode']:
        reset()
        different = copy.deepcopy(record)
        if mismatch == 'socket':
            different['herdr']['socket'] += '-other'
        elif mismatch == 'launch_mode':
            different['request']['launch_mode'] = 'tab'
        else:
            different['receipt'][mismatch] += '-other'
        session.write_text(json.dumps(different))
        event(created)
        event(activity)
        assert json.loads((root / 'naming-input.json').read_text()) == {'terminal_excerpt': 'Codex startup dialog\n'}

    # No concrete task: unchanged output does not consume retries; new output can.
    reset()
    session.unlink()
    event(created)
    title['FIXTURE_BRANCH_NAME'] = '{"title":null}'
    event(activity)
    event(activity)
    assert attempt()['calls'] == 1 and not attempt()['finished']
    title['FIXTURE_TITLE_EXCERPT'] = task
    title['FIXTURE_BRANCH_NAME'] = '{"title":"fix-login-redirect"}'
    event(activity)
    assert label() == '(repo) fix-login-redirect' and attempt()['finished']

print('Workspace naming passed: saved tasks, fallback, enrollment, retries, cap and identity checks.')
