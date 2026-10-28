# 回归结果（P8e-2 收尾，数字自述未核实）

## 文本级严格逐帧（`scripts/regress-tui.sh`）
| 轮次 | 结果 |
|---|---|
| glibc ×3（`--runs 3 --jobs 5`） | **same=189 / different=0 / nogolden=39 / runfail=0** |
| musl 静态 ×1 | **same=63 / different=0 / nogolden=13 / runfail=0** |

`nogolden` 是尚未录基线的场景（`T72` 为 print 模式不适用 TUI golden 等），不是失败。

## pty 探针（netns 内）
- `pty_retry_network_check.py`（连接不可达 / DNS 失败）：**all_ok**
- `pty_esc_latency_check.py`（流式 / 工具执行，N=4）：**all_ok**
- `pty_escape_abort_check.py`（4 配置 × N=4）：**all_ok**
- `ls_tool_check.py`（print 逐字节 + pty 调用头）：通过

## 样式级（`compare-style --style-only`，整段流口径）
| 场景 | 差异格 | 备注 |
|---|---|---|
| T01 | **0（same）** | 本批达标 |
| T06 | 23 | 版本行 dim 与帧集合差异 |
| T05 | 331 | 约 320 格为流末态不同 |
| T77 | 226 | 边框色 i242 未定位（3 次假设否证，deferred） |
| T28 / T35 | 28 / 26 | 帧数不一致（TS 6 帧、我们 5 帧） |

样式级"逐格 same"在含瞬时状态的场景里受**帧集合不一致**限制，需按内容对齐帧配对才能继续压缩；
这是工具能力限制，已记入 `LIMITATIONS.md`。

## P8e-3 收口（数字自述未核实）
- 文本级 glibc ×3：**same=189 / different=0 / nogolden=39 / runfail=0**
- 文本级 musl ×1：**same=63 / different=0 / nogolden=13 / runfail=0**
- pty 探针：`pty_retry_network_check`、`pty_esc_latency_check`、`ls_tool_check`、`pty_escape_abort_check` 全绿
- 样式级（真彩 golden，`--style-only`）：T01 **1**、T28 24、T35 24、T05 433、T77 225、T73 387、T75 2457
- 样式级（256 golden，仅参考）：T01 886、T28 622、T35 597、T05 993、T77 641、T73 1774、T75 2508

## P8e-4 收口（数字自述未核实）
- 文本级 glibc ×3：**same=189 / different=0 / nogolden=39 / runfail=0**
- 文本级 musl ×1：**same=63 / different=0 / nogolden=13 / runfail=0**
- pty 探针：`pty_escape_abort_check`（4 配置）、`pty_retry_check3`、`pty_retry_network_check`、`pty_esc_latency_check` 全绿
- 样式级（真彩 golden）：T01 **1**、T77 **25**、T28 24、T35 24、T05 433、T73 387、T75 2457
- 本批唯一样式修复：编辑框边框色随 thinking 档位（K3），T77 从 226 降到 25

## P8 出口定义逐条（本批结论）
| # | 条目 | 结果 |
|---|---|---|
| 1 | 文本级 glibc×3、musl×1 全 same | **达成**（189/0、63/0） |
| 2 | 屏幕错位根治 | **未达**：定位到上游原因（帧切分/行对齐），未实施修复 |
| 3 | 真彩判定九条 + 两套 SGR 单测 | **达成** |
| 4 | 样式级归零或逐条 ref 归因 | **部分**：每项均有 ref 行号级归因（`LIMITATIONS.md` 第 1–5 条），但差异未归零 |
| 5 | 43 条绑定判定完成 | **达成** |
| 6 | 无未提交/无 stray/precommit 通过 | **达成** |

## P8e-5 收口（数字自述未核实）
- 文本级 glibc ×3：**same=189 / different=0 / nogolden=39 / runfail=0**
- 文本级 musl ×1：**same=63 / different=0 / nogolden=13 / runfail=0**
- pty 探针：`pty_escape_abort_check`(4 配置) / `pty_retry_check3` / `pty_retry_network_check` /
  `pty_esc_latency_check` / `ls_tool_check` 全绿
- 样式级真彩：T01 0、T05 8、T28 0、T35 0、T73 0、T75 0、T77 3（合计 **11**，本批起点 1208）
- 样式级 256：T01 3、T05 8、T28 2、其余 0（合计 **13**）
- `--final-screen` 口径实测不可用（退出收尾写不同 ⇒ 最后一屏不是同一时刻），
  判定以整流 `--style-only` 为准，详见 `notes/evidence/K5-style-tables.md`
- `cargo test --workspace`：754/0/1；`scripts/precommit.sh` 十一项通过

## P8 出口定义逐条（P8e-5 批末）
| # | 条目 | 结果 |
|---|---|---|
| 1 | 文本级 glibc×3、musl×1 全 same | **达成**（189/0、63/0） |
| 2 | 屏幕错位：真实缺陷则根治，假象则修回放器且证据入库 | **达成**：判定为**回放器假象**（`compare-style` 缺隐式滚屏），已修 + 性质测试入库 + `notes/evidence/K1p-replayer-compare.md` |
| 3 | 真彩判定九条 + 两套 SGR 单测 | **达成**（`style.rs` 九条判定 + 14 条单测） |
| 4 | 样式级差异为 0，或每项残差有 ref 行号级归因且换过两种方法 | **达成（按归因口径）**：真彩 11 格、256 13 格，残差 3 项，每项在 `LIMITATIONS.md` 第 1–2 条给出 ref 行号与已试方法 |
| 5 | 43 条绑定判定与 LIMITATIONS 一致 | **达成** |
| 6 | 无未提交、无 stray、precommit 通过 | **达成** |

