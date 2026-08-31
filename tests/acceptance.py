"""Run after cargo build. External tools are fakes; Git and persistence are real."""
import json, os, pathlib, shutil, subprocess, tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
BINARY=ROOT/'target/debug/herdr-composer'

def run(args,env,cwd,input=None,ok=True):
    p=subprocess.run([str(BINARY),*args],env=env,cwd=cwd,input=input,text=True,capture_output=True,timeout=20)
    if ok: assert p.returncode==0,(args,p.stdout,p.stderr)
    else: assert p.returncode!=0,(args,p.stdout,p.stderr)
    return p
def setup(root):
    repo=root/'repo';repo.mkdir();subprocess.run(['git','init','-b','main',str(repo)],check=True,capture_output=True)
    subprocess.run(['git','-C',str(repo),'-c','user.name=Test','-c','user.email=test@example.invalid','commit','--allow-empty','-m','initial'],check=True,capture_output=True)
    bin=root/'bin';bin.mkdir()
    for name in ['herdr','wt','codex']:
        shutil.copy(ROOT/'tests/fixture_tool.py',bin/name);(bin/name).chmod(0o755)
    for name in ['git','python3']: (bin/name).symlink_to(shutil.which(name))
    config=root/'config';config.mkdir();(config/'config.toml').write_text('[defaults]\nagent="codex"\n[agents.codex]\nallow_custom_model=true\n[[agents.codex.models]]\nid="fixture-model"\naliases=["daily"]\nefforts=["low","high"]\nspeeds=["normal","fast"]\n')
    env=dict(os.environ,PATH=str(bin),COMPOSER_CONFIG_DIR=str(config),COMPOSER_STATE_DIR=str(root/'state'),HERDR_BIN_PATH=str(bin/'herdr'),HERDR_SOCKET_PATH=str(root/'socket'),FIXTURE_ROOT=str(root))
    for k in ['HERDR_PLUGIN_CONFIG_DIR','HERDR_PLUGIN_STATE_DIR','HERDR_WORKSPACE_ID']:env.pop(k,None)
    return repo,env
def records(root):return list((root/'state/sessions').glob('*.json'))
def launch(root,repo,env,provider='herdr',extra=(),task='literal `task`\n$(touch never)\n日本語  \n'):
    run(['launch',*(['--provider',provider] if provider else []),'--model','daily','--effort','high','--speed','normal',*extra,'-'],env,repo,task)
    path=max(records(root),key=lambda p:p.stat().st_mtime_ns);record=json.loads(path.read_text());assert record['request']['task']==task;return path,record['id']
def calls(root):return [json.loads(s) for s in (root/'calls.jsonl').read_text().splitlines()]

