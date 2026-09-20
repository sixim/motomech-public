#!/usr/bin/env python3
"""
scancheck.py — 扫描质量体检

扫 300 页，中间漏了 5 页、糊了 3 页，当场是发现不了的。
等三个月后要查那一章才发现，书可能都不在手边了。
这个脚本在你扫完当天就把问题揪出来。

用法:
  python3 tools/scancheck.py                     体检 raw/ 里所有 PDF
  python3 tools/scancheck.py --pdf 某个.pdf      只查一个
  python3 tools/scancheck.py --dpi 100           降分辨率加速（默认 100，够用）

检查项:
  模糊 · 过暗 · 反光过曝 · 空白页 · 页码跳号 · 文字量异常
"""
import argparse, glob, os, re, subprocess, sys, tempfile
import numpy as np
from PIL import Image

PLATFORM = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_vehicle(v=None):
    """车辆解析：参数 > 环境变量 > .current-vehicle。绝不猜。"""
    v = v or os.environ.get("VEHICLE")
    if not v:
        f = os.path.join(PLATFORM, ".current-vehicle")
        if os.path.exists(f):
            v = open(f, encoding="utf-8").read().strip()
    vdir = os.path.join(PLATFORM, "vehicles")
    if not v or not os.path.isdir(os.path.join(vdir, v)):
        avail = sorted(os.listdir(vdir)) if os.path.isdir(vdir) else []
        raise SystemExit(f"❌ 未指定或不存在的车辆: {v!r}\n   可用: {avail}\n"
                         f"   用 --vehicle <id>，或 bash tools/vehicle.sh use <id>")
    return os.path.join(vdir, v)


# ⚠️ ROOT/RAW 必须等参数解析完再定，不能在模块加载时就定死——
#    否则 --vehicle 被静默忽略，会操作到错误车辆的数据。
ROOT = None
RAW = None


def _bind_vehicle(v=None):
    global ROOT, RAW
    ROOT = resolve_vehicle(v)
    RAW = os.path.join(ROOT, "10_手册资料", "third-party", "haynes-4598", "raw")

# 阈值（用合成缺陷页校准，--dpi 100）
#
# ⚠️ 踩过的坑：最初用"亮像素占比"判反光，结果正常白纸页报 86% 全军覆没。
#    白底文档本来就大面积是白的。反光的真正特征是**局部又亮又平**
#    ——文字被光冲掉，那一块没有细节——而且必须排除页边空白。
#    清晰度也必须先做亮度归一化，否则暗页会被误判成糊。
BLUR_MIN     = 60.0    # 归一化后的拉普拉斯方差
DARK_MAX     = 110.0   # 原始平均亮度
# 反光不做自动判据：试过"亮像素占比"和"中心区亮且平的块占比"，
# 正常文档页与反光页都在 22% 左右，分不开——白纸页本来就有大量
# 行距和表格空白。误报会让人忽略所有警告，代价比漏报大。
# 反光肉眼一眼就能看出来；真正难发现的是漏页和糊页，那两项这里能可靠抓到。
# glare 值仍然算出来显示，仅供参考，不触发警告。
CONTENT_MIN  = 0.006   # 归一化后的内容像素占比


def analyze(path):
    im = Image.open(path).convert("L")
    a = np.asarray(im, dtype=np.float64)

    # 亮度归一化：把 5–95 百分位拉伸到 0–255，消除曝光差异对后续指标的干扰
    lo, hi = np.percentile(a, [5, 95])
    norm = np.clip((a - lo) / (hi - lo) * 255, 0, 255) if hi - lo > 10 else a.copy()

    lap = (norm[:-2, 1:-1] + norm[2:, 1:-1] + norm[1:-1, :-2] + norm[1:-1, 2:]
           - 4 * norm[1:-1, 1:-1])

    # 反光：只看中心 70%（排除页边留白），找"均值高且内部方差低"的块
    h, w = a.shape
    y0, y1 = int(h * 0.15), int(h * 0.85)
    x0, x1 = int(w * 0.15), int(w * 0.85)
    c = a[y0:y1, x0:x1]
    N = 16
    bh, bw = c.shape[0] // N, c.shape[1] // N
    glare = 0.0
    if bh > 2 and bw > 2:
        blocks = c[:bh * N, :bw * N].reshape(N, bh, N, bw)
        bmean = blocks.mean(axis=(1, 3))
        bvar = blocks.var(axis=(1, 3))
        glare = float(((bmean > 248) & (bvar < 8)).mean())

    return {
        "blur":    float(lap.var()),
        "bright":  float(a.mean()),
        "glare":   glare,
        "content": float((norm < 180).mean()),
        # ⚠️ content 是在 5–95 百分位拉伸**之后**算的 —— 近乎空白的页被拉伸后，
        #    纸张纹理占满整个动态范围，于是空白页也能报出 78% 的「内容」。
        #    归一化恰好毁掉了判断空白所需的信息。
        #    ink 用原图算，且以这一页自己的纸面亮度为基准，
        #    因此不受暖色偏移和光照不均影响（实测本书 R/B=1.5、四角亮度差 23%）。
        "ink": float((a < np.percentile(a, 85) * 0.6).mean()),
    }