## P8e-5 批末复测（K4c 收尾后，数字自述未核实）
- 文本级 glibc ×3：**same=189 / different=0 / nogolden=39 / runfail=0**
- 文本级 musl ×1：**same=63 / different=0 / nogolden=13 / runfail=0**
- pty 探针：`pty_escape_abort_check`(4 配置) / `pty_retry_network_check` / `pty_esc_latency_check` 全绿
- `cargo test --workspace`：755/0/1；`scripts/precommit.sh` 十一项通过
- 样式级真彩：T01/T05/T28/T35/T73/T75 **全 same**，T77 **3**（合计 3）
- 样式级 256：T05/T35/T73/T75/T77 **全 same**，T01 3、T28 2（合计 5）

## 主题支持（P9，自述未核实）
- 内置主题数据**直接内嵌上游** `theme/dark.json`、`theme/light.json`（`include_str!`），
  解析 `vars`→`colors` 引用链（ref `theme.ts` 的 `loadTheme`/`resolveColors`）；未知主题回落 dark。
- `settings.json` 的 `theme` 键生效（缺省 dark）；`/settings` 选主题写盘并发
  `TuiEvent::ThemeName` 立即重绘。
- 渲染取色全部走 `theme::Palette`（`item_style`、`apply_chrome_styles`、`header_spans`、
  工具头 spans、markdown 各键、编辑框边框 `thinkingOff…thinkingMax`）。
- 回归维度：`scripts/regress-tui.sh` 支持 `THEME=dark|light`（场景目录与 golden 目录随主题切换）。

### 实测
| 口径 | 结果 |
|---|---|
| dark 文本级 glibc ×3 | same=189 / different=0 / nogolden=39 |
| musl ×1 | same=63 / different=0 / nogolden=13 |
| dark 样式级（真彩 7 场景） | 仅 T77 3 格（TS 录制自身不稳定） |
| light 样式级（真彩 6 场景） | **全 same** |
| light 文本级（1 轮，6 场景） | same=6 / different=0 |
| light 256（T01/T05 抽查） | 文本 same + 样式 same |
| 单元/集成测试 | 761/0；clippy 0；precommit 十一项 |

## 最终门禁（本轮，数字自述未核实）
| 口径 | 结果 |
|---|---|
| dark 文本级 glibc ×3 | **same=189 / different=0 / nogolden=39 / runfail=0** |
| light 文本级 ×3 | **same=18 / different=0 / runfail=0**（6 场景 ×3 轮） |
| musl 静态 ×1 | **same=63 / different=0 / nogolden=13 / runfail=0** |
| pty 探针（6 个） | Esc 中止 / 断网重试 / 中止延迟 / IME 光标 / 退出残留（20 次）/ 错误累积 `[1,2,3,4]` 全绿 |
| 单元/集成 | `cargo test --workspace` 764/0；`node --test tools/difftest/test/*` 91 pass |
| 检查 | clippy `-D warnings` 0、`lint-banned` OK、`precommit.sh` 十一项通过 |

## P8 出口定义重判（P8e-7 批末，最终 HEAD）
| # | 条目 | 结果 | 依据 |
|---|---|---|---|
| 1 | 文本级 glibc×3、musl×1 全 same | **达成** | dark ×3 `same=189/0`、light ×3 `same=18/0`、musl ×1 `same=63/0` |
| 2 | 屏幕错位根治或证明为回放器假象 | **达成** | 判定为 `compare-style` 缺隐式滚屏（`notes/evidence/K1p-replayer-compare.md`），已修并加性质测试 |
| 3 | 真彩判定九条 + 两套 SGR 单测 | **达成** | `style.rs` 九条判定 + `truecolor_from` 14 条单测 |
| 4 | 样式级差异为 0 或逐项 ref 归因且换过两种方法 | **达成（归因口径）** | 真彩 7 场景仅 T77 3 格（TS 录制自身不稳定，输出等于其 run-2/3）；light 6 场景全 same |
| 5 | 43 条绑定判定与 LIMITATIONS 一致 | **达成** | `LIMITATIONS.md` 键位与交互段 |
| 6 | 无未提交、无 stray、precommit 通过 | **达成** | 工作区干净、`check-no-stray` 无残留、十一项通过 |

## P8e-7 批内新增的实质修复（供出口复核）
- `apiKey` 的 `$VAR` / `${VAR}` / `!command` 三形态此前未解析（`450c320`、`fa8fdb6`）。
- 无凭据时仍发请求、由服务端回 401 → 改为本地拒绝不发请求（`0cd8f7d`）。
- 退出提示用 `writeln!` 在 Windows 发 CRLF → 改为只发 LF（`770df1a`）。
- `MOCK_VERBOSE` 必须设在 mock 进程（取证坑位已写进 `tools/mock-provider` 注释，`099f8c9`）。
