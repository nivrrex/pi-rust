// `@earendil-works/pi-tui` 的最小面（P9-1-B）。
//
// 三条原则：
// 1. **纯函数给真实实现**（`truncateToWidth`/`visibleWidth`/`wrapTextWithAnsi`/`matchesKey`）——
//    扩展在工厂期就会调用它们，返回值错了会静默出错；
// 2. **组件给"可构造」骨架**（`Text`/`Container`/`Spacer`/`SelectList`）——
//    本批不实现渲染钩子，但扩展里 `new Text(...)` 不能崩；
// 3. **其余符号显式抛"未实现"**，而不是返回 undefined（P8e-11 的教训：静默降级最难查）。

/** 把任意组件序列化成降级帧；未支持的组件显式给出 `unsupported`。 */
export function frameOf(component) {
  if (component && typeof component.toFrame === "function") return component.toFrame();
  return {
    kind: "unsupported",
    reason: component?.constructor?.name ?? typeof component,
  };
}

const ANSI = /\u001b\[[0-9;?]*[ -/]*[@-~]|\u001b\][^\u0007]*\u0007/g;

export function visibleWidth(text) {
  const plain = String(text).replace(ANSI, "");
  let width = 0;
  for (const ch of plain) {
    const code = ch.codePointAt(0);
    // 东亚宽字符区间（够用的近似：CJK/全角/emoji 记 2 列）。
    const wide =
      (code >= 0x1100 && code <= 0x115f) ||
      (code >= 0x2e80 && code <= 0xa4cf) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6) ||
      (code >= 0x1f300 && code <= 0x1f64f) ||
      (code >= 0x1f900 && code <= 0x1f9ff);
    width += wide ? 2 : 1;
  }
  return width;
}

export function truncateToWidth(text, width, ellipsis = "…") {
  const plain = String(text);
  if (visibleWidth(plain) <= width) return plain;
  const limit = Math.max(0, width - visibleWidth(ellipsis));
  let out = "";
  let used = 0;
  for (const ch of plain) {
    const cost = visibleWidth(ch);
    if (used + cost > limit) break;
    out += ch;
    used += cost;
  }
  return out + ellipsis;
}

export function wrapTextWithAnsi(text, width) {
  const source = String(text);
  if (width <= 0) return [source];
  const lines = [];
  for (const paragraph of source.split("\n")) {
    let current = "";
    let used = 0;
    for (const ch of paragraph) {
      const cost = visibleWidth(ch);
      if (used + cost > width && current) {
        lines.push(current);
        current = "";
        used = 0;
      }
      current += ch;
      used += cost;
    }
    lines.push(current);
  }
  return lines;
}

export function matchesKey(data, keyId) {
  const table = {
    enter: ["\r", "\n"],
    escape: ["\u001b"],
    tab: ["\t"],
    up: ["\u001b[A"],
    down: ["\u001b[B"],
    left: ["\u001b[D"],
    right: ["\u001b[C"],
    backspace: ["\u007f", "\b"],
    space: [" "],
    // ref `packages/tui/src/keybindings.ts:154-159` 的选择器默认键位别名。
    "tui.select.confirm": ["\r", "\n"],
    "tui.select.cancel": ["\u001b", "\u0003"],
    "tui.select.up": ["\u001b[A"],
    "tui.select.down": ["\u001b[B"],
    "tui.select.pageUp": ["\u001b[5~"],
    "tui.select.pageDown": ["\u001b[6~"],
    // 单行输入框（pi-secrets 的掩码输入）默认键位。
    "tui.input.submit": ["\r", "\n"],
    "tui.input.cancel": ["\u001b", "\u0003"],
    "tui.input.deleteCharBackward": ["\u007f", "\b"],
  };
  let keys = table[keyId];
  if (!keys) {
    // `ctrl+<字母>` ⇒ 控制字符（`ctrl+u` = 0x15）。
    const ctrl = /^ctrl\+([a-z])$/.exec(String(keyId));
    if (ctrl) keys = [String.fromCharCode(ctrl[1].charCodeAt(0) - 96)];
  }
  if (!keys) throw new Error(`pi-ext-host shim: matchesKey("${keyId}") 未实现`);
  return keys.includes(String(data));
}

class Component {
  render() {
    throw new Error("pi-ext-host shim: 渲染钩子未实现（P9 后续批次）");
  }
  invalidate() {}
}

export class Text extends Component {
  // ref `components/text.ts:18`：`Text(text, paddingX = 1, paddingY = 1)`。
  constructor(text = "", paddingX = 1, paddingY = 1) {
    super();
    this.text = String(text);
    this.paddingX = paddingX;
    this.paddingY = paddingY;
  }
  render(width = 80) {
    // ref `components/text.ts:48-100`：三处都要照做，否则逐帧对拍会差。
    // ① 空文本（或纯空白）⇒ **空数组**，不占行；
    // ② 每行加**左右两侧** padding 并补齐到整宽（右侧不是空的）；
    // ③ `paddingY` 的空行是 **width 个空格**，不是空串。
    if (!this.text || this.text.trim() === "") return [];
    const normalized = this.text.replace(/\t/g, "   ");
    // ref `:64-65`：paddingX 按可用宽度夹住，避免内容区变负。
    const paddingX = Math.min(this.paddingX, Math.max(0, Math.floor((width - 1) / 2)));
    const contentWidth = Math.max(1, width - paddingX * 2);
    const left = " ".repeat(paddingX);
    const body = wrapTextWithAnsi(normalized, contentWidth).map((line) => {
      const withMargins = `${left}${line}${left}`;
      const pad = Math.max(0, width - visibleWidth(withMargins));
      return withMargins + " ".repeat(pad);
    });
    const empty = " ".repeat(width);
    const blank = Array.from({ length: this.paddingY }, () => empty);
    return [...blank, ...body, ...blank];
  }
  toFrame() {
    return {
      kind: "text",
      text: this.text,
      paddingX: this.paddingX,
      paddingY: this.paddingY,
    };
  }
}

