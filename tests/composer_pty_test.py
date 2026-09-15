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
        assert b'\x1b[?1002h' in data, 'drag tracking must remain enabled'
        assert b'\x1b[?1003h' not in data, 'unused hover tracking floods remote input'
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
    def collect(fd,seconds):
        data=b'';deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            if select.select([fd],[],[],max(0,deadline-time.monotonic()))[0]:data+=os.read(fd,65536)
        return data
    def expect_output(fd,marker):
        data=b'';deadline=time.monotonic()+2
        while marker not in data and time.monotonic()<deadline:
            if select.select([fd],[],[],.05)[0]:data+=os.read(fd,65536)
        assert marker in data,repr(data[-2000:])
        assert b'\x1b[6n' not in data, 'rendering must not wait for cursor-position replies'
    def latest_session():return max((tmp/'state/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
    def deliver(record=None):
        if record is None:record=latest_session()
        result=subprocess.run([str(binary),'__run',json.loads(record.read_text())['id']],env=env,cwd=repo,text=True,capture_output=True)
        assert result.returncode==0,result.stderr
        assert json.loads(record.read_text())['delivery']=='Confirmed'
    pid,fd=start(remote=True)
    collect(fd,.3)
    assert collect(fd,.4)==b'', 'idle editor should not produce terminal traffic'
    # A host can invalidate the screen without changing the final PTY size.
    os.kill(pid,signal.SIGWINCH);expect_output(fd,b'New task')
    for rows,cols in [(8,30),(32,110)]*5:
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
        expect_output(fd,b'Enlarge the pane' if cols==30 else b'New task')
    collect(fd,.2)
    # Ignore any stale hover events already in flight, but still process input.
    os.write(fd,b'\x1b[<35;10;10M'*20)
    assert collect(fd,.2)==b'', 'hover events must not trigger redraws'
    os.write(fd,b'z');expect_output(fd,b'z')
    # Autosave must still persist and render feedback after the idle timer fires.
    assert collect(fd,.6), 'autosave feedback must be redrawn'
    assert draft()['task']=='z'
    send(fd,b'\x7f\x1b');finish(pid,fd)
    assert draft()['task']==''
    task='Fix `auth`\n$(echo literal)\n日本語 🐑'
    pid,fd=start();paste(fd,task)
    send(fd,b'\x0c\x0a\r');send(fd,b'j\r') # override configured Tab with New worktree
    send(fd,b'\x0a\r');send(fd,b'jj\r') # provider -> Worktrunk
    send(fd,b'\x0a\r');send(fd,b'j\r') # agent -> Codex
    send(fd,b'\x0a\r');send(fd,b'j\r') # configured model
    send(fd,b'\x1b');finish(pid,fd)
    saved=draft();assert saved['launch_mode']=='worktree';assert saved['task']==task;assert saved['provider']=='worktrunk';assert saved['agent']=='codex';assert saved['model']=='fixture'
    env['FIXTURE_HANDOFF_FAIL']='1'
    pid,fd=start();send(fd,b'\x13');expect_output(fd,b'attention:');send(fd,b'\x1b');finish(pid,fd)
    env.pop('FIXTURE_HANDOFF_FAIL')
    assert draft()==saved,'unsuccessful handoff must preserve the draft'
    pid,fd=start();send(fd,b'\x13');finish(pid,fd)
    first_session=latest_session()
    assert json.loads(first_session.read_text())['request']['task']==task
    assert draft()['task']=='','handoff must clear the draft before the runner starts'
    assert draft()['model']==saved['model'] and draft()['provider']==saved['provider']
    calls=[json.loads(line) for line in (tmp/'calls.jsonl').read_text().splitlines()]
    assert not any(program=='codex' and args[:1]==['exec'] for program,args in calls),'editor must close before naming runs'
    # Reopen immediately and queue another task while the first is pending.
    next_task='Review a second task'
    pid,fd=start();paste(fd,next_task);send(fd,b'\x13');finish(pid,fd)
    second_session=latest_session()
    assert second_session!=first_session
    assert json.loads(second_session.read_text())['request']['task']==next_task
    assert draft()['task']==''
    # Older runners must not erase an unsent draft, even completing out of order.
    pid,fd=start();paste(fd,'Unsent third task');send(fd,b'\x1b');finish(pid,fd)
    unsent=draft()
    deliver(second_session);deliver(first_session)
    assert draft()==unsent
    pid,fd=start();send(fd,b'\x7f'*len(unsent['task'])+b'\x1b');finish(pid,fd)
    assert draft()['task']==''
    def png():
        def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
        return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0'+b'\xff\0\0'*2+b'\0'+b'\0\xff\0'*2))+chunk(b'IEND',b'')
    original=tmp/'original image.png';payload=png();original.write_bytes(payload)
    pid,fd=start(remote=True);paste(fd,"'"+str(original)+"'");paste(fd,"'"+str(original)+"'");send(fd,b'\x1b');finish(pid,fd)
    saved=draft();assert saved['task']=='';assert len(saved['attachments'])==1
    retained=Path(saved['attachments'][0]['path']);original.unlink();assert retained.read_bytes()==payload
    pid,fd=start(remote=True);send(fd,b'\x0a\r');send(fd,b'\x08');send(fd,b'\x1b');finish(pid,fd) # preview closes independently
    pid,fd=start(remote=True);send(fd,b'\x13');finish(pid,fd)
    assert draft()['attachments']==[],'handoff must clear images before delivery'
    assert json.loads(latest_session().read_text())['request']['attachments']==saved['attachments']
    deliver();assert draft()['attachments']==[];assert retained.read_bytes()==payload
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
    # A real image stream with a deliberately stalled metadata response must
    # not block typing. All sockets and image bytes belong to this fixture.
    import queue,socket,threading
    listener=socket.socket(socket.AF_UNIX);listener.bind(str(tmp/'socket'));listener.listen();listener.settimeout(3)
    env.update(HERDR_ENV='1',HERDR_PANE_ID='w1:p1')
    pid,fd=start(remote=True) # fork before starting the fixture thread
    requested=threading.Event();release=threading.Event();closed=threading.Event()
    frames=queue.Queue();errors=[]
    def wait_graphics(predicate):
        deadline=time.monotonic()+2
        while not predicate() and time.monotonic()<deadline:collect(fd,.01)
        assert predicate(),errors
    def graphics_server():
        try:
            with listener:
                conn,_=listener.accept()
                with conn,conn.makefile('rb') as reader:
                    assert json.loads(reader.readline())['method']=='pane.graphics.info'
                    requested.set();assert release.wait(2)
                    conn.sendall(b'{"result":{"cell_width_px":10,"cell_height_px":20}}\n')
                conn,_=listener.accept()
                with conn,conn.makefile('rb') as reader:
                    conn.settimeout(3)
                    assert json.loads(reader.readline())['method']=='pane.graphics.stream'
                    conn.sendall(b'{"result":{"type":"ok"}}\n')
                    while line:=reader.readline():
                        header=json.loads(line)
                        assert len(reader.read(header['data_length']))==header['data_length']
                        frames.put(header)
                closed.set()
        except Exception as error:errors.append(error)
    server=threading.Thread(target=graphics_server,daemon=True);server.start()
    os.write(fd,b'\x1b[200~'+str(clipboard).encode()+b'\x1b[201~')
    wait_graphics(requested.is_set)
    try:
        started=time.monotonic();os.write(fd,b'Z');expect_output(fd,b'Z')
        assert time.monotonic()-started<.25, 'typing waited for the graphics API'
    finally:release.set()
    wait_graphics(lambda:not frames.empty());first=frames.get_nowait()
    collect(fd,.3)
    os.write(fd,b'Q');expect_output(fd,b'Q')
    collect(fd,.6) # autosave and the pixel/fallback transition settle
    assert collect(fd,.2)==b'', 'attached images must not trigger idle redraws'
    assert frames.empty(), 'typing must not upload unchanged images'
    # Several resizes can queue while graphics work is pending. The final
    # placement must catch up, and removal must close the owned stream.
    for cols in [100,90,120]:
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',32,cols,0,0))
    expect_output(fd,b'New task')
    wait_graphics(lambda:not frames.empty());resized=frames.get_nowait()
    assert resized['placement']!=first['placement']
    send(fd,b'\x0a\x1b[3~')
    wait_graphics(closed.is_set)
    send(fd,b'\x1b');finish(pid,fd)
    server.join(timeout=2);assert not server.is_alive() and not errors,errors
    assert draft()['task']=='ZQ' and draft()['attachments']==[]
print('PTY passed: editing, launch flows, draft recovery, retained images, responsive graphics, idle output, resize and image-stream cleanup.')
