#!/usr/bin/env bash
# install-bin.sh — 把 release 二进制装到 bin/(常驻服务用的那个)
#
# ⚠️ 2026-09-19 踩到的坑:直接 `cp` 覆盖**正在运行**的二进制,
#    会破坏它的代码签名,macOS 随后用 SIGKILL 杀掉 —— 表现是
#    `repairbench --help` 退出码 **137**、没有任何输出、也没有报错信息。
#    很容易被误判成「程序坏了」,实际是签名问题。
#
# 三件事缺一不可:
#    ① 先停服务(别覆盖运行中的镜像)
#    ② 重新 ad-hoc 签名(cp 会让签名失效)
#    ③ 用 mv 原子替换,不是 cp
set -euo pipefail
ROOT="$HOME/Ai Developer/AI+摩托维修助理"
SRC="$ROOT/repairbench/src-tauri/target/release/repairbench"
DST="$ROOT/bin/repairbench"
PLIST="$HOME/Library/LaunchAgents/com.simon.repairbench.serve.plist"

[ -f "$SRC" ] || { echo "❌ 没有 release 二进制,先 cargo build --release" >&2; exit 1; }

launchctl unload "$PLIST" 2>/dev/null || true
cp "$SRC" "$DST.new"
codesign -s - -f "$DST.new" >/dev/null 2>&1
mv -f "$DST.new" "$DST"
launchctl load "$PLIST" 2>/dev/null || true

# 验证:退出码必须是 0。137 就是上面说的签名问题。
if "$DST" --help >/dev/null 2>&1; then
  echo "✅ 已安装并可执行"
else
  echo "❌ 装完跑不起来(退出码 $?)—— 137 = 签名被杀,检查 codesign" >&2; exit 1
fi
until curl -s -m1 http://127.0.0.1:8765/health >/dev/null 2>&1; do sleep 1; done
echo "✅ 常驻服务已起"
