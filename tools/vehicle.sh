#!/usr/bin/env bash
# vehicle.sh — 查看/切换当前车辆
#   bash tools/vehicle.sh            显示当前车辆
#   bash tools/vehicle.sh list       列出所有车辆
#   bash tools/vehicle.sh use <id>   切换当前车辆
#   bash tools/vehicle.sh info       显示当前车辆的关键标识
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

case "${1:-show}" in
  list) list_vehicles ;;
  use)
    [ -n "${2:-}" ] || { echo "用法: vehicle.sh use <id>" >&2; list_vehicles >&2; exit 1; }
    [ -d "$VEHICLES_DIR/$2" ] || { echo "❌ 没有这台车: $2" >&2; list_vehicles >&2; exit 1; }
    echo "$2" > "$CURRENT_FILE"; echo "✅ 当前车辆 → $2" ;;
  info)
    R="$(resolve_vehicle)" || exit 1
    echo "路径: $R"
    [ -f "$R/VEHICLE.yml" ] && sed -n '1,25p' "$R/VEHICLE.yml" || echo "(无 VEHICLE.yml)" ;;
  *)
    if [ -f "$CURRENT_FILE" ]; then echo "当前车辆: $(cat "$CURRENT_FILE")"; else echo "未设置当前车辆"; list_vehicles; fi ;;
esac
