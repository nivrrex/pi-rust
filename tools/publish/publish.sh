#!/bin/sh
# pi-rs 发布脚本（用户本人执行）。
#
# 依赖：sh、git、curl。凭据只从环境变量 GITHUB_TOKEN 读取，并通过 `curl -K -`
# 以 stdin 配置方式传入（不出现在进程参数、不写文件、不打印）。
#
# 用法：
#   cd /root/archives/publish-<日期>
#   export GITHUB_TOKEN=...
#   sh publish.sh
set -eu

REPO="${REPO:-nivrrex/pi-rust}"
TAG="${TAG:-v0.87.1}"
API="${API_BASE:-https://api.github.com}/repos/$REPO"
DRY_RUN="${DRY_RUN:-0}"
ASSETS="pi-rs-linux-amd64 pi-rs-linux-amd64-musl pi-rs-linux-arm64 pi-rs-windows-amd64.exe"
HERE=$(cd "$(dirname "$0")" && pwd)
BACKUP_BUNDLE="${BACKUP_BUNDLE:-/root/archives/pi-rust-remote-backup-20261029.bundle}"

die() { printf '✗ %s\n' "$1" >&2; exit 1; }
step() { printf '\n== %s\n' "$1"; }

step "0/6 前置检查"
[ -n "${GITHUB_TOKEN:-}" ] || die "缺少环境变量 GITHUB_TOKEN（只在环境里提供，不要写进脚本）"
[ -f "$BACKUP_BUNDLE" ] || die "找不到备份 bundle：$BACKUP_BUNDLE（先在远端状态良好时做过 git clone --mirror 才对）"
[ -f "$HERE/SHA256SUMS" ] || die "缺少 $HERE/SHA256SUMS"
[ -f "$HERE/pi-rs.git.bundle" ] || die "缺少发布提交的 bundle：$HERE/pi-rs.git.bundle"
for f in $ASSETS; do [ -f "$HERE/$f" ] || die "缺少产物：$f"; done
( cd "$HERE" && sha256sum -c SHA256SUMS >/dev/null ) || die "SHA256SUMS 与本地产物不一致"
printf '✓ 备份 bundle、SHA256SUMS、四份产物、git bundle 齐备\n'

# curl 的凭据配置走 stdin（`-K -`），避免出现在 argv / 历史里
gh() {
  method=$1; url=$2; out=$3; data=${4:-}
  if [ -n "$data" ]; then
    printf 'header = "Authorization: token %s"\n' "$GITHUB_TOKEN" \
      | curl -sS -K - -X "$method" -H 'Accept: application/vnd.github+json' \
             -H 'Content-Type: application/json' --data-binary "$data" "$url" -o "$out"
  else
    printf 'header = "Authorization: token %s"\n' "$GITHUB_TOKEN" \
      | curl -sS -K - -X "$method" -H 'Accept: application/vnd.github+json' "$url" -o "$out"
  fi
}

gh_up() { # upload <upload_url> <file>
  printf 'header = "Authorization: token %s"\n' "$GITHUB_TOKEN" \
    | curl -sS -K - -X POST -H 'Content-Type: application/octet-stream' \
           --data-binary "@$2" "$1" -o /dev/null
}

step "1/6 强推 main（发布用单提交）"
if [ "$DRY_RUN" = 1 ]; then
  printf '（DRY_RUN：跳过 git push main）\n'
else
  git -C "$HERE/repo" push --force "https://github.com/$REPO" \
    refs/heads/publish:refs/heads/master \
    || die "强推失败。回滚：git clone $BACKUP_BUNDLE 后按其中的 refs/heads/master 恢复"
  printf '✓ main 已更新\n'
fi

step "2/6 移动 tag $TAG"
if [ "$DRY_RUN" = 1 ]; then
  printf '（DRY_RUN：跳过 git push tag）\n'
else
  git -C "$HERE/repo" push --force "https://github.com/$REPO" "refs/tags/$TAG:refs/tags/$TAG" \
    || die "tag 推送失败。回滚：以备份 bundle 中的 tag 为准强推回去"
  printf '✓ tag 已指向新提交\n'
fi

step "3/6 删除 Release 旧附件"
gh GET "$API/releases/tags/$TAG" "$HERE/.release.json" || die "查询 Release 失败"
python3 - "$HERE/.release.json" > "$HERE/.asset-ids" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
for asset in data.get("assets", []):
    print(asset["id"], asset["name"])
PY
[ -s "$HERE/.asset-ids" ] || printf '（Release 目前没有附件）\n'
while read -r id name; do
  [ -n "${id:-}" ] || continue
  gh DELETE "$API/releases/assets/$id" "$HERE/.del.json" || die "删除附件 $name 失败"
  printf '  - 已删除 %s\n' "$name"
done < "$HERE/.asset-ids"

step "4/6 上传新附件"
UPLOAD=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["upload_url"].split("{")[0])' "$HERE/.release.json")
for f in $ASSETS; do
  gh_up "$UPLOAD?name=$f" "$HERE/$f" || die "上传 $f 失败（可用备份 bundle 回滚 tag 与 main）"
  printf '  + %s\n' "$f"
done
gh_up "$UPLOAD?name=SHA256SUMS" "$HERE/SHA256SUMS" || die "上传 SHA256SUMS 失败"

step "5/6 重新下载并逐个核对 sha256"
gh GET "$API/releases/tags/$TAG" "$HERE/.release2.json" || die "查询 Release 失败"
python3 - "$HERE/.release2.json" > "$HERE/.asset-urls" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
for asset in data.get("assets", []):
    print(asset["name"], asset["browser_download_url"])
PY
fail=0
while read -r name url; do
  case " $ASSETS SHA256SUMS " in *" $name "*) ;; *) continue ;; esac
  # 清单自己不列自身 ⇒ 只确认存在，不比对 sha256
  if [ "$name" = "SHA256SUMS" ]; then printf '  ✓ %s（清单本身，仅确认存在）\n' "$name"; continue; fi
  curl -sSL "$url" -o "$HERE/.check-$name" || { printf '  ✗ 下载 %s 失败\n' "$name"; fail=1; continue; }
  want=$(grep " $name\$" "$HERE/SHA256SUMS" | awk '{print $1}')
  got=$(sha256sum "$HERE/.check-$name" | awk '{print $1}')
  if [ "$want" = "$got" ]; then printf '  ✓ %s %s\n' "$name" "$got"; else printf '  ✗ %s 期望 %s 实际 %s\n' "$name" "$want" "$got"; fail=1; fi
  rm -f "$HERE/.check-$name"
done < "$HERE/.asset-urls"
[ "$fail" = 0 ] || die "附件校验失败（已上传的内容可用备份 bundle 回滚）"

step "6/6 完成"
printf '✓ 发布完成：https://github.com/%s/releases/tag/%s\n' "$REPO" "$TAG"
printf '  回滚：用 %s 里的 refs 恢复 main/tag，再重跑本脚本上传旧产物。\n' "$BACKUP_BUNDLE"
