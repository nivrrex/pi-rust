# Q2：整屏 diff（TS vs pi-rs，同状态同尺寸）

## 方法
`tmux` 指定 `-x cols -y rows`，两侧同样操作后用 `tmux capture-pane -p` 取整屏（含空行），再 `diff`。

## 修复前（100×30，fresh）
```
TS: 14,15 docs 路径 / 16 空 / 17 上边框 / 18 输入区 / 19 下边框 / 20 cwd / 21 footer
rs: 14 空 / 16 providers / 18 空 / 19 models / 21 上边框 / 23 下边框 / 24 cwd / 25 footer
```
差异 = **多 4 行**：我们把一条多行提示拆成三条 `Status`，每条自带一个空行。

## 根因（对照 ref 布局代码）
- 提示文案本身是一条多行消息：`core/auth-guidance.ts:16-20` 的 `formatNoModelsAvailableMessage()`
  返回 `No models available. … See:\n  providers.md\n  models.md` **一个字符串**；
- TS 把它作为一个块显示，后面只跟一个空行（`interactive-mode.ts` 的 `showStatus` 语义：
  一条消息 ⇒ `Spacer(1)` 一次）；
- 我们的 `TranscriptItem::Status` 渲染是"一行 + 一个空行"，所以拆成三条就多了 3 个空行，
  编辑器因此整体下移 4 行（含块间距）。

## 修复
`push_no_models_notice` 改为**单条 `TranscriptItem::Notice`**（Notice 的渲染按 `\n` 分行并
自动折行，末尾只加一个空行），文本保持 ref 原文（含 `See:` 与两行 docs 路径）。

## 修复后实测
| 尺寸/状态 | 修复前 diff 行数 | 修复后 |
|---|---|---|
| 100×30 fresh / paste3 / paste12 | 12 / 16 / 14 | **8 / 8 / 8** |
| 80×24 fresh / paste3 / paste12 | 16 / 16 / 16 | 18（TS 因内容更高发生上滚，见下） |

**光标行已完全一致**：12 行粘贴后两侧都是 `20,17`（修复前我们 `20,21`）。

## 剩余差异（非布局）
1. docs 路径文本：TS 指向自身安装包的 `docs/`，我们指向 `<agentDir>/docs`（路径长度不同
   还会导致 80 列下 TS 折行、我们不折）；
2. cwd 行：TS 显示 `~/Desktop/pi-rust (master)`，我们显示绝对路径；
3. footer：TS `0.0%/0 (auto) … unknown`，我们 `0.0%/128k (auto) … no-model`。
   前两项与"没有可用模型"这一状态下 TS 的窗口/模型信息缺失有关，属 B 段文案范畴。
