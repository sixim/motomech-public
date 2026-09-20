#!/usr/bin/env bash
# pdf2pages.sh — 把翻拍的手册 PDF 拆成「每页一图 + 每页一 md」
#
# 为什么要拆：
#   翻拍页是 300dpi 整页 JPEG，一页 5MB，一本书 2GB，而同样的内容还躺着
#   raw 源文件和 .ocr 中间产物各一份 —— 6GB 里有 3 份同样的东西。
#   拆成页之后单页可直接打开核对（双层检索的「回原图」那一步不用再翻 2GB 的 PDF）。
#
# 分辨率为什么是 150dpi（2026-09-16 实测定的，不是拍脑袋）：
#   · 扭力表小号数字 + 车型限定词（"R1200 GS, RT and R"）清晰可读
#     —— 后者是适用性判定的命脉，糊了就会拿错档的值
#   · **彩色电路图**（p261–288，ABS/线束）线色可分、端子号与线径标注可读
#   · 300dpi 每页 1.1MB，150dpi 每页 ~350KB，信息没丢而体积小 3 倍
#   再往下（120dpi）开始发虚，不取。
#
# 用法: bash tools/pdf2pages.sh <手册目录>
#   目录里要有一个同名 PDF。输出到 <手册目录>/pages/
#   **只增不删** —— 原件一个都不动，验证通过后由人决定删不删。
set -euo pipefail

DIR="${1:-}"
[ -z "$DIR" ] && { sed -n '2,20p' "$0"; exit 1; }
[ -d "$DIR" ] || { echo "❌ 不是目录: $DIR" >&2; exit 1; }

PDF="$(find "$DIR" -maxdepth 1 -name '*.pdf' | head -1)"
[ -z "$PDF" ] && { echo "❌ $DIR 下没有 PDF" >&2; exit 1; }

for c in pdfimages pdftotext magick pdfinfo; do
  command -v "$c" >/dev/null || { echo "❌ 缺 $c（brew install poppler imagemagick）" >&2; exit 1; }
done

N=$(pdfinfo "$PDF" | awk '/^Pages:/{print $2}')
OUT="$DIR/pages"
mkdir -p "$OUT"
echo "📖 $(basename "$PDF")  $N 页 → $OUT"
echo "   分辨率 150dpi（宽 1276px，q82）—— 实测可读扭力小字与彩色电路图"

WIDTH=1276; Q=82
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
fallback=0

for ((p=1; p<=N; p++)); do
  f=$(printf "p%03d" "$p")
  img="$OUT/$f.jpg"; md="$OUT/$f.md"

  if [ ! -s "$img" ]; then
    rm -f "$TMP"/x-*
    # 优先无损抽出内嵌 JPEG 再缩放（比重新栅格化保真）
    pdfimages -j -f "$p" -l "$p" "$PDF" "$TMP/x" 2>/dev/null || true
    src="$(ls "$TMP"/x-*.jpg "$TMP"/x-*.ppm 2>/dev/null | head -1 || true)"
    if [ -n "$src" ]; then
      magick "$src" -resize "${WIDTH}x" -quality "$Q" -strip "$img"
    else
      # 页里不是单张内嵌图（矢量/多图）—— 退回整页栅格化，同样 150dpi
      pdftoppm -jpeg -r 150 -f "$p" -l "$p" "$PDF" "$TMP/r" 2>/dev/null
      rs="$(ls "$TMP"/r-*.jpg 2>/dev/null | head -1 || true)"
      [ -n "$rs" ] && { magick "$rs" -resize "${WIDTH}x" -quality "$Q" -strip "$img"; fallback=$((fallback+1)); }
    fi
  fi

  if [ ! -s "$md" ]; then
    txt="$(pdftotext -layout -f "$p" -l "$p" "$PDF" - 2>/dev/null || true)"
    {
      echo "---"
      echo "pdf_page: $p"
      echo "image: $f.jpg"
      echo "source_pdf: $(basename "$PDF")"
      echo "---"
      echo
      # 文本层只作检索与定位用；**数值一律以同页图片为准**
      printf '%s\n' "$txt"
    } > "$md"
  fi

  (( p % 25 == 0 )) && echo "   … $p/$N"
done

echo "✅ 完成：$(ls "$OUT"/*.jpg 2>/dev/null | wc -l | tr -d ' ') 图 / $(ls "$OUT"/*.md 2>/dev/null | wc -l | tr -d ' ') md / 共 $(du -sh "$OUT" | cut -f1)"
[ "$fallback" -gt 0 ] && echo "   ℹ️ 其中 $fallback 页走了整页栅格化（页内不是单张内嵌图）"
echo "   原件一个没动 —— 验证通过前不要删任何东西"
