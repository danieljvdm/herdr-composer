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
    approved_test_repo=False
    while time.monotonic()<deadline:
        record=json.loads(path.read_text())
        if record['error'] or record['step']=='delivered':break
        if record['receipt'] and record['receipt']['pane'] and not approved_test_repo:
            pane=record['receipt']['pane']
            screen=command(['herdr','pane','read',pane,'--source','visible'],env=env)
            if 'Do you trust the contents of this directory?' in screen:
                # This test owns the empty repository. Emulate its user's
                # deliberate trust choice; production Composer never does this.
                assert record['delivery']=='NotSent'
                time.sleep(1)
                assert json.loads(path.read_text())['delivery']=='NotSent','task sent into trust dialog'
                command(['herdr','agent','send-keys',pane,'enter'],env=env)
                approved_test_repo=True
                print('Approved the disposable test repository; task was still NotSent.',flush=True)
        time.sleep(.25)
    print(provider,json.dumps(record,indent=2),flush=True)
    if record['step']!='delivered':raise RuntimeError('live delivery failed; retained resources at '+str(path))
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        panes=json.loads(command(['herdr','pane','list'],env=env))['result']['panes']
        if all(p['pane_id']!=record['runner_pane'] for p in panes):break
        time.sleep(.1)
    assert all(p['pane_id']!=record['runner_pane'] for p in panes),'preparation pane survived successful delivery'
    assert any(p['pane_id']==record['receipt']['pane'] for p in panes),'task pane was closed'
    # A lifecycle transition can come from startup rather than the submitted
    # task. Require the actual answer, not just Herdr's delivery acknowledgement.
    command(['herdr','pane','wait-output',record['receipt']['pane'],'--regex',r'^\s*[•]?\s*COMPOSER_INTEGRATION_OK\s*$', '--timeout','60000'],env=env)
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
