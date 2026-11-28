#!/bin/sh
# 批末孤儿检查（**只报告，不杀任何进程**）：列出本次开发可能遗留的自动化进程。
#
# 背景（P9-F5）：多轮 pty 实验曾遗留约 40 个 `pi-rs --no-session`，把 load 顶到 44，
# 让无关的并发测试超时失败。这里只做可见性，杀进程由人决定（避免误伤用户会话）。
set -u
echo "== pi-rs / mock-provider / runner.mjs 进程"
ps -eo pid,etimes,args | awk 'NR==1 || /pi-rs( |$)|mock-provider|ext-host\/runner\.mjs/ {print}' | head -40
echo
echo "== 判定：命令行含 scenarios-tui/ 或 --scenario 的是自动化进程"
auto=$(ps -eo args | grep -E 'scenarios-tui/|--scenario' | grep -vE 'check-orphans|grep -E' | wc -l)
echo "自动化进程计数（不含本检查本身）：$auto"
echo "== 手工清理示例（先确认 PID 属于本机测试）：kill -TERM <pid>"
