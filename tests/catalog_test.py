"""Catalog defaults with an isolated Codex cache. Run after cargo build."""
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
    codex.write_text('#!/bin/sh\nexit 0\n')
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
        assert entry['models'][0]['id'] == 'fixture-codex', entry
        assert entry['models'][0]['efforts'] == ['high'], entry

    result = catalog('[agents.codex]\ncatalog="curated"\n[agents.claude]')
    assert result['agents']['codex']['models'] == [], result
    assert result['agents']['claude']['models'], result
    (cache / 'models_cache.json').unlink()
    result = catalog('[agents.codex]\n[[agents.codex.models]]\nid="configured"')
    assert result['diagnostics'], result
    assert result['agents']['codex']['models'][0]['id'] == 'configured', result

print('catalog defaults: ok')
