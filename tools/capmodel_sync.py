#!/usr/bin/env python3
"""
capmodel_sync.py —— 从明细表重新生成能力模型的汇总表

为什么要有这个脚本：
  2026-09-14 监工两次抓到同一个问题 ——
  第一次：汇总表是手写的，与明细对不上。
  第二次：文件里加了一句「本表由明细表自动统计得出」，**但并没有**，L2 仍然错。

  「声称自动」比「承认手写」更糟：它让读者不再去核对。
  所以修法不是再手改一遍，是真的把它生成出来。

用法:
  python3 tools/capmodel_sync.py           # 就地重写汇总表
  python3 tools/capmodel_sync.py --check   # 只检查是否一致，不一致退出码 1（可进 CI）
"""
import io, re, sys, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
F = os.path.join(ROOT, "knowledge/能力体系/技师能力模型.md")

# 明细行形如：| L2.3 | 名称 | ✅ 说明 |
ROW = re.compile(r"^\|\s*(L\d)\.(\d+)\s*\|([^|]*)\|(.*?)\|\s*$", re.M)
# 汇总表：表头行 + 分隔行 + 数据行，直到空行
SUMMARY = re.compile(r"(^\| 层 \| ✅ \| 🔶 \| ❌/🔍 \| 优先级理由 \|\n\|[-|]+\|\n)((?:\|.*\n)+)", re.M)

# 优先级理由是人写的判断，不是统计出来的 —— 按层名保留，不由脚本编造
def parse(text):
    stats = {}
    for lay, num, name, status in ROW.findall(text):
        s = status
        if "✅" in s:   k = "ok"
        elif "🔶" in s: k = "half"
        else:           k = "todo"      # ❌ / 🔍 / 空
        stats.setdefault(lay, {"ok":0,"half":0,"todo":0,"n":0})
        stats[lay][k] += 1
        stats[lay]["n"] += 1
    return stats

def main():
    check = "--check" in sys.argv
    text = io.open(F, encoding="utf-8").read()
    stats = parse(text)
    m = SUMMARY.search(text)
    if not m:
        print("❌ 找不到汇总表（表头必须是 `| 层 | ✅ | 🔶 | ❌/🔍 | 优先级理由 |`）")
        sys.exit(2)

    old_rows = m.group(2).rstrip("\n").split("\n")
    new_rows, changed = [], []
    for r in old_rows:
        cells = [c.strip() for c in r.strip().strip("|").split("|")]
        if len(cells) < 5:
            new_rows.append(r); continue
        label, reason = cells[0], cells[4]
        lay = re.match(r"(L\d)", label)
        if not lay or lay.group(1) not in stats:
            new_rows.append(r); continue
        st = stats[lay.group(1)]
        row = f"| {label} | {st['ok']} | {st['half']} | {st['todo']} | {reason} |"
        if row != r.rstrip():
            changed.append(f"{label}: 表里 {cells[1]}/{cells[2]}/{cells[3]} → 明细实为 {st['ok']}/{st['half']}/{st['todo']}（共 {st['n']} 项）")
        new_rows.append(row)

    if check:
        if changed:
            print("❌ 汇总表与明细不符：")
            for c in changed: print("   ·", c)
            sys.exit(1)
        print("✅ 汇总表与明细一致")
        return

    if not changed:
        print("✅ 已经一致，无需改动"); return
    out = text[:m.start(2)] + "\n".join(new_rows) + "\n" + text[m.end(2):]
    io.open(F, "w", encoding="utf-8").write(out)
    print("✅ 已按明细重算汇总表：")
    for c in changed: print("   ·", c)

if __name__ == "__main__":
    main()