def verdict(m):
    bad = []
    if m["blur"] < BLUR_MIN:        bad.append("糊")
    if m["bright"] < DARK_MAX:      bad.append("暗")
    if m["content"] < CONTENT_MIN:  bad.append("空白")
    return bad


def page_numbers(text):
    """从页脚挖页码。

    ⚠️ 只匹配**独占一行**的页码（Haynes 页脚形如 2A•5，或纯数字）。
       早期版本匹配行内数字，结果把扭力值表里的数字全当成页码，
       "illustration 4.7" 被读成页码 4 和 7，报出一堆假跳号。
       扭力值表满页都是数字，宽松匹配在这本书上必然失效。
    """
    nums = []
    lines = [l.strip() for l in text.strip().splitlines()]
    for line in lines[-3:] + lines[:1]:
        if not line or len(line) > 12:
            continue
        m = re.fullmatch(r'(\d{1,3})', line)
        if m:
            nums.append(int(m.group(1))); continue
        m = re.fullmatch(r'(\d{1,2}[A-Z]?)[•·\-–](\d{1,3})', line)
        if m:
            nums.append(int(m.group(2)))
    return nums


def fingerprint(path, size=16):
    """给一页算个感知指纹：缩到 16x16 灰度，与均值比较得 256 位。
    JPEG 噪声、轻微位移、曝光差异都不影响它，但换一页内容就会大不同。"""
    try:
        from PIL import Image
        im = Image.open(path).convert("L").resize((size, size))
        px = list(im.getdata())
        avg = sum(px) / len(px)
        return [1 if p > avg else 0 for p in px]
    except Exception:
        return None


def hamming(a, b):
    return sum(x != y for x, y in zip(a, b))


