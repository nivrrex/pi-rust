#!/bin/sh
python3 - "$1" <<'PY'
import json, os, sys
out = sys.argv[1]
def collapsed(path):
    with open(path, encoding="utf-8") as h:
        d = json.load(h)
    return sum(1 for t in d.get("tools", []) if t.get("collapsed_row"))
base = os.path.join(out, "ctrlo2-pi-rs", "steps")
counts = [collapsed(os.path.join(base, f"step-{i}.json")) for i in (0, 1, 2)]
print(f"  pi-rs 折叠行数: 初态 {counts[0]} → 展开 {counts[1]} → 回折 {counts[2]}")
bad = 0
if counts[1] != 0:
    print("  ✗ 展开态仍有折叠行"); bad += 1
else:
    print("  ✓ 展开态折叠行清零")
if counts[2] != counts[0]:
    print("  ✗ 回折后未回到初态"); bad += 1
else:
    print("  ✓ 回折后折叠行数回到初态")
sys.exit(bad)
PY
