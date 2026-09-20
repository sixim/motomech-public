#!/usr/bin/env bash
# setup.sh — 一次性安装本项目依赖
# 用法: bash tools/setup.sh
set -euo pipefail

echo "=== 2009 R1200GS 维修工作台 · 依赖检查 ==="
echo

if ! command -v brew >/dev/null 2>&1; then
  echo "❌ 未找到 Homebrew。先装 Homebrew: https://brew.sh"
  exit 1
fi

# 需要用 brew 安装的包（包名 → 检测用的命令名）
declare -a PKGS=(
  "imagesnap:imagesnap"      # 摄像头抓帧
  "ocrmypdf:ocrmypdf"        # PDF 加 OCR 文字层
  "imagemagick:magick"       # 图像处理（拆页/增强/纠偏）
  "poppler:pdftotext"        # PDF 转文本
  "img2pdf:img2pdf"          # 图片合成 PDF（无损）
)

MISSING=()
for entry in "${PKGS[@]}"; do
  pkg="${entry%%:*}"; cmd="${entry##*:}"
  if command -v "$cmd" >/dev/null 2>&1; then
    printf "  %-14s ✅ 已装\n" "$pkg"
  else
    printf "  %-14s ⬇️  待装\n" "$pkg"
    MISSING+=("$pkg")
  fi
done

echo
if [ ${#MISSING[@]} -eq 0 ]; then
  echo "✅ brew 依赖齐全，无需安装"
else
  echo "即将安装: ${MISSING[*]}"
  brew install "${MISSING[@]}"
  echo "✅ 安装完成"
fi

echo
echo "=== 已有工具确认（无需安装）==="
for c in tesseract ffmpeg python3; do
  printf "  %-14s " "$c"
  command -v "$c" >/dev/null 2>&1 && echo "✅" || echo "❌ 缺失（意外情况，请告知）"
done

echo
echo "=== 本地模型推理框架 ==="
for p in mlx_vlm mlx_lm transformers; do
  printf "  %-14s " "$p"
  python3 -c "import $p" 2>/dev/null && echo "✅" || echo "❌ 未安装"
done

echo
echo "=== 摄像头设备 ==="
if command -v imagesnap >/dev/null 2>&1; then
  imagesnap -l 2>/dev/null | sed 's/^/  /'
else
  echo "  (装完 imagesnap 后再看)"
fi

echo
echo "完成。下一步: bash tools/snap.sh -l  查看可用摄像头"
