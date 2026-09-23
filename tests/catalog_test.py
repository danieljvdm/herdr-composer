"""Codex discovery with an isolated CLI and stale cache. Run after cargo build."""
import json
import os
import pathlib
import subprocess
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

    result = catalog('[agents.codex]\ncatalog="curated"\n[agents.claude]')
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

print('catalog defaults: ok')
