// `sessionManager.getLeafId()` / `getBranch()` 夹具：
// 报出 leafId 与 branch 的 id 序列。
export default function branchLeafProbe(pi) {
  pi.on("session_before_tree", (_e, ctx) => {
    const sm = ctx?.sessionManager;
    return {
      summary: JSON.stringify({
        leafId: sm?.getLeafId?.() ?? null,
        branch: (sm?.getBranch?.() ?? []).map((entry) => entry?.id),
      }),
    };
  });
}
