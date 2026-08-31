"""Focused PTY flow through the real editor, resolver, drafts and runner."""
import fcntl,json,os,pty,select,shutil,signal,struct,subprocess,tempfile,termios,time,zlib
from pathlib import Path
root=Path(__file__).resolve().parents[1];binary=root/'target/debug/herdr-composer'
with tempfile.TemporaryDirectory(prefix='composer-pty-') as tmp:
    tmp=Path(tmp);repo=tmp/'repo';repo.mkdir();config=tmp/'config';config.mkdir();bin=tmp/'bin';bin.mkdir()
    subprocess.run(['git','init','-b','main',str(repo)],check=True,capture_output=True)
    subprocess.run(['git','-C',str(repo),'-c','user.name=Test','-c','user.email=test@example.invalid','commit','--allow-empty','-m','initial'],check=True,capture_output=True)
    for name in ['herdr','wt','codex']:
        shutil.copy(root/'tests/fixture_tool.py',bin/name);(bin/name).chmod(0o755)
    for name in ['git','python3']:(bin/name).symlink_to(shutil.which(name))
    (config/'config.toml').write_text('[defaults]\nlaunch_mode="tab"\nagent="codex"\n[agents.codex]\n[[agents.codex.models]]\nid="fixture"\nlabel="Configured model"\nefforts=["low","deep"]\n[branch_naming]\nenabled=true\nmodel="fixture-namer"\n')
    # Use an isolated catalog and wait for its displayed diagnostic before input.
    # A partial first frame is visible before asynchronous discovery completes.
    catalog_command=['python3','-c','import sys; print(\'{"version":1,"models":[]}\'); print("CATALOG_READY", file=sys.stderr)']
    settings=(config/'config.toml').read_text().replace('[agents.codex]\n','[agents.codex]\ncatalog="command"\ncommand='+json.dumps(catalog_command)+'\n')
    (config/'config.toml').write_text(settings)
    env=dict(os.environ,TERM='xterm-256color',PATH=str(bin),COMPOSER_CONFIG_DIR=str(config),COMPOSER_STATE_DIR=str(tmp/'state'),HERDR_SOCKET_PATH=str(tmp/'socket'),HERDR_BIN_PATH=str(bin/'herdr'),FIXTURE_ROOT=str(tmp))
    for key in ['HERDR_PLUGIN_CONFIG_DIR','HERDR_PLUGIN_STATE_DIR','HERDR_ENV','HERDR_PANE_ID','COMPOSER_INVOKING_CHECKOUT']:env.pop(key,None)
    def start(remote=False):
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(repo)
            childenv=dict(env)
            if remote:childenv['SSH_CONNECTION']='192.0.2.1 1234 192.0.2.2 22'
            else:childenv.pop('SSH_CONNECTION',None);childenv.pop('SSH_TTY',None)
            os.execve(str(binary),[str(binary)],childenv)
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',32,110,0,0))
        data=b'';deadline=time.monotonic()+5
        while b'CATALOG_READY' not in data and time.monotonic()<deadline:
            if select.select([fd],[],[],.1)[0]:data+=os.read(fd,65536)
        assert b'CATALOG_READY' in data,repr(data[-1000:])
        return pid,fd
    def finish(pid,fd):
        deadline=time.monotonic()+8;data=b''
        while time.monotonic()<deadline:
            done,status=os.waitpid(pid,os.WNOHANG)
            if done:os.close(fd);assert os.waitstatus_to_exitcode(status)==0,(status,data[-2000:]);return
            if select.select([fd],[],[],.05)[0]:
                try:data+=os.read(fd,65536)
                except OSError:pass
        os.kill(pid,signal.SIGTERM);raise AssertionError('editor did not exit: '+repr(data[-3000:]))
    def draftpath():return next((tmp/'state/drafts').glob('*.json'))
    def draft():return json.loads(draftpath().read_text())
    def paste(fd,text):os.write(fd,b'\x1b[200~'+text.encode()+b'\x1b[201~');time.sleep(.15)
    def send(fd,keys):os.write(fd,keys);time.sleep(.1)
    def deliver():
        record=max((tmp/'state/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
        result=subprocess.run([str(binary),'__run',json.loads(record.read_text())['id']],env=env,cwd=repo,text=True,capture_output=True)
        assert result.returncode==0,result.stderr
        assert json.loads(record.read_text())['delivery']=='Confirmed'
    task='Fix `auth`\n$(echo literal)\n日本語 🐑'
    pid,fd=start();paste(fd,task)
    send(fd,b'\x0c\x0a\r');send(fd,b'j\r') # override configured Tab with New worktree
    send(fd,b'\x0a\r');send(fd,b'jj\r') # provider -> Worktrunk
    send(fd,b'\x0a\r');send(fd,b'j\r') # agent -> Codex
    send(fd,b'\x0a\r');send(fd,b'j\r') # configured model
    send(fd,b'\x1b');finish(pid,fd)
    saved=draft();assert saved['launch_mode']=='worktree';assert saved['task']==task;assert saved['provider']=='worktrunk';assert saved['agent']=='codex';assert saved['model']=='fixture'
    pid,fd=start();send(fd,b'\x13');finish(pid,fd)
    assert draft()['task']==task,'queued tasks must retain their draft'
    calls=[json.loads(line) for line in (tmp/'calls.jsonl').read_text().splitlines()]
    assert not any(program=='codex' and args[:1]==['exec'] for program,args in calls),'editor must close before naming runs'
    deliver();assert draft()['task']==''
    def png():
        def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
        return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0'+b'\xff\0\0'*2+b'\0'+b'\0\xff\0'*2))+chunk(b'IEND',b'')
    original=tmp/'original image.png';payload=png();original.write_bytes(payload)
    pid,fd=start(remote=True);paste(fd,"'"+str(original)+"'");paste(fd,"'"+str(original)+"'");send(fd,b'\x1b');finish(pid,fd)
    saved=draft();assert saved['task']=='';assert len(saved['attachments'])==1
    retained=Path(saved['attachments'][0]['path']);original.unlink();assert retained.read_bytes()==payload
    pid,fd=start(remote=True);send(fd,b'\x0a\r');send(fd,b'\x08');send(fd,b'\x1b');finish(pid,fd) # preview closes independently
    pid,fd=start(remote=True);send(fd,b'\x13');finish(pid,fd);deliver();assert draft()['attachments']==[];assert retained.read_bytes()==payload
    # Local clipboard uses fixture bytes; the desktop clipboard is untouched.
    clipboard=tmp/'clipboard.png';clipboard.write_bytes(payload)
    for name in ['pngpaste','wl-paste','xclip']:
        reader=bin/name;reader.write_text('#!'+shutil.which('python3')+'\nimport pathlib,sys\nsys.stdout.buffer.write(pathlib.Path('+repr(str(clipboard))+').read_bytes())\n');reader.chmod(0o755)
    pid,fd=start();send(fd,b'\x16');time.sleep(.3);send(fd,b'\x1b');finish(pid,fd);assert len(draft()['attachments'])==1
    pid,fd=start();send(fd,b'\x0a\x1b[3~');send(fd,b'\x1b');finish(pid,fd);assert draft()['attachments']==[];assert retained.read_bytes()==payload
    pid,fd=start();paste(fd,'Review this checkout');send(fd,b'\x0c\x0a\r');send(fd,b'G\r');send(fd,b'\x1b');finish(pid,fd)
    assert draft()['launch_mode']=='tab'
    pid,fd=start();send(fd,b'\x13');finish(pid,fd);deliver()
    latest=max((tmp/'state/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
    receipt=json.loads(latest.read_text())['receipt'];assert receipt['owned'] is False and receipt['tab'];assert Path(receipt['checkout'])==repo.resolve()
print('PTY passed: launch-mode toggle and restoration, provider/catalog selection, literal editing, draft recovery, image-only tasks, preview, remote/local import, retained originals.')