export class Container extends Component {
  toFrame() {
    return { kind: "stack", children: this.children.map((child) => frameOf(child)) };
  }

  constructor() {
    super();
    this.children = [];
  }
  addChild(child) {
    this.children.push(child);
    return child;
  }
  removeChild(child) {
    this.children = this.children.filter((item) => item !== child);
  }
  clear() {
    this.children = [];
  }
  render(width = 80) {
    return this.children.flatMap((child) => child.render(width));
  }
}

export class Spacer extends Component {
  constructor(count = 1) {
    super();
    this.count = count;
  }
  render() {
    return Array.from({ length: this.count }, () => "");
  }
  toFrame() {
    return { kind: "spacer", lines: this.count };
  }
}

export class SelectList extends Component {
  constructor(items = [], maxVisible = 10, options = {}) {
    super();
    this.items = items.map((item) =>
      typeof item === "string" ? { label: item, value: item } : item,
    );
    this.maxVisible = maxVisible;
    this.options = options;
    this.selectedIndex = 0;
  }
  invalidate() {}
  getSelectedItem() {
    return this.items[this.selectedIndex];
  }
  setSelectedIndex(index) {
    this.selectedIndex = Math.max(0, Math.min(this.items.length - 1, index));
  }
  setItems(items) {
    this.items = items.map((item) =>
      typeof item === "string" ? { label: item, value: item } : item,
    );
    this.selectedIndex = Math.min(this.selectedIndex, Math.max(0, this.items.length - 1));
  }
  // ref `select-list.ts`：up/down 移动、enter 确认（由调用方的 done 处理）。
  handleInput(data) {
    if (matchesKey(data, "down")) this.setSelectedIndex(this.selectedIndex + 1);
    else if (matchesKey(data, "up")) this.setSelectedIndex(this.selectedIndex - 1);
  }
  render(width = 80) {
    // ref `select-list.ts:75-109`：空列表先走 `theme.noMatch`。
    // 此外列表过长时要给 `theme.scrollInfo` 的 `(n/total)` 行，
    // 每一项的选中态用 `selectedText`/`selectedPrefix`、描述列用 `theme.description`。
    // 此前这里**完全忽略** `options`（构造时传入的样式函数）⇒ `pi-interact`
    // 多选面板传入的五个样式函数全部失效：没有高亮、没有描述、没有滚动提示。
    const noMatch = this.options.noMatch;
    if (this.items.length === 0) {
      return [truncateToWidth(typeof noMatch === "function" ? noMatch("  No matching items") : "  No matching items", width)];
    }
    const maxVisible = Math.max(1, this.maxVisible);
    const start = Math.max(0, Math.min(this.selectedIndex, this.items.length - maxVisible));
    const end = Math.min(this.items.length, start + maxVisible);
    const lines = [];
    for (let index = start; index < end; index += 1) {
      const item = this.items[index];
      const isSelected = index === this.selectedIndex;
      const label = typeof item === "string" ? item : String(item.label ?? item.value ?? "");
      const description =
        typeof item === "object" && item && typeof item.description === "string"
          ? item.description.replace(/\s+/g, " ").trim()
          : "";
      // 描述列只在够宽时出现（ref `:193` 的 `width > 40`）。
      if (description && width > 40) {
        const desc = this.options.description;
        const painted = typeof desc === "function" ? desc(description) : description;
        const prefix = isSelected ? "→ " : "  ";
        const line = `${prefix}${label}  ${painted}`;
        lines.push(truncateToWidth(isSelected && typeof this.options.selectedText === "function" ? this.options.selectedText(line) : line, width));
      } else {
        const prefix = isSelected ? "→ " : "  ";
        const line = `${prefix}${label}`;
        lines.push(truncateToWidth(isSelected && typeof this.options.selectedText === "function" ? this.options.selectedText(line) : line, width));
      }
    }
    if (start > 0 || end < this.items.length) {
      const scroll = `  (${this.selectedIndex + 1}/${this.items.length})`;
      const paint = this.options.scrollInfo;
      lines.push(truncateToWidth(typeof paint === "function" ? paint(scroll) : scroll, width));
    }
    return lines;
  }
}

export const DynamicBorder = class extends Component {};
export const TUI = undefined;
export const Focusable = undefined;
// ref `packages/tui/src/keys.ts`：`Key` 既是常量表，也提供 `Key.ctrl/alt/shift(...)` 组合器。
const KEY_CONSTANTS = {
  enter: "enter",
  escape: "escape",
  tab: "tab",
  up: "up",
  down: "down",
  left: "left",
  right: "right",
  backspace: "backspace",
  space: "space",
};

export const Key = Object.assign(
  (...parts) => parts.join("+"),
  KEY_CONSTANTS,
  {
    ctrl: (key) => `ctrl+${key}`,
    alt: (key) => `alt+${key}`,
    shift: (key) => `shift+${key}`,
  },
);