with tempfile.TemporaryDirectory(prefix='composer-acceptance-') as tmp:
    root=pathlib.Path(tmp);repo,env=setup(root)
    # Pre-mutation errors: invalid explicit values, missing originals, occupied branch.
    for extra in [ ['--model','missing'], ['--effort','ultra'], ['--branch','main'], ['--base','missing'], ['--attach',str(root/'missing.png')], ['--provider','missing'] ]:
        run(['launch',*extra,'task'],env,repo,ok=False)
    assert not records(root)
    assert json.loads((root/'herdr.json').read_text())['workspaces']==[]
    # Native path works without Worktrunk, fzf, jq, or gh on PATH.
    (root/'bin/wt').unlink()
    path,id=launch(root,repo,env)
    old=json.loads(path.read_text());saved_config=(root/'config/config.toml').read_text();(root/'config/config.toml').write_text('[agents.codex]\nenabled=false\n');run(['__run',id],env,repo);(root/'config/config.toml').write_text(saved_config)
    record=json.loads(path.read_text());assert record['delivery']=='Confirmed';assert json.loads((root/'prompt.json').read_text())==record['request']['task']
    assert record['request']['native_args']==['--model','fixture-model','-c','model_reasoning_effort="high"','-c','service_tier="default"']
    run(['__run',id],env,repo,ok=False)
    assert len([c for c in calls(root) if c[1][:2]==['agent','prompt']])==1
    # Focus/default changes cannot redirect recorded cleanup; dirty Git refuses.
    checkout=pathlib.Path(record['receipt']['checkout']);(checkout/'pending').write_text('keep')
    run(['remove','--session',id],env,repo,ok=False);assert (checkout/'pending').read_text()=='keep'
    # A documented refusal can be retried explicitly after saving the dirty work.
    record=json.loads(path.read_text());assert record['error'];assert record['step']=='removal_failed';(checkout/'pending').unlink()
    (root/'config/config.toml').write_text('[defaults]\nworkspace="worktrunk"\n')
    run(['remove','--session',id],env,repo);record=json.loads(path.read_text());assert record['step']=='removed';assert record['request'] is None
    assert subprocess.run(['git','-C',str(repo),'show-ref','--verify','refs/heads/'+old['request']['branch']],capture_output=True).returncode==0
    run(['remove','--session',id],env,repo,ok=False)
    # Restore config for further launches.
    (root/'config/config.toml').write_text('[defaults]\nagent="codex"\n[agents.codex]\nallow_custom_model=true\n[[agents.codex.models]]\nid="fixture-model"\naliases=["daily"]\nefforts=["high"]\nspeeds=["normal"]\n')
    shutil.copy(ROOT/'tests/fixture_tool.py',root/'bin/wt');(root/'bin/wt').chmod(0o755)
    path,id=launch(root,repo,env,'worktrunk');run(['__run',id],env,repo);assert (root/'hook-finished').exists()
    record=json.loads(path.read_text());ws=record['receipt']['workspace'];target=pathlib.Path(record['receipt']['checkout'])
    # --current uses caller context and needs one exact record.
    run(['remove','--current'],dict(env,HERDR_WORKSPACE_ID='wrong'),target,ok=False)
    # Workspace binding change stops cleanup before the provider.
    h=json.loads((root/'herdr.json').read_text());saved=json.loads(json.dumps(h));next(w for w in h['workspaces'] if w['workspace_id']==ws)['worktree']['checkout_path']=str(repo);(root/'herdr.json').write_text(json.dumps(h))
    before=len(calls(root));run(['remove','--session',id],env,repo,ok=False);assert not any(c[0]=='wt' and c[1][0]=='remove' for c in calls(root)[before:]);(root/'herdr.json').write_text(json.dumps(saved))
    run(['remove','--current'],dict(env,HERDR_WORKSPACE_ID=ws),target);assert target.exists();assert json.loads(path.read_text())['cleanup_pane'];run(['__remove',id],env,repo);assert not target.exists()
    # Worktrunk ownership is durable before Herdr open; hook failure never starts.
    path,id=launch(root,repo,env,'worktrunk');before=len(calls(root));run(['__run',id],dict(env,FIXTURE_OPEN_FAIL='1'),repo,ok=False)
    record=json.loads(path.read_text());assert record['receipt']['owned'] and record['receipt']['workspace'] is None
    assert not any(c[1][:2]==['agent','start'] for c in calls(root)[before:]);run(['remove','--session',id],env,repo)
    path,id=launch(root,repo,env,'worktrunk');before=len(calls(root));run(['__run',id],dict(env,FIXTURE_HOOK_FAIL='1'),repo,ok=False)
    assert not any(c[1][:2]==['agent','start'] for c in calls(root)[before:]);assert json.loads(path.read_text())['receipt'];run(['remove','--session',id],env,repo)
    # Delivery failure is never treated as confirmed and never automatically resent.
    for code,expected in [('agent_blocked','NotSent'),('agent_prompt_stalled','Unknown'),('timeout','Unknown')]:
        path,id=launch(root,repo,env);run(['__run',id],dict(env,FIXTURE_PROMPT_FAIL=code),repo,ok=False)
        assert json.loads(path.read_text())['delivery']==expected;run(['__run',id],env,repo,ok=False);run(['remove','--session',id],env,repo)
    # Configured executable uses the same receipt and pinned argv on removal.
    with (root/'config/config.toml').open('a') as f:f.write('\n[providers.fixture]\ncommand = '+json.dumps([shutil.which('python3'),str(ROOT/'examples/provider.py')])+'\n')
    path,id=launch(root,repo,env,'fixture');run(['__run',id],env,repo);record=json.loads(path.read_text());assert record['receipt']['owned']
    (root/'config/config.toml').write_text('[defaults]\nworkspace="herdr"\n');run(['remove','--session',id],env,repo)
    assert json.loads(path.read_text())['removal']['provider']=='fixture'
    # Malformed provider responses cannot cause fallback or a repeated prepare.
    (root/'config/config.toml').write_text(saved_config+'\n[providers.broken]\ncommand='+json.dumps([shutil.which('python3'),'-c','print("not JSON")'])+'\n')
    path,id=launch(root,repo,env,'broken');before=len(calls(root));run(['__run',id],env,repo,ok=False);run(['__run',id],env,repo,ok=False)
    record=json.loads(path.read_text());assert record['step']=='preparing' and record['receipt'] is None and record['error']
    assert not any(c[1][:2] in [['worktree','create'],['agent','start']] for c in calls(root)[before:])
    # A custom provider can report a validated partial receipt on failure.
    program='import runpy,sys;runpy.run_path('+repr(str(ROOT/'examples/provider.py'))+',run_name="__main__");sys.exit(1)'
    (root/'config/config.toml').write_text(saved_config+'\n[providers.partial]\ncommand='+json.dumps([shutil.which('python3'),'-c',program])+'\n')
    path,id=launch(root,repo,env,'partial');before=len(calls(root));run(['__run',id],env,repo,ok=False)
    assert json.loads(path.read_text())['receipt']['owned'];assert not any(c[1][:2]==['agent','start'] for c in calls(root)[before:])
    # Failed catalog sources retain configured entries; invalid suggestions stay advisory.
    source=saved_config.replace('allow_custom_model=true','allow_custom_model=true\ncatalog="command"\ncommand=["/missing/catalog-command"]')
    (root/'config/config.toml').write_text(source)
    cat=json.loads(run(['catalog','--json'],env,repo).stdout);assert cat['diagnostics'];assert cat['agents']['codex']['models'][0]['id']=='fixture-model'
    suggestion=json.dumps({'version':1,'suggestions':{'agent':'disabled-agent','repo':'missing','branch':'bad branch','effort':'invented','speed':'warp','provider':'missing'}})
    (root/'config/config.toml').write_text('prose_resolver='+json.dumps([shutil.which('python3'),'-c','print('+repr(suggestion)+')'])+'\n'+saved_config)
    path,id=launch(root,repo,env);record=json.loads(path.read_text());assert record['request']['diagnostics'];assert record['request']['agent']=='codex'
    # Tab defaults are independent of the worktree provider and require no wt.
    tab_config=saved_config.replace('[defaults]','[defaults]\nlaunch_mode="tab"\nworkspace="worktrunk"')
    (root/'config/config.toml').write_text(tab_config);(root/'bin/wt').unlink()
    before_worktrees=subprocess.check_output(['git','-C',str(repo),'worktree','list','--porcelain'])
    before_refs=subprocess.check_output(['git','-C',str(repo),'show-ref'])
    (repo/'keep-dirty.txt').write_text('Shared checkout work must survive tab cleanup.')
    tab_records=[]
    for text in ['Review one','Review two']:
        before=len(calls(root));path,id=launch(root,repo,env,provider=None,task=text);run(['__run',id],env,repo)
        record=json.loads(path.read_text());receipt=record['receipt'];tab_records.append((path,id,receipt))
        assert record['request']['launch_mode']=='tab' and receipt['owned'] is False
        assert pathlib.Path(receipt['checkout'])==repo.resolve() and receipt['tab']
        assert not any(c[0]=='wt' or c[1][:2] in [['worktree','create'],['worktree','open']] for c in calls(root)[before:])
        assert any(c[1][:2]==['tab','focus'] and c[1][2]==receipt['tab'] for c in calls(root)[before:])
    assert tab_records[0][2]['tab']!=tab_records[1][2]['tab']
    # Explicit worktree mode overrides a tab default; explicit tab overrides worktree.
    path,id=launch(root,repo,env,extra=['--launch-mode','worktree']);assert json.loads(path.read_text())['request']['launch_mode']=='worktree'
    (root/'config/config.toml').write_text(saved_config)
    path,id=launch(root,repo,env,extra=['--launch-mode','tab']);assert json.loads(path.read_text())['request']['launch_mode']=='tab'
    for extra in [['--launch-mode','invalid'],['--launch-mode','tab','--branch','fresh'],['--launch-mode','tab','--base','main']]:
        run(['launch',*extra,'task'],env,repo,ok=False)
    # --current targets the calling tab, not another task sharing its checkout.
    path,id,receipt=tab_records[0];ws=receipt['workspace'];tab=receipt['tab']
    run(['remove','--current'],dict(env,HERDR_WORKSPACE_ID=ws,HERDR_TAB_ID=ws+':unrelated'),repo,ok=False)
    h=json.loads((root/'herdr.json').read_text());saved=json.loads(json.dumps(h));next(t for t in h['tabs'] if t['tab_id']==tab)['pane_id']='moved';(root/'herdr.json').write_text(json.dumps(h))
    before=len(calls(root));run(['remove','--session',id],env,repo,ok=False)
    assert not any(c[1][:2]==['tab','close'] for c in calls(root)[before:]);(root/'herdr.json').write_text(json.dumps(saved))
    run(['remove','--current'],dict(env,HERDR_WORKSPACE_ID=ws,HERDR_TAB_ID=tab),repo)
    assert json.loads(path.read_text())['cleanup_pane'];run(['__remove',id],env,repo)
    assert json.loads(path.read_text())['step']=='removed'
    remaining=json.loads((root/'herdr.json').read_text());assert any(t['tab_id']==tab_records[1][2]['tab'] for t in remaining['tabs']);assert any(w['workspace_id']==ws for w in remaining['workspaces'])
    assert (repo/'keep-dirty.txt').read_text()=='Shared checkout work must survive tab cleanup.'
    assert subprocess.check_output(['git','-C',str(repo),'worktree','list','--porcelain'])==before_worktrees
    assert subprocess.check_output(['git','-C',str(repo),'show-ref'])==before_refs
    # A selected linked checkout is retained too, rather than replaced by primary.
    linked=root/'existing-checkout';subprocess.run(['git','-C',str(repo),'worktree','add','-b','existing-tab-branch',str(linked)],check=True,capture_output=True)
    path,id=launch(root,linked,env,extra=['--launch-mode','tab']);run(['__run',id],env,linked);receipt=json.loads(path.read_text())['receipt'];assert pathlib.Path(receipt['checkout'])==linked.resolve()
    run(['__remove',id],env,repo);assert linked.exists()
