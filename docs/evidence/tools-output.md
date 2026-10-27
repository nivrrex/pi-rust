# 工具结果正文

## 规则（按工具区分）
- `read`：**折叠态不显示正文**，展开或出错才显示；
- `bash` / `powershell`：折叠显示**尾部 5 行**，多出的提示 `... (N earlier lines, ctrl+o to expand)`；
- `ls`：折叠显示头部若干行；
- 其余：fallback 头部 10 行。

`ctrl+o` 全局切换展开/折叠，并显示状态行 `Tool output: expanded|collapsed`。

## 复现
- `scenarios-tui/T75-bash-60.json`：模型调 `bash "seq 1 60"`，验证折叠预览与展开。
- `tools/difftest/ls_tool_check.py`：`ls` 的 print 口径与上游逐字节一致。
