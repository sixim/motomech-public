#!/usr/bin/env bash
# vlm.sh — 本地视觉模型便捷入口（vlm.py 的薄包装）
#
#   bash tools/vlm.sh -l                     列出可用模型
#   bash tools/vlm.sh -b [目录]              批量打标签（默认当前车的 60_影像/诊断）
#   bash tools/vlm.sh 图片.jpg ["提问"]       单图分析
#   加 -v <车辆id> 可指定车辆
#
# 2026-09-14 修复：此前第 15/16/18 行引用未定义的 $PLATFORM，
# 叠加 set -u 导致三条分支全部崩溃（监工审计发现）。
# 根因是批量替换 $ROOT/tools → $PLATFORM/tools 时没让本脚本加载 _common.sh。
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

VEHICLE_ARG=""
_args=(); while [ $# -gt 0 ]; do
  case "$1" in
    -v) VEHICLE_ARG="$2"; shift 2 ;;
    *) _args+=("$1"); shift ;;
  esac
done
set -- "${_args[@]+"${_args[@]}"}"

ROOT="$(resolve_vehicle "$VEHICLE_ARG")" || exit 1
echo "车辆: $(basename "$ROOT")" >&2   # 硬约束：每次操作打印当前车辆

PY="$PLATFORM/tools/vlm.py"
VARG=""; [ -n "$VEHICLE_ARG" ] && VARG="--vehicle $VEHICLE_ARG"

case "${1:-}" in
  -l|--list) python3 "$PY" --list ;;
  -b|--batch) python3 "$PY" $VARG --batch "${2:-$ROOT/60_影像/诊断}" ;;
  ""|-h|--help) sed -n '2,9p' "$0" ;;
  *) python3 "$PY" $VARG --image "$1" ${2:+--prompt "$2"} ;;
esac
