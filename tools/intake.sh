#!/usr/bin/env bash
# intake.sh — 现场收图流水线
#
# iPhone 拍照 → 存进 60_影像/inbox/ → 跑这个脚本 → 我就能读图诊断
#
# 用法:
#   bash tools/intake.sh              处理 inbox 里所有新图（默认阶段 P1）
#   bash tools/intake.sh -p P3        指定维修阶段
#   bash tools/intake.sh -n           预演，只看会做什么，不动文件
#   bash tools/intake.sh -t           跳过本地模型打标签（更快）
#
# 它做四件事:
#   1. 读 EXIF 真实拍摄时间（不是文件时间）
#   2. HEIC → JPG（iPhone 默认格式，我读不了 HEIC）
#   3. 原图归档 + 生成诊断用压缩版（长边 2048）
#   4. 调本地视觉模型批量打标签
#
# 安全: 原图先复制、校验字节数一致、才删 inbox 里的原件。
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"
VEHICLE_ARG=""
# -v 在正式解析前先摘出来（各脚本自己的 getopts 不认识它）
_args=(); while [ $# -gt 0 ]; do
  case "$1" in
    -v) VEHICLE_ARG="$2"; shift 2 ;;
    *) _args+=("$1"); shift ;;
  esac
done
set -- "${_args[@]+"${_args[@]}"}"
ROOT="$(resolve_vehicle "$VEHICLE_ARG")" || exit 1
INBOX="$ROOT/60_影像/inbox"
ORIG="$ROOT/60_影像/原图"
DIAG="$ROOT/60_影像/诊断"
PHASE="P1"; DO_TAG=1; DRY=0; MAXDIM=2048

while getopts "p:ntmh" opt; do
  case $opt in
    p) PHASE="$OPTARG" ;;
    n) DRY=1 ;;
    t) DO_TAG=0 ;;
    m) MAXDIM="$OPTARG" ;;
    h) sed -n '2,20p' "$0"; exit 0 ;;
    *) exit 1 ;;
  esac
done

mkdir -p "$INBOX" "$ORIG" "$DIAG"

# 收集待处理文件
shopt -s nullglob nocaseglob
FILES=("$INBOX"/*.heic "$INBOX"/*.jpg "$INBOX"/*.jpeg "$INBOX"/*.png)
shopt -u nocaseglob

if [ ${#FILES[@]} -eq 0 ]; then
  echo "📭 inbox 是空的：$INBOX"
  echo "   把 iPhone 拍的照片存到这个目录（「文件」App 里能直接看到它）"
  exit 0
fi

echo "📥 待处理 ${#FILES[@]} 个文件  |  阶段 $PHASE  |  诊断版长边 ${MAXDIM}px"
[ "$DRY" -eq 1 ] && echo "   （预演模式，不会改动任何文件）"
echo

# EXIF 拍摄时间 → YYYYMMDD-HHMMSS；取不到就退回文件修改时间
shot_time() {
  local f="$1" c
  c="$(sips -g creation "$f" 2>/dev/null | awk -F': ' '/creation:/{print $2}')"
  if [ -n "$c" ]; then
    echo "$c" | sed 's/[: ]//g' | awk '{print substr($0,1,8)"-"substr($0,9,6)}'
  else
    date -r "$f" +%Y%m%d-%H%M%S
  fi
}

OK=0; SKIP=0
for f in "${FILES[@]}"; do
  base="$(basename "$f")"
  ts="$(shot_time "$f")"
  day="${ts%%-*}"
  target="${ts}_${PHASE}"

  # 同一秒可能拍多张（连拍），目标名冲突就加序号 —— 绝不静默丢弃照片
  if [ -e "$DIAG/${target}.jpg" ]; then
    n=2
    while [ -e "$DIAG/${target}-$(printf '%02d' $n).jpg" ]; do n=$((n+1)); done
    target="${target}-$(printf '%02d' $n)"
  fi

  echo "▸ $base  →  ${target}.jpg  (拍摄于 $ts)"
  if [ "$DRY" -eq 1 ]; then OK=$((OK+1)); continue; fi

  mkdir -p "$ORIG/$day"
  arch="$base"
  if [ -e "$ORIG/$day/$arch" ]; then
    stem="${base%.*}"; ext="${base##*.}"; n=2
    while [ -e "$ORIG/$day/${stem}-$(printf '%02d' $n).${ext}" ]; do n=$((n+1)); done
    arch="${stem}-$(printf '%02d' $n).${ext}"
  fi

  # 1) 原图归档：复制 → 校验 → 才删原件
  if ! cp -p "$f" "$ORIG/$day/$arch"; then
    echo "  ❌ 原图归档失败，保留 inbox 原件"; continue
  fi
  s1=$(stat -f%z "$f"); s2=$(stat -f%z "$ORIG/$day/$arch")
  if [ "$s1" != "$s2" ]; then
    echo "  ❌ 归档校验不一致（$s1 vs $s2），保留 inbox 原件"; continue
  fi

  # 2) 诊断版：统一转 JPG + 压到长边 MAXDIM
  if ! sips -s format jpeg -Z "$MAXDIM" "$f" --out "$DIAG/${target}.jpg" >/dev/null 2>&1; then
    echo "  ❌ 转换失败，保留 inbox 原件"
    rm -f "$ORIG/$day/$arch"; continue
  fi

  # 3) 原件已安全归档且诊断版已生成，才清 inbox
  rm -f "$f"
  echo "  ✅ 原图 → 原图/$day/$arch   诊断版 $(ls -lh "$DIAG/${target}.jpg" | awk '{print $5}')"
  OK=$((OK+1))
done

echo
echo "完成：处理 $OK 个，跳过 $SKIP 个"

if [ "$DO_TAG" -eq 1 ] && [ "$DRY" -eq 0 ] && [ "$OK" -gt 0 ]; then
  echo
  echo "🏷  本地模型打标签中（需要 LM Studio 开着）..."
  python3 "$PLATFORM/tools/vlm.py" --batch "$DIAG" || \
    echo "⚠️  打标签失败，照片已就位不受影响。稍后可单独跑: python3 tools/vlm.py --batch 60_影像/诊断"
fi

echo
echo "📂 诊断版在: 60_影像/诊断/    （告诉我看哪张，或直接让我全看）"
