#!/usr/bin/env python3
"""
vlm.py — 本地视觉模型调用（走 LM Studio OpenAI 兼容 API）

用法:
  python3 tools/vlm.py --image 60_影像/inbox/xxx.jpg
      单图分析，默认用维修诊断提示词

  python3 tools/vlm.py --image xxx.jpg --prompt "这个卡钳有锈吗"
      自定义提问

  python3 tools/vlm.py --batch 60_影像/inbox
      批量给目录里所有图打标签，写入 80_Vlog/素材库/tags.json
      （已打过标签的自动跳过，可反复运行）

  python3 tools/vlm.py --list
      列出 LM Studio 里可用的模型

注意: 本地模型负责"看见什么"，不负责"诊断什么"。
      真正的故障判断交给 Claude，它能结合手册、故障码和维修历史推理。

本机模型的视觉能力实测（2026-09-13）:
  ✅ qwen/qwen3.8-27b                        LM Studio 里唯一能看图的
  ❌ gemma-3-12b-it-projection-abliterated   不支持图像输入 —— abliteration
                                             把视觉投影层剥掉了，名字是字面意思
  ⚠️ Qwen2.5-VL-3B / Qwen2-VL-2B / Omni-7B   真视觉模型，但在 huggingface 缓存里，
                                             LM Studio 不认；要走 mlx_vlm，而
                                             mlx_vlm 0.1.15 与 transformers 4.57 冲突
                                             (ValueError: Only returning PyTorch tensors)

  → 结论：打标签和看图都用 27B。它是推理模型，思考先吃 token，配额必须给足。
"""
import argparse, base64, json, os, sys, time, urllib.request, urllib.error

API = "http://localhost:1234/v1"
DEFAULT_MODEL = "qwen/qwen3.8-27b"
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


# ⚠️ 见 scancheck.py 同处注释：不能在模块加载时定死
ROOT = None
TAGS_FILE = None


def _bind_vehicle(v=None):
    global ROOT, TAGS_FILE
    ROOT = resolve_vehicle(v)
    TAGS_FILE = os.path.join(ROOT, "60_影像", "tags.json")
EXTS = (".jpg", ".jpeg", ".png", ".heic", ".webp")

DIAG_PROMPT = """你在看一张 2009 BMW R1200GS（hexhead）摩托车的维修照片。这台车停放了十年，正在修复。
请客观描述你看到的内容，不要臆测：
1. 画面里是哪个部位/部件
2. 可见的状态：锈蚀、油渍、裂纹、变形、脏污、磨损（有就说位置和范围，没有就说没有）
3. 照片本身的质量问题：模糊、过暗、反光、关键部位被遮挡

只描述你确实看见的。看不清就说看不清。"""

# 批量打标签用 /no_think 关闭推理：实测 7.1s vs 17s，快 2.4 倍。
# 打标签是感知任务不需要深度推理，思考纯属浪费。
TAG_PROMPT = """/no_think
只输出 JSON，不要解释：
{"parts":["部件英文名"],"conditions":["rust|oil_leak|crack|dirt|wear|clean，无则空"],"shot_type":"wide|medium|closeup","quality":"good|blurry|dark|glare","vlog_usable":true}"""


def post(payload, timeout=1800):
    req = urllib.request.Request(
        f"{API}/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def shrink(path, maxdim=1024):
    """打标签用小图：实测 2048px 要 250s/张，1024px 只要个位数秒。
    诊断用大图（要看纹理细节），打标签用小图（只要认出是什么部件）。"""
    import subprocess, tempfile
    tmp = tempfile.NamedTemporaryFile(suffix=".jpg", delete=False)
    tmp.close()
    r = subprocess.run(["sips", "-s", "format", "jpeg", "-Z", str(maxdim),
                        path, "--out", tmp.name],
                       capture_output=True)
    if r.returncode == 0 and os.path.getsize(tmp.name) > 0:
        return tmp.name, True
    os.unlink(tmp.name)
    return path, False


def ask(model, image_path, prompt, max_tokens=2000):
    """
    ⚠️ Qwen3.8 是推理模型，思考占用大量 token（实测 239/400）。
    max_tokens 给小了，思考吃光配额，content 会是空字符串。默认给足 2000。
    """
    with open(image_path, "rb") as f:
        b64 = base64.b64encode(f.read()).decode()
    ext = os.path.splitext(image_path)[1].lstrip(".").lower()
    mime = "jpeg" if ext in ("jpg", "jpeg") else ext
    data = post({
        "model": model,
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": prompt},
            {"type": "image_url",
             "image_url": {"url": f"data:image/{mime};base64,{b64}"}},
        ]}],
        "max_tokens": max_tokens,
        "temperature": 0.2,
    })
    msg = data["choices"][0]["message"]
    content = (msg.get("content") or "").strip()
    if not content:
        # 思考吃光了配额，正文没生成。退而取思考内容，聊胜于无，并提示。
        rc = (msg.get("reasoning_content") or "").strip()
        if rc:
            return f"[⚠️ 正文为空，以下是模型思考过程]\n{rc}"
    return content