print('Acceptance passed: launch defaults and overrides, shared-checkout tabs, tab-specific cleanup, native worktrees, Worktrunk hooks, custom providers, delivery states, replay guards, literal input.')

with tempfile.TemporaryDirectory(prefix='composer-naming-') as tmp:
    root=pathlib.Path(tmp);repo,env=setup(root)
    base_config=(root/'config/config.toml').read_text()
    naming_config=base_config+'\n[branch_naming]\nenabled=true\nmodel="fixture-namer"\neffort="medium"\nspeed="fast"\nprefix="test/"\n'
    (root/'config/config.toml').write_text(naming_config)
    def naming_calls():return [args for program,args in calls(root) if program=='codex' and args[:1]==['exec']]
    literal='Fix `login`\n$(touch never)\n日本語\n'
    path,id=launch(root,repo,env,task=literal);record=json.loads(path.read_text())
    assert record['request']['branch']=='test/fix-login-redirect'
    assert json.loads((root/'naming-input.json').read_text())=={'task':literal}
    args=naming_calls()[-1];assert args[args.index('--model')+1]=='fixture-namer'
    assert 'model_reasoning_effort="medium"' in args and 'service_tier="fast"' in args
    assert '--ephemeral' in args and args[args.index('--sandbox')+1]=='read-only'
    assert literal not in ' '.join(args)
    assert record['request']['native_args']==['--model','fixture-model','-c','model_reasoning_effort="high"','-c','service_tier="default"']
    run(['__run',id],env,repo)
    # A generated name that already exists falls back without failing the task.
    path,id=launch(root,repo,env);r=json.loads(path.read_text())['request'];assert r['branch'].startswith('task-') and r['diagnostics']
    before=len(naming_calls())
    path,id=launch(root,repo,env,extra=['--branch','manual-name']);assert json.loads(path.read_text())['request']['branch']=='manual-name'
    run(['launch','branch:inline-name Fix login'],env,repo)
    path,id=launch(root,repo,env,extra=['--launch-mode','tab']);assert json.loads(path.read_text())['request']['branch']=='main'
    run(['launch','--model','invalid','task'],env,repo,ok=False)
    assert len(naming_calls())==before,'explicit branches, tab mode, and invalid requests must skip naming'
    for extra_env in [{'FIXTURE_BRANCH_NAME':'../invalid'},{'FIXTURE_NAMING_FAIL':'1'},{'FIXTURE_NAMING_EMPTY':'1'}]:
        path,id=launch(root,repo,dict(env,**extra_env));r=json.loads(path.read_text())['request']
        assert r['branch'].startswith('task-') and any('Branch naming failed' in d for d in r['diagnostics'])
    (root/'config/config.toml').write_text(naming_config.replace('enabled=true','enabled=false'))
    before=len(naming_calls());path,id=launch(root,repo,env);assert len(naming_calls())==before
    assert json.loads(path.read_text())['request']['branch'].startswith('task-')
print('Branch naming passed: separate model settings, literal input, precedence, collision/failure fallback, disabled mode.')
