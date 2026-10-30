# G' 发布包（P8e-13）

## 包内容
`/root/archives/publish-<日期>/`（由 `tools/publish/build-package.sh` 生成，可重复执行）：

| 文件 | 说明 |
|---|---|
| `pi-rs-linux-amd64` / `-musl` / `pi-rs-linux-arm64` / `pi-rs-windows-amd64.exe` | 四份产物（与 `/root/archives` 同一批构建） |
| `SHA256SUMS` | 上述四份的校验和（`sha256sum -c` 通过） |
| `pi-rs.git.bundle` | 发布用**单一提交**（`b501041`，tree 与 HEAD 相同） |
| `repo/` | 含该提交的本地仓库（分支 `publish`），供 `publish.sh` 强推 |
| `publish.sh` | 发布脚本（见下） |
| `发布说明.md` | 版本、产物、已知限制、三行操作步骤 |

## publish.sh 的约束落实
| 要求 | 实现 |
|---|---|
| 只依赖 sh/git/curl | 是（`sha256sum` 用于自检，属 coreutils） |
| 令牌不进进程参数 | `curl -K -` 从 stdin 读配置；`ps` 里看不到令牌 |
| 令牌只读 `GITHUB_TOKEN` | 缺失即报错退出 |
| 开头校验 | 备份 bundle 存在、`SHA256SUMS` 与本地产物一致、四份产物齐全、git bundle 存在 |
| 步骤 | 强推 main → 移动 tag → 删旧附件 → 上传（含 `SHA256SUMS`）→ 重新下载逐个核对 → 打印结果 |
| 失败即停 + 回滚提示 | 每个 `die` 都指向备份 bundle |
| 不打印令牌 | 全部输出无令牌；`curl` 静默 |

## 静态检查与干跑（无网络、假令牌）
- `sh -n` 通过；`shellcheck -s sh` 0 告警；
- 用本地假 GitHub API（`python3 http.server`）干跑两种情形：
  1. Release 无附件 → 上传 5 个文件 → 下载回验 5/5 ✓；
  2. Release 已有附件 → 先删除（日志可见 `- 已删除 …`）→ 再上传 → 回验 5/5 ✓。
- 干跑用 `DRY_RUN=1` 跳过两个 `git push` 步骤（本地无远端写权限），其余步骤真实走 HTTP。

## 用户操作步骤（三行）
```sh
cd /root/archives/publish-20261030
export GITHUB_TOKEN=<令牌>
sh publish.sh
```
验证：脚本末尾会重新下载每个附件并逐个比对 sha256，全部 `✓` 即成功；
打印 `✓ 发布完成：https://github.com/nivrrex/pi-rust/releases/tag/v0.87.1` 为终态。
