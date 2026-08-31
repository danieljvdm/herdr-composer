#!/usr/bin/env python3
"""Fake external tools with real disposable Git worktrees and durable call logs."""
import json, os, pathlib, subprocess, sys, time

root=pathlib.Path(os.environ['FIXTURE_ROOT'])
state_path=root/'herdr.json'
state=json.loads(state_path.read_text()) if state_path.exists() else {'workspaces':[], 'tabs':[], 'next':1, 'next_tab':2}
args=sys.argv[1:]
program=pathlib.Path(sys.argv[0]).name
with (root/'calls.jsonl').open('a') as f: f.write(json.dumps([program,args])+ '\n')
def git(cwd,*args):
    p=subprocess.run(['git','-C',str(cwd),*args],text=True,capture_output=True)
    if p.returncode: raise RuntimeError(p.stderr)
    return p.stdout.strip()
def flag(name,default=None):
    return args[args.index(name)+1] if name in args else default
def emit(result):
    state_path.write_text(json.dumps(state));print(json.dumps({'result':result}))
def workspace(path,repo):
    wid='w'+str(state['next']);state['next']+=1
    w={'workspace_id':wid,'worktree':{'checkout_path':str(path),'repo_root':str(repo)}}
    state['workspaces'].append(w)
    state['tabs'].append({'tab_id':wid+':t1','workspace_id':wid,'pane_id':wid+':p1','cwd':str(path)})
    return {'workspace':w,'root_pane':{'pane_id':wid+':p1'}}
def prepare(repo,branch,base):
    target=(root/'checkouts'/branch).resolve();target.parent.mkdir(parents=True,exist_ok=True)
    git(repo,'worktree','add','-b',branch,str(target),base or 'HEAD')
    return target
try:
    if program=='wt':
        repo=pathlib.Path.cwd()
        if args[0]=='switch':
            target=prepare(repo,args[2],flag('--base'))
            if os.environ.get('FIXTURE_HOOK_FAIL'):
                print(json.dumps({'path':str(target)}));print('blocking hook failed',file=sys.stderr);sys.exit(1)
            (root/'hook-finished').write_text('done');print(json.dumps({'path':str(target)}))
        elif args[0]=='remove': git(repo,'worktree','remove',args[1]);print('branch kept')
    elif program=='codex':
        if args and args[0]=='exec':
            (root/'naming-input.json').write_text(sys.stdin.read())
            if os.environ.get('FIXTURE_NAMING_FAIL'): raise RuntimeError('naming unavailable')
            if os.environ.get('FIXTURE_NAMING_EMPTY'): sys.exit(0)
            print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':os.environ.get('FIXTURE_BRANCH_NAME','fix-login-redirect')}}))
    elif program=='herdr':
        command=args[:2]
        if command==['workspace','list']: emit({'workspaces':state['workspaces']})
        elif command==['pane','list']: emit({'panes':[t for t in state['tabs'] if flag('--workspace') in [None,t['workspace_id']]]})
        elif command==['worktree','list']:
            repo=next(w['worktree']['repo_root'] for w in state['workspaces'] if w['workspace_id']==flag('--workspace'));emit({'source':{'repo_root':repo},'worktrees':[]})
        elif command==['workspace','create']:
            checkout=pathlib.Path(flag('--cwd'));repo=pathlib.Path(git(checkout,'worktree','list','--porcelain').splitlines()[0].removeprefix('worktree '));emit(workspace(checkout,repo))
        elif command==['tab','create']:
            ws=flag('--workspace');number=str(state['next_tab']);state['next_tab']+=1
            t={'tab_id':ws+':t'+number,'workspace_id':ws,'pane_id':ws+':p'+number,'cwd':flag('--cwd')};state['tabs'].append(t)
            emit({'tab':t,'root_pane':{'pane_id':t['pane_id']}})
        elif command==['tab','list']: emit({'tabs':[t for t in state['tabs'] if flag('--workspace') in [None,t['workspace_id']]]})
        elif command==['tab','focus']: emit({'type':'tab_focused'})
        elif command==['tab','close']:
            if os.environ.get('FIXTURE_CLOSE_FAIL'): raise RuntimeError('close failed')
            state['tabs']=[t for t in state['tabs'] if t['tab_id']!=args[2]];emit({'type':'tab_closed'})
        elif command==['pane','run']: emit({'type':'pane_input_sent'})
        elif command==['worktree','create']:
            repo=pathlib.Path(next(w['worktree']['repo_root'] for w in state['workspaces'] if w['workspace_id']==flag('--workspace')));target=prepare(repo,flag('--branch'),flag('--base'))
            v=workspace(target,repo);v['worktree']={'path':str(target)};v['type']='worktree_created';emit(v)
        elif command==['worktree','open']:
            if os.environ.get('FIXTURE_OPEN_FAIL'): raise RuntimeError('open failed')
            repo=next(w['worktree']['repo_root'] for w in state['workspaces'] if w['workspace_id']==flag('--workspace'))
            v=workspace(pathlib.Path(flag('--path')),pathlib.Path(repo));v['type']='worktree_opened';emit(v)
        elif command==['worktree','remove']:
            w=next(w for w in state['workspaces'] if w['workspace_id']==flag('--workspace'))
            git(w['worktree']['repo_root'],'worktree','remove',w['worktree']['checkout_path']);state['workspaces'].remove(w);emit({'type':'worktree_removed'})
        elif command==['workspace','close']:
            if os.environ.get('FIXTURE_CLOSE_FAIL'): raise RuntimeError('close failed')
            state['workspaces']=[w for w in state['workspaces'] if w['workspace_id']!=args[2]];emit({'type':'workspace_closed'})
        elif command==['workspace','focus']: emit({'type':'workspace_focused'})
        elif command==['agent','list']: emit({'agents':[]})
        elif command==['agent','start']:
            if os.environ.get('FIXTURE_START_FAIL'): raise RuntimeError('agent failed to become ready')
            emit({'type':'agent_started','agent':{'pane_id':flag('--pane'),'agent':'codex','name':args[2]},'argv':args})
        elif command==['agent','prompt']:
            if os.environ.get('FIXTURE_PROMPT_FAIL'):
                print(json.dumps({'error':{'code':os.environ['FIXTURE_PROMPT_FAIL']}}),file=sys.stderr);sys.exit(1)
            (root/'prompt.json').write_text(json.dumps(args[3]));emit({'type':'agent_prompted','agent':{'pane_id':args[2],'agent_status':'working'}})
        else: raise RuntimeError('unexpected Herdr command '+repr(args))
    else: raise RuntimeError('unexpected fixture tool '+program)
except Exception as e:
    print(str(e),file=sys.stderr);sys.exit(1)
