#!/usr/bin/env bash
# bookproc.sh — Haynes 4598 扫描件后处理流水线
#
# 输入: 10_手册资料/third-party/haynes-4598/raw/ 里的 PDF（iOS 扫描导出）
#       或 .jpg/.png（翻拍台模式拍的图）
# 输出: haynes-4598.pdf  (原图 + 隐藏 OCR 文字层，画质不变)
#       haynes-4598.txt  (全文，带 === PAGE n === 标记，供 grep 定位页码)
#
# 用法:
#   bash tools/bookproc.sh          处理 raw/ 里所有文件
#   bash tools/bookproc.sh --force  重做已处理过的
#
# 设计取舍:
#   · 不拆双页。iOS 扫描的双页 PDF，OCR 一样能读，我看图也没问题；
#     拆页只增加出错风险。摊开拍 = 1 个 PDF 页对应书的 2 页，页码映射反而简单。
#   · 不做二值化/去噪增强。Haynes 电路图靠颜色区分线束，
#     任何"增强"都可能毁掉它。iOS 扫描已经做过透视校正和去阴影，够用了。
set -euo pipefail

# -h / --help：只看用法，**不要顺手把流水线跑起来**
case "${1:-}" in
  -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
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
BASE="$ROOT/10_手册资料/third-party/haynes-4598"
RAW="$BASE/raw"; OCR="$BASE/.ocr"
OUT_PDF="$BASE/haynes-4598.pdf"; OUT_TXT="$BASE/haynes-4598.txt"
FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1

for c in ocrmypdf pdfunite pdftotext; do
  command -v "$c" >/dev/null 2>&1 || { echo "❌ 缺 $c，先跑: bash tools/setup.sh" >&2; exit 1; }
done

mkdir -p "$RAW" "$OCR"
shopt -s nullglob nocaseglob

# ── 1. 图片先合成 PDF（翻拍台模式的输入）────────────────────────────
IMGS=("$RAW"/*.jpg "$RAW"/*.jpeg "$RAW"/*.png)
if [ ${#IMGS[@]} -gt 0 ]; then
  if command -v img2pdf >/dev/null 2>&1; then
    echo "📸 发现 ${#IMGS[@]} 张图片，合成 PDF（无损）..."
    img2pdf --output "$RAW/_from_images.pdf" "${IMGS[@]}"
    mkdir -p "$RAW/_images_done" && mv "${IMGS[@]}" "$RAW/_images_done/"
  else
    echo "⚠️  有图片但缺 img2pdf，跳过。装: brew install img2pdf" >&2
  fi
fi

PDFS=("$RAW"/*.pdf)
if [ ${#PDFS[@]} -eq 0 ]; then
  echo "❌ $RAW 里没有 PDF。先按 80_Vlog/../书页拍摄规范 扫描并 AirDrop 过来。" >&2
  exit 1
fi

# ── 2. 逐个 OCR 加文字层 ────────────────────────────────────────────
echo "📖 OCR ${#PDFS[@]} 个文件（保留原图，只加隐藏文字层）"
for f in "${PDFS[@]}"; do
  name="$(basename "$f")"
  dst="$OCR/$name"
  if [ -s "$dst" ] && [ "$FORCE" -eq 0 ]; then
    echo "  ⏭  $name（已处理）"; continue
  fi
  printf "  ⚙️  %s ... " "$name"
  if ocrmypdf --skip-text -l eng --optimize 0 --quiet "$f" "$dst" 2>/dev/null; then
    echo "✅"
  else
    echo "⚠️  OCR 失败，原样收录（该文件无法全文检索）"
    cp "$f" "$dst"
  fi
done

# ── 2.5 清理陈旧产物 ────────────────────────────────────────────────
# raw/ 里改名或删掉的文件，.ocr/ 里的旧产物必须跟着走，
# 否则合并时同一份内容会进两次 —— 页码整体错位，而「回原图确认」全靠页码。
# （2026-09-15 实测：001_ 前缀改名后，旧名产物仍在，合并会得到 232 页而非 182 页。）
for o in "$OCR"/*.pdf; do
  [ -e "$o" ] || continue
  if [ ! -e "$RAW/$(basename "$o")" ]; then
    echo "  🧹 清理陈旧产物：$(basename "$o")（raw/ 里已无同名文件）"
    rm -f "$o"
  fi
done

# ── 3. 合并 + 导出全文 ──────────────────────────────────────────────
# 合并顺序 = 文件名字典序。**这就是为什么文件要用 001_ / 002_ 前缀** ——
# 顺序一变，所有已记录的页码引用集体失效。
OCRD=("$OCR"/*.pdf)
echo "🔗 合并 ${#OCRD[@]} 个文件 → $(basename "$OUT_PDF")"
for o in "${OCRD[@]}"; do echo "     · $(basename "$o")"; done
if [ ${#OCRD[@]} -eq 1 ]; then cp "${OCRD[0]}" "$OUT_PDF"; else pdfunite "${OCRD[@]}" "$OUT_PDF"; fi

# 页码映射表：PDF 第 N 页来自哪个源文件的第几页。
# 书越厚越需要它 —— 没有它，「回原图」只能靠数。
MANIFEST="$(dirname "$OUT_PDF")/页码映射.tsv"
{
  echo -e "pdf_page\t源文件\t源文件内页码"
  off=0
  for o in "${OCRD[@]}"; do
    n=$(pdfinfo "$o" 2>/dev/null | awk '/^Pages:/{print $2}')
    [ -z "$n" ] && n=0
    for i in $(seq 1 "$n"); do
      echo -e "$((off + i))\t$(basename "$o")\t$i"
    done
    off=$((off + n))
  done
} > "$MANIFEST"
echo "🗺  页码映射 → $(basename "$MANIFEST")（$(( $(wc -l < "$MANIFEST") - 1 )) 行）"

echo "📝 导出全文（带页码标记）"
pdftotext -layout "$OUT_PDF" "$OUT_TXT.tmp"
python3 - "$OUT_TXT.tmp" "$OUT_TXT" <<'PY'
import sys
src, dst = sys.argv[1], sys.argv[2]
pages = open(src, encoding="utf-8", errors="replace").read().split("\f")
with open(dst, "w", encoding="utf-8") as f:
    for i, p in enumerate(pages, 1):
        if p.strip():
            f.write(f"\n=== PAGE {i} ===\n{p}")
print(f"   共 {len(pages)} 页")
PY
rm -f "$OUT_TXT.tmp"

echo
echo "✅ 完成"
echo "   PDF: $(basename "$OUT_PDF")  ($(du -h "$OUT_PDF" | cut -f1))"
echo "   TXT: $(basename "$OUT_TXT")  ($(wc -l < "$OUT_TXT" | tr -d ' ') 行)"
echo
echo "检索示例:  grep -n -i -B2 -A6 'valve clearance' '$OUT_TXT' | head -40"
echo "找到页码后告诉我，我直接读 PDF 那一页看原图。"
