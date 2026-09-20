#!/usr/bin/env bash
# listen.sh — 异响诊断双通道
#
# 通道 1: 音频 → 频谱图 PNG  → Claude 看频域形态
# 通道 2: 音频 → Qwen2.5-Omni → 纯感知描述（⚠️ 提示词必须中性，见 omni_listen.py 头部）
# 两条通道基于不同信息，交叉印证比任何单通道可靠。
#
# 【2026-09-14 实测边界】用合成音验证过（42Hz 基频+谐波 + 3.2kHz 啸叫）：
#   · Omni 只报告了"低频、连续、弱、有规律重复"，**完全漏掉 3.2kHz 啸叫**
#   · 频谱图上 3200Hz 是一条贯穿全程的亮线，一眼可见
#   → Omni 对高频细节不敏感。轴承啸叫、气门响这类高频特征**必须看频谱图**。
#     这不是缺陷，正是双通道存在的理由：单用任何一条都会漏。
#   性能：21G 模型加载 13s + 推理 6s，可接受。
#
# 用法:
#   bash tools/listen.sh 录音.m4a              两个通道都跑
#   bash tools/listen.sh 录音.m4a --spec-only  只出频谱图（Omni 没装好时用）
#   bash tools/listen.sh 视频.mov              视频会先抽音轨
#
# 输出到 60_影像/inbox/（频谱图）和终端（Omni 描述）
set -euo pipefail

# -h / --help：只看用法，**不要顺手把流水线跑起来**
case "${1:-}" in
  -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
esac

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
SRC="${1:-}"
MODE="${2:-full}"
[ -z "$SRC" ] && { sed -n '2,12p' "$0"; exit 1; }
[ -f "$SRC" ] || { echo "❌ 找不到文件: $SRC" >&2; exit 1; }

command -v ffmpeg >/dev/null 2>&1 || { echo "❌ 缺 ffmpeg" >&2; exit 1; }

STAMP="$(date +%Y%m%d_%H%M%S)"
BASE="$(basename "${SRC%.*}")"
WORK="$(mktemp -d)"
WAV="$WORK/audio.wav"
OUTDIR="$ROOT/60_影像/inbox"
mkdir -p "$OUTDIR"

echo "🎧 提取音轨（16kHz 单声道）..."
ffmpeg -v error -y -i "$SRC" -ac 1 -ar 16000 "$WAV"
DUR="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$WAV" | cut -d. -f1)"
echo "   时长 ${DUR}s"

# ── 通道 1: 频谱图 ──────────────────────────────────────────────
SPEC="$OUTDIR/${STAMP}_${BASE}_spectrum.png"
echo "📊 通道1: 生成频谱图..."
# lin 频率轴 + 对数幅度，机械异响的谐波结构在线性轴上更容易看出基频与倍频关系
ffmpeg -v error -y -i "$WAV" \
  -lavfi "showspectrumpic=s=1600x900:mode=combined:scale=log:legend=1" \
  "$SPEC"
echo "   ✅ $SPEC"

# 波形图作为补充：看冲击性异响（爆震、敲缸）的时域包络
WAVE="$OUTDIR/${STAMP}_${BASE}_waveform.png"
ffmpeg -v error -y -i "$WAV" \
  -lavfi "showwavespic=s=1600x400:colors=white" "$WAVE"
echo "   ✅ $WAVE"

# ── 通道 2: Qwen2.5-Omni 听 ────────────────────────────────────
if [ "$MODE" = "--spec-only" ]; then
  echo "⏭  跳过 Omni 通道"
else
  echo "🤖 通道2: Qwen2.5-Omni 听音中（首次加载 21G 模型较慢）..."
  HF_HOME="$HOME/Models/huggingface" python3 "$PLATFORM/tools/omni_listen.py" "$WAV" || {
    echo "   ⚠️  Omni 通道失败。频谱图仍然可用，把上面两张图发给 Claude。"
  }
fi

rm -rf "$WORK"
echo
echo "下一步: 把频谱图和波形图发给 Claude，连同 Omni 的描述一起判断。"
echo "同时告诉 Claude: 异响出现在什么工况（冷车/热车/转速/挡位/是否随车速变化）——"
echo "这个信息比声音本身更能定位故障。"
