#!/usr/bin/env python3
"""Protocol v1 fixture provider. Use only with disposable repositories."""
import json, pathlib, subprocess, sys

request=json.load(sys.stdin)
if request['version']!=1: raise SystemExit('unsupported protocol version')
def git(repo,*args):
    return subprocess.check_output(['git','-C',str(repo),*args],text=True).strip()
if request['operation']=='prepare':
    w=request['workspace'];repo=pathlib.Path(w['repository'])
    checkout=repo.parent/('composer-'+request['launch_id'])
    git(repo,'worktree','add','-b',w['branch'],str(checkout),w['base_commit'] or 'HEAD')
    receipt={'version':1,'launch_id':request['launch_id'],'checkout':str(checkout.resolve()),
             'branch':w['branch'],'owned':True,'workspace':None,'pane':None,
             'prepared_head':git(checkout,'rev-parse','HEAD'),'cleanup':{'repository':str(repo)}}
    print(json.dumps({'version':1,'status':'prepared','receipt':receipt}))
elif request['operation']=='remove':
    r=request['receipt']
    git(r['cleanup']['repository'],'worktree','remove',r['checkout'])
    print(json.dumps({'version':1,'status':'removed','branch_kept':True}))
else: raise SystemExit('unsupported operation')
