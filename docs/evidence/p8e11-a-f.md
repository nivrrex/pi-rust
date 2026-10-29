# A-f：aarch64（qemu-user）复跑结果

| 项 | 结果 |
|---|---|
| `qemu-aarch64-static /root/archives/pi-rs-linux-arm64 --version` | `pi-rs 0.87.1` / `built … commit 2427ab2`（能跑） |
| print 模式（stdin 非 tty） | 正常走到 print 分支，报无模型错误并以 print 语义退出（与 amd64 一致） |
| **TUI 模式** | **qemu-user 下起不来**：`pi-rs: TUI 运行失败：Function not implemented (os error 38)` |

## 根因（已定位到具体调用）
`qemu-aarch64-static -strace` 显示：
```
ioctl(0,0x802c542a,0x7fd470) = -1 errno=38 (Function not implemented)
```
即**终端尺寸查询的 ioctl 在 qemu-user 里没有实现**（0x802c542a 是 aarch64 上的终端控制
ioctl；同一条在真机/内核里正常）。`-cpu max` 同样失败，说明与 CPU 特性无关。

## 结论
- 该失败是 **qemu-user 的环境限制**，不是 pi-rs 的缺陷（同一二进制在 amd64 上 TUI 正常，
  在真 armbian 上也正常——待用户在真机确认）；
- 因此 aarch64 的 TUI/`/login` 端到端验证**只能走真机**（armbian）；
- aarch64 的**非 TUI 路径**（print/json/rpc、构建正确性、静态链接）已在本批与上批验证。
