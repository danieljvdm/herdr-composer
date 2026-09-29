"""Codex discovery with an isolated CLI and stale cache. Run after cargo build."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile

BINARY = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/herdr-composer'

with tempfile.TemporaryDirectory(prefix='composer-catalog-') as tmp:
    root = pathlib.Path(tmp)
    config = root / 'config'
    cache = root / 'codex-home'
    bin_dir = root / 'bin'
    for directory in [config, cache, bin_dir]:
        directory.mkdir()
    codex = bin_dir / 'codex'
    def cli_output(value):
        codex.write_text("#!/bin/sh\n[ \"$*\" = 'debug models' ] || exit 2\n"
                         "printf '%s\\n' '" + json.dumps(value) + "'\n")

    live_models = {'models': [{
        'slug': slug,
        'display_name': slug.upper(),
        'visibility': 'hide' if slug == 'hidden-model' else 'list',
        'priority': order,
        'supported_reasoning_levels': [{'effort': 'high'}],
    } for slug, order in [('gpt-6-luna', 3), ('gpt-6-sol', 2), ('hidden-model', 4)]]}
    cli_output(live_models)
    codex.chmod(0o755)
    (cache / 'models_cache.json').write_text(json.dumps({'models': [{
        'slug': 'fixture-codex',
        'display_name': 'Fixture Codex',
        'visibility': 'list',
        'supported_reasoning_levels': [{'effort': 'high'}],
    }]}))
    env = dict(os.environ, PATH=str(bin_dir), CODEX_HOME=str(cache),
               COMPOSER_CONFIG_DIR=str(config))
    env.pop('HERDR_PLUGIN_CONFIG_DIR', None)

    def catalog(settings):
        (config / 'config.toml').write_text(settings)
        result = subprocess.run([str(BINARY), 'catalog', '--json'], env=env,
                                text=True, capture_output=True, check=True)
        return json.loads(result.stdout)

    for settings, agent in [('', 'codex'), ('[agents.codex]\nlabel="Coding"', 'codex'),
                            ('[agents.custom]\nkind="codex"', 'custom')]:
        result = catalog(settings)
        assert not result['diagnostics'], result
        entry = result['agents'][agent]
        assert entry['catalog'] == 'discovery', entry
        assert [m['id'] for m in entry['models']] == [
            'gpt-6-sol', 'gpt-6-luna', 'hidden-model'], entry
        assert entry['models'][0]['label'] == 'GPT-6-SOL', entry
        assert entry['models'][0]['efforts'] == ['high'], entry
        assert not entry['models'][2]['visible'], entry

    result = catalog('[agents.codex]\n[[agents.codex.models]]\n'
                     'id="gpt-6-sol"\nlabel="Daily work"')
    assert result['agents']['codex']['models'][0]['label'] == 'Daily work', result
    assert result['agents']['codex']['models'][0]['efforts'] == ['high'], result

    result = catalog('[agents.codex]\ncatalog="curated"\n[agents.claude]\ncatalog="curated"')
    assert result['agents']['codex']['models'] == [], result
    assert result['agents']['claude']['models'], result

    for script in ['#!/bin/sh\nexit 2\n', '#!/bin/sh\nprintf invalid\n']:
        codex.write_text(script)
        result = catalog('[agents.codex]')
        assert any('may be stale' in d for d in result['diagnostics']), result
        assert result['agents']['codex']['models'][0]['id'] == 'fixture-codex', result

    (cache / 'models_cache.json').unlink()
    cli_output(live_models)
    result = catalog('[agents.codex]')
    assert not result['diagnostics'], result
    assert result['agents']['codex']['models'][0]['id'] == 'gpt-6-sol', result

    codex.write_text('#!/bin/sh\nexit 2\n')
    result = catalog('[agents.codex]\n[[agents.codex.models]]\nid="configured"')
    assert result['diagnostics'], result
    assert result['agents']['codex']['models'][0]['id'] == 'configured', result

    cli_output(live_models)
    claude = bin_dir / 'claude'
    claude_models = [
        {'value': 'default', 'resolvedModel': 'claude-fixture-opus', 'displayName': 'Default'},
        {'value': 'opus', 'resolvedModel': 'claude-fixture-opus', 'displayName': 'Latest Opus',
         'supportsEffort': True, 'supportedEffortLevels': ['high', 'xhigh'], 'supportsFastMode': True},
        {'value': 'claude-fixture-opus', 'resolvedModel': 'claude-fixture-opus', 'displayName': 'Pinned Opus'},
        {'value': 'haiku', 'resolvedModel': 'claude-fixture-haiku', 'displayName': 'Haiku'},
    ]
    claude.write_text(f'#!{sys.executable}\n' + '''import json,sys
assert sys.argv[1:] == ['--safe-mode', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}', '--tools', '', '--no-session-persistence', '--input-format', 'stream-json', '--output-format', 'stream-json', '--verbose', '--print']
request = json.loads(sys.stdin.readline())
assert request == {'type': 'control_request', 'request_id': 'composer-catalog', 'request': {'subtype': 'initialize'}}
assert sys.stdin.read() == ''  # No prompt/inference messages.
print(json.dumps({'type': 'system', 'subtype': 'status'}))
''' + 'print(json.dumps(' + repr({'type': 'control_response', 'response': {
        'subtype': 'success', 'request_id': 'composer-catalog',
        'response': {'models': claude_models}}}) + '))\n')
    claude.chmod(0o755)
    for settings, agent in [('', 'claude'), ('[agents.claude]', 'claude'),
                            ('[agents.custom]\nkind="claude"', 'custom')]:
        result = catalog(settings)
        assert not result['diagnostics'], result
        entry = result['agents'][agent]
        assert entry['catalog'] == 'discovery', entry
        assert [m['id'] for m in entry['models']] == [m['value'] for m in claude_models] + ['claude-fixture-haiku']
        assert entry['models'][1]['efforts'] == ['high', 'xhigh']
        assert entry['models'][1]['speeds'] == []
        assert entry['models'][0]['aliases'] == []  # Resolved ID is its own entry.
        assert entry['models'][3]['aliases'] == []
        assert entry['models'][4]['id'] == 'claude-fixture-haiku'
        assert not entry['models'][4]['visible']
    result = catalog('[agents.claude]\n[[agents.claude.models]]\nid="opus"\nlabel="Daily"')
    assert result['agents']['claude']['models'][1]['label'] == 'Daily'
    assert result['agents']['claude']['models'][1]['efforts'] == ['high', 'xhigh']
    result = catalog('[agents.claude]\ncatalog="curated"')
    assert [m['id'] for m in result['agents']['claude']['models']] == ['sonnet', 'opus', 'haiku']
    for script in ['#!/bin/sh\nexit 2\n', '#!/bin/sh\nprintf invalid\n',
                   '#!/bin/sh\nprintf \'{"type":"control_response","response":{"request_id":"composer-catalog","subtype":"error","error":"fixture-denied"}}\\n\'\n']:
        claude.write_text(script)
        result = catalog('[agents.claude]\n[[agents.claude.models]]\nid="configured"')
        assert any(d.startswith('claude:') for d in result['diagnostics']), result
        assert [m['id'] for m in result['agents']['claude']['models']] == ['configured']

print('catalog defaults: ok')
