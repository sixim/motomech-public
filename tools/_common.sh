#!/usr/bin/env bash
# _common.sh — 所有工具共享的车辆解析逻辑
#
# 车辆的确定顺序：命令行 -v > 环境变量 VEHICLE > .current-vehicle 文件
# 一台都没有就报错并列出可选项——绝不猜，猜错会把 GS 的数据写进 XL1200

PLATFORM="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VEHICLES_DIR="$PLATFORM/vehicles"
CURRENT_FILE="$PLATFORM/.current-vehicle"

list_vehicles() {
  echo "可用车辆："
  for d in "$VEHICLES_DIR"/*/; do
    [ -d "$d" ] || continue
    n="$(basename "$d")"
    cur=""; [ -f "$CURRENT_FILE" ] && [ "$(cat "$CURRENT_FILE")" = "$n" ] && cur="  ← 当前"
    echo "  · $n$cur"
  done
}

resolve_vehicle() {
  local v="${1:-}"
  [ -z "$v" ] && v="${VEHICLE:-}"
  [ -z "$v" ] && [ -f "$CURRENT_FILE" ] && v="$(cat "$CURRENT_FILE" | tr -d '[:space:]')"
  if [ -z "$v" ]; then
    echo "❌ 未指定车辆。用 -v <id>，或先跑: bash tools/vehicle.sh use <id>" >&2
    list_vehicles >&2; return 1
  fi
  if [ ! -d "$VEHICLES_DIR/$v" ]; then
    echo "❌ 没有这台车: $v" >&2; list_vehicles >&2; return 1
  fi
  echo "$VEHICLES_DIR/$v"
}
