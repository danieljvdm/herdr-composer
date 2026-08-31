import json,os,pathlib,subprocess,tempfile,zlib,struct
ROOT=pathlib.Path(__file__).resolve().parents[1];BINARY=ROOT/'target/debug/herdr-composer'
with tempfile.TemporaryDirectory(prefix='composer-import-') as tmp:
    root=pathlib.Path(tmp);old=root/'old';old.mkdir();(old/'drafts').mkdir();(old/'attachments').mkdir()
    def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\0\0'))+chunk(b'IEND',b'')
    image=old/'attachments/original.png';image.write_bytes(png)
    (old/'config.toml').write_text('default_agent="codex"\ndispatch_focus=false\nopen_mode="tab"\ndisabled_agents="claude, grok"\n')
    (old/'drafts/first.json').write_text(json.dumps({'task':'saved task','attachments':[{'path':str(image),'name':'original.png'}]}))
    env=dict(os.environ,COMPOSER_CONFIG_DIR=str(root/'config'),COMPOSER_STATE_DIR=str(root/'state'))
    env.pop('HERDR_PLUGIN_CONFIG_DIR',None);env.pop('HERDR_PLUGIN_STATE_DIR',None)
    def run(*args):
        p=subprocess.run([str(BINARY),'import-worktrunk',str(old),*args],env=env,text=True,capture_output=True);assert p.returncode==0,p.stderr;return p.stdout
    before={p.relative_to(old):p.read_bytes() for p in old.rglob('*') if p.is_file()}
    assert 'Ignored legacy key: open_mode' in run('--preview');assert not (root/'state').exists();assert not (root/'config').exists()
    run();draft=next((root/'state/drafts').glob('*.json'));data=json.loads(draft.read_text());assert data['provider']=='worktrunk'
    assert pathlib.Path(data['attachments'][0]['path']).read_bytes()==png
    assert 'workspace = "worktrunk"' in (root/'config/config.toml').read_text()
    data['task']='newer destination';data['revision']+=1;draft.write_text(json.dumps(data));saved=draft.read_bytes();run();assert draft.read_bytes()==saved
    assert {p.relative_to(old):p.read_bytes() for p in old.rglob('*') if p.is_file()}==before
    assert not (root/'state/sessions').exists()
    assert len(list((root/'state/imports').rglob('*.json')))==1
print('Import passed: preview, mappings, retained originals, source preservation, newer destination and idempotence.')
