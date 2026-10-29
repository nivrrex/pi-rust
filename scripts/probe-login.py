import os, pty, select, time, json, tempfile, re, fcntl, struct, termios
cfg = tempfile.mkdtemp()
open(os.path.join(cfg,"models.json"),"w").write(json.dumps({"providers":{"mockp":{"api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1","apiKey":"sk-stub"}}}))
open(os.path.join(cfg,"settings.json"),"w").write("{}")
pid, fd = pty.fork()
if pid == 0:
    os.environ["PI_CODING_AGENT_DIR"]=cfg; os.environ["TERM"]="xterm-256color"
    os.execv("./target/debug/pi-rs",["./target/debug/pi-rs"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
buf=b""
def pump(t):
    global buf
    end=time.time()+t
    while time.time()<end:
        r,_,_=select.select([fd],[],[],0.2)
        if r:
            try: buf+=os.read(fd,65536)
            except OSError: return
def wait(marker, timeout=25):
    end=time.time()+timeout
    while time.time()<end:
        if marker.encode() in buf: return True
        pump(0.2)
    return False
def txt():
    return re.sub(rb'\x1b\[[0-9;?]*[a-zA-Z]',b'',buf).decode('utf-8','replace').replace('\x1b]8;;\x07','')
r={}
r["startup"]=wait("no-model")
os.write(fd,b"/login mockp"); pump(0.4); os.write(fd,b"\x1b"); pump(0.3); os.write(fd,b"\r"); r["login_prompt"]=wait("Logging in to mockp")
os.write(fd,b"sk-test-123\r"); r["logged_in"]=wait("Logged in to mockp")
os.write(fd,b"/quit\r"); pump(1.0)
p=os.path.join(cfg,"auth.json")
r["auth"]=os.path.exists(p)
r["mode"]=oct(os.stat(p).st_mode & 0o777) if r["auth"] else None
r["content"]=open(p).read().replace("\n","") if r["auth"] else None
print(json.dumps(r))
print("TAIL:", ' '.join(txt().split())[-260:])
os.kill(pid,9)
