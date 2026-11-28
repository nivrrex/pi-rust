// `session_compact` / `session_compact_failed` 夹具：把收到的载荷原样写回一个
// 全局，宿主侧通过下一条事件的返回值把摘要带回给测试。
//
// 为什么用"返回值"而不是文件：宿主进程只继承白名单环境（`ext_tools.rs`），
// 也不能保证可写路径；而事件派发的返回值本来就会回传（runner 通用分支）。
export default function sessionCompactProbe(pi) {
  let compact = null;
  let failed = null;
  pi.on("session_compact", (event) => {
    compact = event;
    return undefined;
  });
  pi.on("session_compact_failed", (event) => {
    failed = event;
    return undefined;
  });
  // 测试在压缩之后调一次任意可返回值的事件，把两个载荷带回去。
  pi.on("session_before_tree", () => ({ summary: JSON.stringify({ compact, failed }) }));
}
