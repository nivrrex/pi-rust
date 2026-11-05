// `ctx.ui.custom` 夹具（E4e）：最小组件（render + handleInput + done）。
import fs from "node:fs";

const LOG = process.env.PI_EXT_TEST_LOG;

function record(line) {
  if (!LOG) return;
  fs.appendFileSync(LOG, `${line}\n`);
}

class Picker {
  constructor(done) {
    this.done = done;
    this.selected = 0;
    this.items = ["alpha", "beta"];
  }
  render() {
    return this.items.map((item, index) => `${index === this.selected ? "→ " : "  "}${item}`);
  }
  handleInput(data) {
    if (data === "\u001b[B") this.selected = Math.min(1, this.selected + 1);
    if (data === "\u001b[A") this.selected = Math.max(0, this.selected - 1);
    if (data === "\r") this.done(this.items[this.selected]);
    if (data === "\u001b") this.done(undefined);
  }
  invalidate() {}
}

export default function customExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    void ctx.ui.custom((_tui, _theme, _kb, done) => new Picker(done)).then((value) => {
      record(`custom:result:${String(value)}`);
    });
  });
}