def extract_json(text):
    """模型可能裹 markdown 代码块或加解释，尽力挖出 JSON"""
    t = text.strip()
    if "```" in t:
        parts = t.split("```")
        for p in parts:
            p = p.strip()
            if p.startswith("json"):
                p = p[4:].strip()
            if p.startswith("{"):
                t = p
                break
    i, j = t.find("{"), t.rfind("}")
    if i >= 0 and j > i:
        try:
            return json.loads(t[i:j + 1])
        except json.JSONDecodeError:
            pass
    return None


def cmd_list():
    with urllib.request.urlopen(f"{API}/models", timeout=30) as r:
        for m in json.loads(r.read()).get("data", []):
            print(" ", m["id"])


def cmd_batch(model, directory):
    tags = {}
    if os.path.exists(TAGS_FILE):
        with open(TAGS_FILE, encoding="utf-8") as f:
            tags = json.load(f)

    files = []
    for dirpath, _, names in os.walk(directory):
        for n in sorted(names):
            if n.lower().endswith(EXTS) and not n.startswith("."):
                files.append(os.path.join(dirpath, n))

    # 上次失败的要重试 —— 失败条目不该永久占位
    def needs_tag(f):
        rel = os.path.relpath(f, ROOT)
        return rel not in tags or "_error" in tags.get(rel, {})
    todo = [f for f in files if needs_tag(f)]
    retry = sum(1 for f in todo if os.path.relpath(f, ROOT) in tags)
    print(f"目录共 {len(files)} 张，已成功 {len(files)-len(todo)} 张，"
          f"本次处理 {len(todo)} 张（其中重试 {retry} 张）\n")

    os.makedirs(os.path.dirname(TAGS_FILE), exist_ok=True)
    for i, path in enumerate(todo, 1):
        rel = os.path.relpath(path, ROOT)
        print(f"[{i}/{len(todo)}] {os.path.basename(path)} ... ", end="", flush=True)
        t0 = time.time()
        try:
            # 推理模型的思考会先吃掉配额，800 不够（实测被截断导致正文全空）。
            # 打标签的正文只要 ~100 token，其余全留给思考。
            small, is_tmp = shrink(path)
            try:
                raw = ask(model, small, TAG_PROMPT, max_tokens=2500)
            finally:
                if is_tmp:
                    os.unlink(small)
            parsed = extract_json(raw)
            if parsed is None:
                print(f"⚠️  JSON 解析失败，存原文")
                tags[rel] = {"_raw": raw[:500], "_error": "json_parse_failed"}
            else:
                parsed["_sec"] = round(time.time() - t0, 1)
                tags[rel] = parsed
                print(f"{parsed.get('parts', [])} {parsed.get('conditions', [])} "
                      f"({parsed['_sec']}s)")
        except Exception as e:
            print(f"❌ {type(e).__name__}: {e}")
            tags[rel] = {"_error": str(e)[:200]}
        # 每张都落盘，中断了也不丢
        with open(TAGS_FILE, "w", encoding="utf-8") as f:
            json.dump(tags, f, ensure_ascii=False, indent=2)

    print(f"\n✅ 标签写入 {os.path.relpath(TAGS_FILE, ROOT)}")


def main():
    ap = argparse.ArgumentParser(add_help=True)
    ap.add_argument("--image")
    ap.add_argument("--batch")
    ap.add_argument("--prompt")
    ap.add_argument("--model", default=DEFAULT_MODEL)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--vehicle")
    a = ap.parse_args()
    _bind_vehicle(a.vehicle)
    print(f"车辆: {os.path.basename(ROOT)}")

    try:
        if a.list:
            cmd_list()
        elif a.batch:
            cmd_batch(a.model, a.batch)
        elif a.image:
            print(ask(a.model, a.image, a.prompt or DIAG_PROMPT))
        else:
            ap.print_help()
    except urllib.error.URLError as e:
        print(f"❌ 连不上 LM Studio ({API})。确认 LM Studio 已启动且 server 开着:\n"
              f"   lms server start\n   {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
