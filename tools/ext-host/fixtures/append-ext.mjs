// `pi.appendEntry` 夹具（P9-1-E3b / S4）：往会话里写一条 custom 条目。
export default function appendExtension(pi) {
  pi.on("session_start", () => {
    pi.appendEntry("note", { from: "extension" });
  });
}
