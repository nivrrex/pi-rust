# 版本核对

- 用户实测二进制：`commit 9bf3ee2`（built 2026-11-09T09:16:44Z）。
- `git log --oneline -15`：`9bf3ee2` **在 recover 分支历史中**，是 F51 的中间提交
  （"render an Error notice for every failed attempt … "，重试错误行那一步），
  位于 F50 HEAD `20cda840` 之后、F51 收口 `c2a1fc6` 之前。
- `git branch --contains 9bf3ee2` → `* recover`（只有本分支）。
- 结论：**不是外来分支/他人构建**，就是本批中途的构建；与 F50 HEAD 不符是正常的
  （当时 F51 已提交到第 9 个提交）。
- 后续一切测量以 `c2a1fc69`（F51 收口）重建的二进制为准。
