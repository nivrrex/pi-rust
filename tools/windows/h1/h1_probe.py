"""H1 探针：验证"写入被按固定块切分 ⇒ 转义序列被切断 ⇒ 屏幕上出现 `0m`/`;;` 残留"。

本脚本不依赖 pi-rs：它自己拼一段字节写出去，再读回屏幕文本，检查有没有半截序列残留。

两种宿主：
  * conpty  —— WezTerm 窗格（用户实际路径；需要 WezTerm 在运行并可用 `wezterm cli`）
  * conhost —— 经典控制台窗口（CreateProcess(CREATE_NEW_CONSOLE)，读屏幕缓冲区）

用法：
  rustc -O h1.rs -o h1.exe
  python h1_probe.py --matrix core         # shape×mode
  python h1_probe.py --matrix boundary     # 4096 字节切点全位置
  python h1_probe.py --matrix linebuffer   # 1024 字节行缓冲切点
  python h1_probe.py --matrix all --json out.json

只读观测：不读取任何凭据，测试文本全为合成内容（`A`/`L` 填充）。
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import subprocess
import sys
import time

BASE = os.path.dirname(os.path.abspath(__file__))
EXE = os.path.join(BASE, "h1.exe")
WEZ = shutil.which("wezterm") or r"E:\compiler\WezTerm\wezterm.exe"

k32 = ctypes.WinDLL("kernel32", use_last_error=True)


class COORD(ctypes.Structure):
    _fields_ = [("X", ctypes.c_short), ("Y", ctypes.c_short)]


class SMALL_RECT(ctypes.Structure):
    _fields_ = [("Left", ctypes.c_short), ("Top", ctypes.c_short),
                ("Right", ctypes.c_short), ("Bottom", ctypes.c_short)]


class CONSOLE_SCREEN_BUFFER_INFO(ctypes.Structure):
    _fields_ = [("dwSize", COORD), ("dwCursorPosition", COORD),
                ("wAttributes", wt.WORD), ("srWindow", SMALL_RECT),
                ("dwMaximumWindowSize", COORD)]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("lpReserved", wt.LPWSTR), ("lpDesktop", wt.LPWSTR),
                ("lpTitle", wt.LPWSTR), ("dwX", wt.DWORD), ("dwY", wt.DWORD),
                ("dwXSize", wt.DWORD), ("dwYSize", wt.DWORD),
                ("dwXCountChars", wt.DWORD), ("dwYCountChars", wt.DWORD),
                ("dwFillAttribute", wt.DWORD), ("dwFlags", wt.DWORD),
                ("wShowWindow", wt.WORD), ("cbReserved2", wt.WORD),
                ("lpReserved2", ctypes.POINTER(ctypes.c_byte)),
                ("hStdInput", wt.HANDLE), ("hStdOutput", wt.HANDLE), ("hStdError", wt.HANDLE)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [("hProcess", wt.HANDLE), ("hThread", wt.HANDLE),
                ("dwProcessId", wt.DWORD), ("dwThreadId", wt.DWORD)]


k32.CreateProcessW.restype = wt.BOOL
k32.CreateProcessW.argtypes = [wt.LPCWSTR, wt.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
                               wt.BOOL, wt.DWORD, wt.LPCWSTR, wt.LPCWSTR,
                               ctypes.POINTER(STARTUPINFOW), ctypes.POINTER(PROCESS_INFORMATION)]
k32.CreateFileW.restype = wt.HANDLE
k32.CreateFileW.argtypes = [wt.LPCWSTR, wt.DWORD, wt.DWORD, ctypes.c_void_p, wt.DWORD,
                            wt.DWORD, wt.HANDLE]
k32.AttachConsole.restype = wt.BOOL
k32.AttachConsole.argtypes = [wt.DWORD]
k32.ReadConsoleOutputCharacterW.restype = wt.BOOL
k32.ReadConsoleOutputCharacterW.argtypes = [wt.HANDLE, wt.LPWSTR, wt.DWORD, COORD,
                                            ctypes.POINTER(wt.DWORD)]
k32.GetConsoleScreenBufferInfo.restype = wt.BOOL
k32.GetConsoleScreenBufferInfo.argtypes = [wt.HANDLE,
                                           ctypes.POINTER(CONSOLE_SCREEN_BUFFER_INFO)]

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
FILE_SHARE_READ = 0x1
FILE_SHARE_WRITE = 0x2
OPEN_EXISTING = 3
CREATE_NEW_CONSOLE = 0x00000010
CREATE_UNICODE_ENVIRONMENT = 0x00000400
CMD = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "cmd.exe")


def conout():
    handle = k32.CreateFileW("CONOUT$", GENERIC_READ | GENERIC_WRITE,
                             FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, 0, None)
    if not handle or handle == wt.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    return handle


def spawn_console(command_line):
    """新开一个经典控制台窗口跑 `command_line`（用 cmd /c 包一层，便于读取与清理）。"""
    si = STARTUPINFOW()
    si.cb = ctypes.sizeof(si)
    pi = PROCESS_INFORMATION()
    cmdline = ctypes.create_unicode_buffer('"%s" /c %s' % (CMD, command_line))
    env = dict(os.environ)
    envbuf = "\0".join("%s=%s" % kv for kv in env.items()) + "\0\0"
    ok = k32.CreateProcessW(CMD, cmdline, None, None, False,
                            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
                            envbuf, BASE, ctypes.byref(si), ctypes.byref(pi))
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())
    return pi


def read_console_buffer(pid, max_rows=400):
    k32.FreeConsole()
    if not k32.AttachConsole(pid):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        handle = conout()
        info = CONSOLE_SCREEN_BUFFER_INFO()
        k32.GetConsoleScreenBufferInfo(handle, ctypes.byref(info))
        width = info.dwSize.X
        height = min(info.dwSize.Y, max_rows)
        buf = ctypes.create_unicode_buffer(width + 1)
        read = wt.DWORD(0)
        lines = []
        for y in range(0, height):
            k32.ReadConsoleOutputCharacterW(handle, buf, width, COORD(0, y), ctypes.byref(read))
            lines.append(buf.value[: read.value].rstrip())
        k32.CloseHandle(handle)
        return lines, (width, height, info.srWindow.Bottom)
    finally:
        k32.FreeConsole()


def wez(*args, timeout=40):
    out = subprocess.run([WEZ] + list(args), capture_output=True, timeout=timeout)
    return out.stdout.decode("utf-8", "replace")


def judge(screen, target):
    """屏幕文本里 `target` 之后不应再出现 `0m` / `;;`；`<<END>>` 必须完整出现。"""
    index = screen.find("<<END>>")
    head = screen[:index] if index >= 0 else screen
    return {
        "end_found": index >= 0,
        "has_0m": ("[0m" in screen) or (target == "csi" and "0m" in head),
        "has_semisemis": (";;" in screen) or (target == "osc" and ";;" in head),
    }


def run_conpty(shape, mode, prefix):
    pane = wez("cli", "spawn", "--cwd", BASE, "--", CMD, "/k").strip()
    if not pane.isdigit():
        return {"host": "conpty", "shape": shape, "mode": mode, "prefix": prefix,
                "error": "spawn: %r" % pane}
    try:
        time.sleep(1.2)
        wez("cli", "send-text", "--no-paste", "--pane-id", pane, "--",
            '"%s" %s %s %d\r' % (EXE, shape, mode, prefix))
        time.sleep(2.0)
        screen = wez("cli", "get-text", "--pane-id", pane)
    finally:
        wez("cli", "kill-pane", "--pane-id", pane)
    row = {"host": "conpty", "shape": shape, "mode": mode, "prefix": prefix}
    row.update(judge(screen, shape))
    return row


def run_conhost(shape, mode, prefix):
    # h1.exe 写完即退出；用 ping 把控制台窗口拖住，否则 AttachConsole 拿不到
    pi = spawn_console('"%s" %s %s %d & ping -n 20 127.0.0.1 >nul' % (EXE, shape, mode, prefix))
    try:
        time.sleep(1.8)
        lines, meta = read_console_buffer(pi.dwProcessId)
        screen = "\n".join(lines)
    finally:
        subprocess.run(["taskkill", "/PID", str(pi.dwProcessId), "/T", "/F"],
                       capture_output=True)
        k32.CloseHandle(pi.hProcess)
        k32.CloseHandle(pi.hThread)
    row = {"host": "conhost", "shape": shape, "mode": mode, "prefix": prefix, "buffer": meta}
    row.update(judge(screen, shape))
    return row


def matrix(name):
    if name == "core":
        return [(shape, mode, prefix)
                for shape, prefix in (("csi", 4094), ("osc", 4094), ("real", 30),
                                      ("real-long", 3000))
                for mode in ("std", "wcw", "split")]
    if name == "boundary":
        return [(shape, "split", prefix)
                for shape in ("csi", "osc") for prefix in range(4088, 4098)]
    if name == "linebuffer":
        # 行首 19 字节真彩前缀 ⇒ ESC[0m 的起点 = 19+prefix，扫过 1024 字节行缓冲边界
        return [("real-long", "std", prefix) for prefix in (1003, 1004, 1005, 1006)]
    raise SystemExit("unknown matrix: %s" % name)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", default="core", choices=("core", "boundary", "linebuffer", "all"))
    parser.add_argument("--json", default="")
    parser.add_argument("--retries", type=int, default=3)
    args = parser.parse_args()
    if not os.path.exists(EXE):
        raise SystemExit("缺少 h1.exe：先运行 rustc -O h1.rs -o h1.exe")
    names = ("core", "boundary", "linebuffer") if args.matrix == "all" else (args.matrix,)
    rows = []
    for name in names:
        for shape, mode, prefix in matrix(name):
            for host, runner in (("conpty", run_conpty), ("conhost", run_conhost)):
                row = None
                for attempt in range(args.retries):
                    row = runner(shape, mode, prefix)
                    if row.get("end_found"):
                        break
                    time.sleep(0.5)
                row["attempts"] = attempt + 1
                row["matrix"] = name
                rows.append(row)
                print(json.dumps(row, ensure_ascii=False))
    bad = [r for r in rows if not r.get("end_found") or r.get("has_0m") or r.get("has_semisemis")]
    print("cases=%d bad=%d retried=%d" % (len(rows), len(bad), sum(1 for r in rows if r.get("attempts", 1) > 1)))
    if args.json:
        with open(args.json, "w", encoding="utf-8") as handle:
            json.dump(rows, handle, ensure_ascii=False, indent=1)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
