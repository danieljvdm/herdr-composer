"""Disposable test against an already running, explicitly named Herdr server."""
import argparse,json,os,pathlib,subprocess,tempfile,time
parser=argparse.ArgumentParser();parser.add_argument('--socket',required=True);parser.add_argument('--providers',nargs='+',choices=['herdr','worktrunk'],default=['herdr','worktrunk']);options=parser.parse_args()
root=pathlib.Path(tempfile.mkdtemp(prefix='composer-live-')).resolve()
repo=root/'repo';repo.mkdir()
def command(argv,**kwargs):
    p=subprocess.run(argv,text=True,capture_output=True,**kwargs)
    if p.returncode:raise RuntimeError(p.stderr+p.stdout)
    return p.stdout
command(['git','init','-b','main',str(repo)])
command(['git','-C',str(repo),'-c','user.name=Composer test','-c','user.email=test@example.invalid','commit','--allow-empty','-m','initial'])
remote=root/'remote.git';command(['git','init','--bare','-b','main',str(remote)])
command(['git','-C',str(repo),'remote','add','origin',str(remote)])
command(['git','-C',str(repo),'push','-u','origin','main'])
command(['git','-C',str(repo),'remote','set-head','origin','main'])
config=root/'config';config.mkdir();(config/'config.toml').write_text('[defaults]\nagent="codex"\nfocus=false\n')
env=dict(os.environ,COMPOSER_CONFIG_DIR=str(config),COMPOSER_STATE_DIR=str(root/'state'),HERDR_SOCKET_PATH=options.socket)
env.pop('HERDR_PLUGIN_CONFIG_DIR',None);env.pop('HERDR_PLUGIN_STATE_DIR',None)
binary=pathlib.Path(__file__).resolve().parents[1]/'target/debug/herdr-composer'
print('Test files:',root,flush=True)
for provider in options.providers:
    output=command([str(binary),'launch','--provider',provider,'--base','main','--no-focus','Reply with exactly COMPOSER_INTEGRATION_OK. Do not read or modify any files.'],cwd=repo,env=env)
    print(output,flush=True)
    path=max((root/'state/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
    deadline=time.monotonic()+340
    while time.monotonic()<deadline:
        record=json.loads(path.read_text())
        if record['error'] or record['step']=='delivered':break
        time.sleep(.25)
    print(provider,json.dumps(record,indent=2),flush=True)
    if record['step']!='delivered':raise RuntimeError('live delivery failed; retained resources at '+str(path))
    target=pathlib.Path(record['receipt']['checkout'])
    (target/'unmerged-test.txt').write_text('Retain this branch after removing the checkout.\n')
    refusal=subprocess.run([str(binary),'remove','--session',record['id']],cwd=repo,env=env,text=True,capture_output=True)
    assert refusal.returncode!=0 and (target/'unmerged-test.txt').exists(),refusal.stdout+refusal.stderr
    command(['git','-C',str(target),'add','unmerged-test.txt'])
    command(['git','-C',str(target),'-c','user.name=Composer test','-c','user.email=test@example.invalid','commit','-m','unmerged test'])
    if provider=='worktrunk':
        print(command([str(binary),'remove','--current'],cwd=target,env=dict(env,HERDR_WORKSPACE_ID=record['receipt']['workspace'])),flush=True)
        deadline=time.monotonic()+30
        while time.monotonic()<deadline:
            removal=json.loads(path.read_text())
            if removal['step']=='removed':break
            time.sleep(.1)
        assert removal['step']=='removed',removal
    else:print(command([str(binary),'remove','--session',record['id']],cwd=repo,env=env),flush=True)
    assert not target.exists()
    command(['git','-C',str(repo),'show-ref','--verify','refs/heads/'+record['request']['branch']])
print('Requested providers passed live launch/delivery/removal. Sources retained for inspection at',root,flush=True)
