# probe-paste-markers.ps1 - does a REAL paste reach the app with bracketed-paste markers?
#
# Purpose (P8e-12/V3): decide if switching the Windows input path to VT bytes is worth it.
# We enable ENABLE_VIRTUAL_TERMINAL_PROCESSING on stdout (the app does this too), write
# ESC[?2004h so the console knows bracketed paste is on, then read RAW console input records
# while the driver sends a real Shift+Insert (SendInput). For every trial we report the
# characters actually delivered and whether ESC[200~ / ESC[201~ survived.
#
# Only console input is read; nothing else on the machine is touched.

param(
  [ValidateSet('default','vt')][string]$Mode = 'default',
  [int]$Trials = 20,
  [string]$Log = "$env:TEMP\paste-markers.log",
  [int]$TimeoutMs = 6000,
  [int]$IdleMs = 400,
  [string]$Title = ''
)

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class V3 {
  [StructLayout(LayoutKind.Sequential)]
  public struct KEY_EVENT_RECORD {
    public int bKeyDown;
    public ushort wRepeatCount;
    public ushort wVirtualKeyCode;
    public ushort wVirtualScanCode;
    public char UnicodeChar;
    public uint dwControlKeyState;
  }
  [StructLayout(LayoutKind.Explicit)]
  public struct INPUT_RECORD {
    [FieldOffset(0)] public ushort EventType;
    [FieldOffset(4)] public KEY_EVENT_RECORD KeyEvent;
  }
  [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool ReadConsoleInputW(IntPtr h, [Out] INPUT_RECORD[] buf, uint len, out uint read);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern IntPtr GetStdHandle(int n);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool GetConsoleMode(IntPtr h, out uint m);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool SetConsoleMode(IntPtr h, uint m);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool GetNumberOfConsoleInputEvents(IntPtr h, out uint n);

  public static string Drain(IntPtr h, int timeoutMs, int idleMs) {
    var sb = new StringBuilder();
    var buf = new INPUT_RECORD[256];
    var last = Environment.TickCount;
    int keyCount = 0, nonKey = 0;
    while (true) {
      uint avail = 0;
      if (GetNumberOfConsoleInputEvents(h, out avail) && avail > 0) {
        uint want = avail > (uint)buf.Length ? (uint)buf.Length : avail;
        uint got = 0;
        if (ReadConsoleInputW(h, buf, want, out got)) {
          for (uint i = 0; i < got; i++) {
            if (buf[i].EventType == 1) {
              keyCount++;
              char c = buf[i].KeyEvent.UnicodeChar;
              if (buf[i].KeyEvent.bKeyDown != 0 && c != '\0') sb.Append(c);
            } else {
              nonKey++;
            }
          }
          if (got > 0) last = Environment.TickCount;
        }
        continue;
      }
      int idle = Environment.TickCount - last;
      if (idle >= timeoutMs) break;
      if (idle >= idleMs && keyCount > 0) break;
      WaitForSingleObject(h, 50);
    }
    return keyCount + "|" + nonKey + "|" + sb.ToString();
  }
}
"@

$hIn = [V3]::GetStdHandle(-10)
$hOut = [V3]::GetStdHandle(-11)
$modeIn = 0
$modeOut = 0
[void][V3]::GetConsoleMode($hIn, [ref]$modeIn)
[void][V3]::GetConsoleMode($hOut, [ref]$modeOut)
[void][V3]::SetConsoleMode($hOut, $modeOut -bor 0x0004)

if ($Mode -eq 'vt') {
  $newIn = $modeIn -bor 0x0200
} else {
  $newIn = $modeIn -band (-bnot 0x0200)
}
$ok = [V3]::SetConsoleMode($hIn, $newIn)
$modeIn2 = 0
[void][V3]::GetConsoleMode($hIn, [ref]$modeIn2)

if ($Title -eq '') { $Title = "pi-v3-probe-$Mode" }
[Console]::Title = $Title
"mode=$Mode vt_input=$([bool]($modeIn2 -band 0x0200)) set_ok=$ok in=0x$('{0:X}' -f $modeIn2) out=0x$('{0:X}' -f $modeOut) title=$([Console]::Title)" | Set-Content -Path $Log -Encoding UTF8

# Enable bracketed paste exactly like the app does.
[Console]::Write("$([char]27)[?2004h")

$esc = [string][char]27
for ($i = 1; $i -le $Trials; $i++) {
  Add-Content -Path $Log -Value "READY $i" -Encoding UTF8
  $raw = [V3]::Drain($hIn, $TimeoutMs, $IdleMs)
  $parts = $raw.Split('|', 3)
  $keys = $parts[0]
  $nonKey = $parts[1]
  $text = if ($parts.Length -ge 3) { $parts[2] } else { "" }
  $hex = (($text.ToCharArray() | ForEach-Object { '{0:X2}' -f [int]$_ }) -join ' ')
  $has200 = $text.Contains($esc + "[200~")
  $has201 = $text.Contains($esc + "[201~")
  $hasEsc = $text.Contains($esc)
  Add-Content -Path $Log -Value ("trial={0} keys={1} nonkey={2} len={3} esc={4} m200={5} m201={6} hex={7}" -f `
      $i, $keys, $nonKey, $text.Length, $hasEsc, $has200, $has201, $hex) -Encoding UTF8
}

[Console]::Write("$([char]27)[?2004l")
