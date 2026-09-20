#!/usr/bin/env bash
# snap.sh — 摄像头抓帧，自动枚举设备
#
# 用法:
#   bash tools/snap.sh                    抓一帧（自动选最佳设备）
#   bash tools/snap.sh -l                 列出可用摄像头
#   bash tools/snap.sh -t frontbrake      带标签（进文件名，便于归档检索）
#   bash tools/snap.sh -d "设备名"        指定设备
#   bash tools/snap.sh -p P3              指定维修阶段（默认 P0）
#   bash tools/snap.sh -b                 翻拍台模式：按回车连续抓帧，q 退出
#
# 输出: 60_影像/inbox/YYYYMMDD_阶段_标签_snap_序号.jpg
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
OUTDIR="$ROOT/60_影像/inbox"
DEVICE=""; TAG="snap"; PHASE="P0"; BATCH=0; WARMUP=2

if ! command -v imagesnap >/dev/null 2>&1; then
  echo "❌ imagesnap 未安装。先跑: bash tools/setup.sh" >&2
  exit 1
fi

while getopts "ld:t:p:bh" opt; do
  case $opt in
    l) echo "=== 可用摄像头 ==="; imagesnap -l; exit 0 ;;
    d) DEVICE="$OPTARG" ;;
    t) TAG="$OPTARG" ;;
    p) PHASE="$OPTARG" ;;
    b) BATCH=1 ;;
    h) sed -n '2,12p' "$0"; exit 0 ;;
    *) exit 1 ;;
  esac
done

# 自动选设备：iPhone（连续互通，画质最好）> 外接 USB > 内置
if [ -z "$DEVICE" ]; then
  # imagesnap -l 的实际输出是 "=> 设备名"，首行是 "Video Devices:"
  DEVLIST="$(imagesnap -l 2>/dev/null | sed '1d' | sed 's/^=>[[:space:]]*//' | sed 's/^<[^>]*>//' | sed 's/^[[:space:]]*//' | grep -v '^$' || true)"
  DEVICE="$(echo "$DEVLIST" | grep -i "iphone" | head -1 || true)"
  [ -z "$DEVICE" ] && DEVICE="$(echo "$DEVLIST" | grep -viE "facetime|built-?in|macbook" | head -1 || true)"
  [ -z "$DEVICE" ] && DEVICE="$(echo "$DEVLIST" | head -1 || true)"
fi

if [ -z "$DEVICE" ]; then
  echo "❌ 没找到任何摄像头。检查: 系统设置 → 隐私与安全性 → 摄像头，是否已授权终端" >&2
  exit 1
fi

mkdir -p "$OUTDIR"
DATE="$(date +%Y%m%d)"

snap_one() {
  local n=1
  while [ -e "$(printf '%s/%s_%s_%s_snap_%02d.jpg' "$OUTDIR" "$DATE" "$PHASE" "$TAG" "$n")" ]; do
    n=$((n+1))
  done
  local out; out="$(printf '%s/%s_%s_%s_snap_%02d.jpg' "$OUTDIR" "$DATE" "$PHASE" "$TAG" "$n")"
  # 不要把 stderr 扔掉：imagesnap 失败的真实原因就在里面。
  # 2026-09-16 实测：设备能枚举（连 iPhone 都列得出来），抓帧却报
  # "Camera access not granted" —— 旧代码只说「抓帧失败」，
  # 人会以为是设备或线的问题，实际上是一次系统授权就能解开。
  # `|| true` 不能省：set -e 下命令替换失败会直接终止函数，
  # 连下面的判断都走不到 —— 那样就成了「退出码 1 但一个字不说」。
  local err=""
  err="$(imagesnap -q -d "$DEVICE" -w "$WARMUP" "$out" 2>&1 >/dev/null)" || true
  if [ -s "$out" ]; then
    echo "✅ $out"
  else
    rm -f "$out"
    if echo "$err" | grep -qi "access not granted\|not authorized\|permission"; then
      cat >&2 <<'EOF'
❌ 摄像头权限未授予 —— 不是设备问题，设备本身能列出来。

  修法（一次，之后永久生效）：
    系统设置 → 隐私与安全性 → 摄像头 → 打开你运行本脚本的那个程序
    （终端 Terminal / iTerm / VS Code / Claude —— 谁跑的就授权谁）
    授权后**必须完全退出并重开**那个程序，权限才生效。

  没在列表里？先手动跑一次 `imagesnap -w 2 /tmp/t.jpg`，
  系统会弹授权框，之后它才会出现在上面那个列表里。
EOF
    else
      echo "❌ 抓帧失败（设备: $DEVICE）：${err:-无错误输出}" >&2
    fi
    return 1
  fi
}

echo "📷 设备: $DEVICE"

if [ "$BATCH" -eq 1 ]; then
  echo "翻拍台模式：回车抓一帧，输入 q 回车退出"
  WARMUP=1   # 翻拍台是固定机位，不需要长预热
  while true; do
    read -r -p "› " key || break
    [ "$key" = "q" ] && break
    snap_one || true
  done
  echo "结束。"
else
  snap_one
fi
