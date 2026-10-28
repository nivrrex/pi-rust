# probe-vt-input.ps1 —— 判断 Windows 控制台在两种输入模式下各给出什么字节。
#
# 用途（P8e-9/C-d）：裁决"是否值得在 Windows 上改走 VT 输入字节路径"。
# 纯 PowerShell + Add-Type P/Invoke（kernel32），**不读任何配置或凭据**。
#
# 用法（在 conhost 或 Windows Terminal 里跑两次）：
#   powershell -ExecutionPolicy Bypass -File probe-vt-input.ps1 -Mode default
#   powershell -ExecutionPolicy Bypass -File probe-vt-input.ps1 -Mode vt
#
# 按提示依次操作：按 a → Ctrl+D → Ctrl+P → Esc → 上箭头 → 粘贴三行文本 → 回车结束。

param([ValidateSet('default','vt')][string]$Mode = 'default')

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class K32 {
  [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr GetStdHandle(int n);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetConsoleMode(IntPtr h, out uint m);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool SetConsoleMode(IntPtr h, uint m);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadConsole(IntPtr h, System.Text.StringBuilder buf, uint len, out uint read, IntPtr reserved);
}
"@

$STD_INPUT  = -10
$STD_OUTPUT = -11
$ENABLE_VIRTUAL_TERMINAL_INPUT  = 0x0200
$ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x0004
$ENABLE_PROCESSED_INPUT = 0x0001

$hIn  = [K32]::GetStdHandle($STD_INPUT)
$hOut = [K32]::GetStdHandle($STD_OUTPUT)
$modeIn = 0; $modeOut = 0
[void][K32]::GetConsoleMode($hIn, [ref]$modeIn)
[void][K32]::GetConsoleMode($hOut, [ref]$modeOut)
Write-Host ("console in=0x{0:X} out=0x{1:X}  (VT_PROCESSING={2})" -f `
  $modeIn, $modeOut, [bool]($modeOut -band $ENABLE_VIRTUAL_TERMINAL_PROCESSING))

# 输出侧也打开 VT，让 "2004h" 之类的序列能被解释（仅探测用）
[void][K32]::SetConsoleMode($hOut, $modeOut -bor $ENABLE_VIRTUAL_TERMINAL_PROCESSING)

if ($Mode -eq 'vt') {
  $newIn = $modeIn -bor $ENABLE_VIRTUAL_TERMINAL_INPUT
} else {
  $newIn = $modeIn -band (-bnot $ENABLE_VIRTUAL_TERMINAL_INPUT)
}
if (-not [K32]::SetConsoleMode($hIn, $newIn)) {
  Write-Host "SetConsoleMode 失败（可能不支持 VT 输入）" -ForegroundColor Red
}
$modeIn2 = 0; [void][K32]::GetConsoleMode($hIn, [ref]$modeIn2)
Write-Host ("输入模式已设为 0x{0:X}（VT_INPUT={1}）" -f $modeIn2, [bool]($modeIn2 -band $ENABLE_VIRTUAL_TERMINAL_INPUT))

# 打开括号粘贴，检测是否出现 ESC[200~
[Console]::Write("$([char]27)[?2004h")

Write-Host ""
Write-Host "请依次操作：按 a → Ctrl+D → Ctrl+P → Esc → 上箭头 → 粘贴三行文本 → 回车结束" -ForegroundColor Cyan
Write-Host "（每次操作后按回车让它读一次；最后输入 :q 结束）" -ForegroundColor DarkGray

$sb = New-Object System.Text.StringBuilder 4096
$report = New-Object System.Collections.Generic.List[string]
while ($true) {
  $read = 0
  [void][K32]::ReadConsole($hIn, $sb, 4000, [ref]$read, [IntPtr]::Zero)
  if ($read -eq 0) { break }
  $chunk = $sb.ToString(0, $read)
  $sb.Clear() | Out-Null
  $hex = ($chunk.ToCharArray() | ForEach-Object { '{0:X2}' -f [int]$_ }) -join ' '
  $report.Add(("len={0} hex={1}" -f $read, $hex))
  if ($chunk -match ':q') { break }
}

[Console]::Write("$([char]27)[?2004l")
Write-Host ""
Write-Host "==== 结果（模式=$Mode）====" -ForegroundColor Yellow
$joined = $report -join "`n"
$report | ForEach-Object { Write-Host $_ }
Write-Host ""
Write-Host ("出现 ESC[200~ 括号粘贴标记: {0}" -f [bool]($joined -match '1B ?5B ?32 ?30 ?30 ?7E'))
Write-Host ("出现 ESC 前缀序列:          {0}" -f [bool]($joined -match '1B'))
Write-Host ("Ctrl+P 字节 0x10:           {0}" -f [bool]($joined -match '\b10\b'))
Write-Host ("Ctrl+D 字节 0x04:           {0}" -f [bool]($joined -match '\b04\b'))
Write-Host ""
Write-Host "把两种模式（default / vt）的输出都贴回给 agent 即可。" -ForegroundColor Cyan
