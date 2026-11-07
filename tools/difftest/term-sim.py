import os, pty, sys, time, select, termios, struct, fcntl
mode = sys.argv[1]; binpath = sys.argv[2]
resp = {"none": [], "alacritty": [b"\x1b[?1;2c", b"\x1b[?5u"], "xterm": [b"\x1b[?1;2c"]}[mode]
pid, fd = pty.fork()
if pid == 0:
    os.chdir("/tmp/j1work")
    env = dict(os.environ, HOME="/tmp/j1home", PI_CODING_AGENT_DIR="/tmp/j1cfg", PI_CODING_AGENT_SESSION_DIR="/tmp/j1sess")
    argv = binpath.split()
    os.execvpe(argv[0], argv, env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
out=b""; sent=False; killed=False; t=time.time()
while time.time()-t < 12:
    r,_,_ = select.select([fd],[],[],0.2)
    if r:
        try: d=os.read(fd,65536)
        except OSError: break
        if not d: break
        out+=d
        if not sent and b"\x1b[c" in out:
            for x in resp: os.write(fd,x)
            sent=True
        if b"ctrl+o more" in out and not sent:
            pass
        if (b"(auto)" in out) and not killed:
            try: os.kill(pid, 15)
            except OSError: pass
            killed = True
            t = time.time() - 4
open(f"/tmp/out-{mode}.bin","wb").write(out)
try:
    os.kill(pid, 9); os.waitpid(pid, 0)
except OSError:
    pass
