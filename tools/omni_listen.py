#!/usr/bin/env python3
"""
omni_listen.py — Qwen2.5-Omni 听音频，输出纯感知描述

被 listen.sh 调用，也可单独用:
    HF_HOME=~/Models/huggingface python3 tools/omni_listen.py 音频.wav

╔══════════════════════════════════════════════════════════════════╗
║ ⚠️ 为什么提示词里一个字都不能提"摩托车"或"发动机"                ║
║                                                                  ║
║ 2026-09-12 对照实验（同一段室内打包行李的录音）:                 ║
║                                                                  ║
║   中性提示 → "喷雾罐的声音、揉纸的声音、东西被扔掉的声音"        ║
║              ✅ 与视频画面完全吻合                                ║
║                                                                  ║
║   暗示提示 → "机械运转的轰鸣声、发动机的咔哒声"                  ║
║   ("这是摩托车  ❌ 纯属顺着提示词编造，画面里根本没有发动机       ║
║     发动机录音")                                                 ║
║                                                                  ║
║ 结论: 它的听力是真的，但顺从性极强 —— 你暗示什么它就"听到"什么。 ║
║ 诊断时若告诉它"这是异响"，它会附和着编出故障来，                 ║
║ 那比没有这个通道更危险。                                         ║
║                                                                  ║
║ 所以: 提示词保持中性，判断交给 Claude。                          ║
╚══════════════════════════════════════════════════════════════════╝
"""
import sys, time

MODEL_ID = "Qwen/Qwen2.5-Omni-7B"
MAX_SEC = 30

SYSTEM = ("You are Qwen, a virtual human developed by the Qwen Team, Alibaba Group, "
          "capable of perceiving auditory and visual inputs, as well as generating "
          "text and speech.")

# ⚠️ 中性提示词。不要加入任何关于声音来源的暗示。见文件头的对照实验。
PROMPT = """描述这段音频里你听到的所有声音。

要求：
1. 逐一列出你听到的声音成分
2. 每种声音的特征：高频还是低频、连续还是断续、强还是弱
3. 是否有规律的重复？周期大概多长？
4. 有没有突发的、和其他声音不同的事件？出现在第几秒？

只描述你确实听到的。不要推测声音的来源，不要推测发生了什么。
听不清的地方就说听不清。"""


def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    wav = sys.argv[1]
    prompt = sys.argv[2] if len(sys.argv) > 2 else PROMPT

    import torch, soundfile as sf
    from transformers import Qwen2_5OmniProcessor

    audio, sr = sf.read(wav)
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    if len(audio) > MAX_SEC * sr:
        print(f"   (音频 {len(audio)//sr}s，截取前 {MAX_SEC}s)")
        audio = audio[: MAX_SEC * sr]

    dev = "mps" if torch.backends.mps.is_available() else "cpu"
    print(f"   加载 Thinker 到 {dev} (bf16)...", flush=True)
    t0 = time.time()
    try:
        from transformers import Qwen2_5OmniThinkerForConditionalGeneration as Thinker
        model = Thinker.from_pretrained(MODEL_ID, dtype=torch.bfloat16).to(dev).eval()
    except Exception:
        from transformers import Qwen2_5OmniForConditionalGeneration as Omni
        model = Omni.from_pretrained(MODEL_ID, dtype=torch.bfloat16).to(dev).eval()
        if hasattr(model, "disable_talker"):
            model.disable_talker()

    proc = Qwen2_5OmniProcessor.from_pretrained(MODEL_ID)
    print(f"   加载完成 {time.time()-t0:.0f}s，推理中...", flush=True)

    conv = [
        {"role": "system", "content": [{"type": "text", "text": SYSTEM}]},
        {"role": "user", "content": [
            {"type": "audio", "audio": wav},
            {"type": "text", "text": prompt},
        ]},
    ]
    text = proc.apply_chat_template(conv, add_generation_prompt=True, tokenize=False)
    inputs = proc(text=text, audio=[audio], sampling_rate=sr,
                  return_tensors="pt", padding=True)
    inputs = {k: (v.to(dev) if hasattr(v, "to") else v) for k, v in inputs.items()}

    # eos_token_id 必须显式传，否则模型会自问自答停不下来（实测会生成 "Human:" 循环）
    eos = proc.tokenizer.eos_token_id
    t1 = time.time()
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=500, do_sample=False,
                             eos_token_id=eos, pad_token_id=eos)

    reply = proc.batch_decode(out[:, inputs["input_ids"].shape[1]:],
                              skip_special_tokens=True)[0]
    # 双保险：砍掉可能残留的自问自答
    for marker in ("Human:", "\nUser:", "assistant\n"):
        reply = reply.split(marker)[0]
    reply = reply.strip()

    print("\n" + "=" * 60)
    print(reply)
    print("=" * 60)
    print(f"✅ 推理 {time.time()-t1:.0f}s")
    print("\n⚠️ 以上只是声音的感知描述，不是诊断。")
    print("   把它连同频谱图一起交给 Claude，并补充工况信息:")
    print("   冷车还是热车 / 什么转速 / 什么挡位 / 是否随车速变化 / 什么时候开始出现")


if __name__ == "__main__":
    main()