def check_pdf(pdf, dpi):
    name = os.path.basename(pdf)
    print(f"\n{'='*62}\n📄 {name}")
    try:
        info = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True).stdout
        total = int(re.search(r"Pages:\s+(\d+)", info).group(1))
    except Exception:
        total = 0
    print(f"   {total} 页，渲染 {dpi}dpi 分析中...")

    tmp = tempfile.mkdtemp()
    subprocess.run(["pdftoppm", "-r", str(dpi), "-jpeg", pdf,
                    os.path.join(tmp, "p")], capture_output=True)
    imgs = sorted(glob.glob(os.path.join(tmp, "p*.jpg")))
    if not imgs:
        print("   ❌ 渲染失败"); return []

    txt = subprocess.run(["pdftotext", "-layout", pdf, "-"],
                         capture_output=True, text=True).stdout.split("\f")

    issues, seen_nums, wordcounts = [], [], []
    contents, inks, fprints = [], [], []
    for i, im in enumerate(imgs, 1):
        m = analyze(im)
        contents.append(m.get('content', 0.0))
        inks.append(m.get('ink', 0.0))
        fprints.append(fingerprint(im))
        bad = verdict(m)
        t = txt[i-1] if i-1 < len(txt) else ""
        wc = len(t.split())
        wordcounts.append(wc)
        seen_nums.append(page_numbers(t))
        if bad:
            issues.append((i, bad, m))
            print(f"   ⚠️  第{i}页  {'/'.join(bad):8s} "
                  f"[清晰度{m['blur']:6.0f} 亮度{m['bright']:5.0f} "
                  f"内容{m['content']*100:4.1f}%]")
        for f in [im]:
            os.unlink(f)
    os.rmdir(tmp)

    # 文字量异常：某页远少于中位数（可能拍坏或拍到桌面）
    if wordcounts:
        med = float(np.median(wordcounts))
        for i, wc in enumerate(wordcounts, 1):
            if med > 40 and wc < med * 0.15 and not any(x[0] == i for x in issues):
                # 光看词数会把整页图解误判成「拍坏了」。
                # 2026-09-14 实测：Haynes 的 Component locations 页（整页车辆图解）
                # 被报了 5 页「文字量异常少」，回原图一看扫描完全正常 —— 全是误报。
                # 加一条：墨迹覆盖率正常 = 纸上确实有东西，只是那东西不是字。
                ink = inks[i-1] if i-1 < len(inks) else 0.0
                if ink >= 0.02:
                    print(f"   ℹ️  第{i}页  字少但墨迹正常（{wc} 词 / 覆盖 {ink*100:.1f}%）"
                          f" —— 多半是整页图解，不是拍坏")
                else:
                    print(f"   ⚠️  第{i}页  文字量异常少且几乎无墨迹"
                          f"（{wc} 词 / 覆盖 {ink*100:.1f}%）—— 可能是空白页或拍到桌面")
                    issues.append((i, ["少字"], {}))

    # 重复页：翻拍时最真实的失误是同一页拍了两次（翻页没翻动）。
    # 这个检测不依赖 OCR，所以在页码读不出来的书上照样有效。
    dups = {}
    for i, fp in enumerate(fprints, 1):
        if fp is None:
            continue
        for j in range(1, i):
            if fprints[j-1] is not None and hamming(fp, fprints[j-1]) <= 4:
                dups.setdefault(j, []).append(i)
                break
    if dups:
        print("\n   ⚠️  疑似重复页（翻页没翻动？）:")
        for a, bs in sorted(dups.items()):
            print(f"      第{a}页 ≈ 第{', '.join(map(str, bs))}页")
            # 必须计入 issues，否则末尾会打「✅ 全部通过，可以跑 bookproc.sh」——
            # 检测出了问题却给绿灯，比不检测更糟（监工 2026-09-14-c）
            for b in bs:
                if not any(x[0] == b for x in issues):
                    issues.append((b, ["重复页"], {}))
    else:
        print(f"\n   ✅ 无重复页（{len(imgs)} 页两两比对）")

    # 页码连续性：只在确实读到页脚页码时才提示，且明说是参考
    flat = [n for ns in seen_nums for n in ns]
    if len(flat) >= max(3, len(imgs) // 3):
        uniq = sorted(set(flat))
        gaps = [(a, b) for a, b in zip(uniq, uniq[1:]) if 1 < b - a <= 12]
        if gaps:
            print(f"\n   📑 页码参考（可能跳号，请对着书核对）:")
            for a, b in gaps[:10]:
                print(f"      {a} → {b}   中间缺 {b-a-1} 个号")
        else:
            print(f"\n   📑 读到 {len(uniq)} 个页码，未见跳号")
    else:
        print(f"\n   📑 跳号检测不可用 —— 页码没读出来。"
              f"\n      Haynes 的页码在页眉且形如 1•14，OCR 会把「•」读成 + 或 9"
              f"（实测 1•2→「+2」、1•14→「1914」），分隔符被毁得没有规律。"
              f"\n      硬写正则只会制造假跳号。替代办法：上面的重复页检测不依赖 OCR；"
              f"漏页请对着书的目录核一次页数。"
              f"\n      （只读到 {len(flat)} 个可信页码，需要至少 {max(3, len(imgs)//3)} 个）")

    print(f"\n   {'✅ 全部通过' if not issues else f'⚠️  {len(issues)} 页需要复查'}")
    return issues


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pdf")
    ap.add_argument("--dpi", type=int, default=100)
    ap.add_argument("--vehicle")
    a = ap.parse_args()
    _bind_vehicle(a.vehicle)
    print(f"车辆: {os.path.basename(ROOT)}")

    pdfs = [a.pdf] if a.pdf else sorted(glob.glob(os.path.join(RAW, "*.pdf")))
    if not pdfs:
        print(f"❌ {RAW} 里没有 PDF"); sys.exit(1)

    allissues = []
    for p in pdfs:
        allissues += check_pdf(p, a.dpi)

    print(f"\n{'='*62}")
    if allissues:
        print(f"总计 {len(allissues)} 页需要重扫或复查。")
        print("重扫时只补这几页即可 —— 把补拍的 PDF 一起丢进 raw/，"
              "bookproc.sh 会一并处理。")
    else:
        print("✅ 全部页面通过体检，可以跑 bookproc.sh 了。")


if __name__ == "__main__":
    main()
