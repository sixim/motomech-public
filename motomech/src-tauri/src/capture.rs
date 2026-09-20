//! 手机直采 —— 用**手机自己的**摄像头和麦克风，经 Tailscale 直连回本机
//!
//! ## 为什么要这条路
//!
//! 1. **Mac 摄像头权限绕不过去**：2026-09-16 实测，设备能枚举（连 iPhone 连续互通
//!    都列得出来），抓帧却报 `Camera access not granted`。而手机浏览器里拍照，
//!    权限是 Safari 自己管的，跟 Mac 的终端授权毫无关系。
//! 2. **录音从来就没实现过**：`listen.sh` 是分析管线，输入是已有文件，它不采集。
//!    两台车的音频素材是 0 段 —— 双通道只用合成音验证过，没见过真实异响。
//! 3. 修车时人在车边，Mac 在工作台上。手机是唯一真正拿在手里的设备。
//!
//! ## 安全边界（三层，缺一不可）
//!
//! * **进程只绑 127.0.0.1**（`serve` 的默认值，不要改）。对外由
//!   `tailscale serve --bg 8765` 代理 —— 于是访问控制完全是 Tailscale 的 ACL，
//!   本程序不自己发明认证。
//! * **HTTPS 由 Tailscale 终结**（`*.ts.net` 是真 Let's Encrypt 证书）。
//!   这不只是好看：iOS Safari 的 `getUserMedia`（浏览器内录音）**要求安全上下文**，
//!   纯 HTTP 下直接不给用。所以走 `tailscale serve` 不是可选项，是录音能不能用的前提。
//! * **来源 IP 仍然要查**：只接受回环（Tailscale 代理就是从回环来的）与
//!   100.64.0.0/10（Tailscale 的 CGNAT 段）。公网地址一律拒。纵深防御，
//!   万一有人把 `--host` 改成 0.0.0.0 也不至于门户大开。
//!
//! ## 文件名一个字都不信
//!
//! 上传方是手机浏览器，文件名是它给的 —— 可能带路径、可能带 `..`、可能是任意后缀。
//! 所以**扩展名走白名单、文件名由本程序生成**，沿用 snap.sh 的命名约定
//! （`YYYYMMDD_阶段_标签_序号.ext`），让手机拍的和翻拍台拍的在同一个命名空间里。

use crate::platform::Platform;
use anyhow::{bail, Result};
use std::net::IpAddr;
use std::path::PathBuf;

/// 图片白名单 —— 与索引器认的扩展名一致（index.rs 只 walk 这几种）
const IMG: &[&str] = &["jpg", "jpeg", "png", "heic"];
/// 音频白名单 —— ffmpeg 能读、iOS 能录的那几种
const AUD: &[&str] = &["m4a", "mp3", "wav", "aac", "caf", "webm", "ogg"];

/// 来源是否可信：回环 或 Tailscale 的 100.64.0.0/10。
pub fn is_trusted(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() { return true; }
            let o = v4.octets();
            // 100.64.0.0/10 —— Tailscale 用的 CGNAT 段
            o[0] == 100 && (64..128).contains(&o[1])
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// 清洗标签：只留字母数字、CJK、连字符与下划线，截断，空则给默认值。
/// 它会进文件名，所以一个可疑字符都不能放过。
fn clean_tag(t: &str) -> String {
    let s: String = t.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(32).collect();
    if s.is_empty() { "phone".into() } else { s }
}

/// 阶段必须形如 P0–P9，否则退回 P0。不让任意字符串进路径。
fn clean_phase(p: &str) -> String {
    let u = p.trim().to_uppercase();
    let ok = u.len() == 2 && u.starts_with('P') && u.as_bytes()[1].is_ascii_digit();
    if ok { u } else { "P0".into() }
}

/// 从上传的原始文件名里**只**取扩展名，且必须在白名单里。
fn ext_of(filename: &str, mime: &str) -> Option<(String, bool)> {
    let raw = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    let raw: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).take(5).collect();
    if IMG.contains(&raw.as_str()) { return Some((raw, true)); }
    if AUD.contains(&raw.as_str()) { return Some((raw, false)); }
    // 浏览器内录音常常没有扩展名（Blob），退回用 MIME 判
    if mime.starts_with("audio/") {
        let e = if mime.contains("mp4") || mime.contains("m4a") { "m4a" }
                else if mime.contains("webm") { "webm" }
                else if mime.contains("wav") { "wav" } else { "m4a" };
        return Some((e.into(), false));
    }
    if mime.starts_with("image/") {
        let e = if mime.contains("png") { "png" }
                else if mime.contains("heic") { "heic" } else { "jpg" };
        return Some((e.into(), true));
    }
    None
}

pub struct Saved { pub path: PathBuf, pub is_image: bool, pub bytes: usize }

/// 落盘。返回写到哪、是图还是音。
pub fn save(p: &Platform, vehicle: Option<&str>, phase: &str, tag: &str,
            filename: &str, mime: &str, data: &[u8]) -> Result<Saved>
{
    if data.is_empty() { bail!("上传的内容是空的 —— 没写入任何文件"); }
    // resolve_vehicle 已被加固（车辆 id 必须是单段目录名且在枚举列表内），
    // 所以这里拿到的 dir 一定在平台内部，不会被 ../ 带出去。
    let v = p.resolve_vehicle(vehicle)?;
    let (ext, is_image) = ext_of(filename, mime)
        .ok_or_else(|| anyhow::anyhow!(
            "不接受这种文件（名「{filename}」类型「{mime}」）。\
             图片只收 {IMG:?}，音频只收 {AUD:?}"))?;

    // 图片进既有的照片 inbox（索引器就 walk 那里）；
    // 音频另开一个目录 —— 索引器按扩展名过滤，放这里不会被误算成照片。
    let dir = if is_image { v.photos_dir().join("inbox") }
              else { v.photos_dir().join("音频") };
    std::fs::create_dir_all(&dir)?;

    let (ph, tg) = (clean_phase(phase), clean_tag(tag));
    let date = chrono::Local::now().format("%Y%m%d").to_string();
    // 沿用 snap.sh 的命名，手机拍的和翻拍台拍的落在同一命名空间
    let mut n = 1;
    let path = loop {
        let c = dir.join(format!("{date}_{ph}_{tg}_phone_{n:02}.{ext}"));
        if !c.exists() { break c; }
        n += 1;
        if n > 999 { bail!("同名文件太多（>999），先清一下 {}", dir.display()); }
    };
    std::fs::write(&path, data)?;
    Ok(Saved { path, is_image, bytes: data.len() })
}

/// 手机上打开的采集页。故意做成单文件无依赖 —— 车库里网不一定好。
pub fn page(vehicles: &[String], current: &str) -> String {
    let opts: String = vehicles.iter().map(|v| format!(
        "<option value=\"{v}\"{}>{v}</option>",
        if v == current { " selected" } else { "" })).collect();
    format!(r##"<!doctype html><html lang="zh"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>采集 · motomech</title><style>
:root{{--bg:#010309;--card:#0a1330;--line:#3d6bff33;--blue:#4a7dff;--bright:#6fa0ff;--txt:#eaf1ff;--dim:#7f92c8}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--bg);color:var(--txt);font:15px/1.5 -apple-system,system-ui,sans-serif;
padding:16px calc(16px + env(safe-area-inset-right)) calc(24px + env(safe-area-inset-bottom)) calc(16px + env(safe-area-inset-left))}}
h1{{font-size:17px;margin:0 0 4px}} .sub{{color:var(--dim);font-size:12px;margin-bottom:16px}}
label{{display:block;font-size:12px;color:var(--bright);margin:12px 0 4px}}
select,input[type=text]{{width:100%;padding:11px;background:var(--card);color:var(--txt);
border:1px solid var(--line);border-radius:9px;font-size:16px}}
.btn{{display:block;width:100%;padding:17px;margin-top:12px;border-radius:12px;border:1px solid var(--line);
background:var(--card);color:var(--bright);font-size:16px;font-weight:600;text-align:center;cursor:pointer}}
.btn:active{{background:#12224d}} .btn.rec{{color:#ff9aa8;border-color:#ff5a6e55}}
input[type=file]{{display:none}}
#log{{margin-top:18px;font-size:12px}} .ok{{color:#7effc0}} .err{{color:#ff9aa8}}
.row{{padding:7px 9px;background:var(--card);border:1px solid var(--line);border-radius:8px;margin-bottom:6px;
word-break:break-all}}
.note{{margin-top:20px;padding:10px;border:1px solid var(--line);border-radius:9px;color:var(--dim);font-size:11px}}
</style></head><body>
<h1>🏍 现场采集</h1>
<div class="sub">用手机自己的摄像头/麦克风，经 Tailscale 直接存进车辆档案</div>

<label>车辆</label><select id="veh">{opts}</select>
<label>阶段</label><select id="ph">
{phases}
</select>
<label>标签（进文件名，便于以后检索）</label>
<input type="text" id="tag" placeholder="如 frontbrake / finaldrive" value="">

<button class="btn" onclick="cam.click()">📷 拍照</button>
<input type="file" id="cam" accept="image/*" capture="environment" multiple>

<button class="btn" id="recBtn">🎙 录异响（按一次开始，再按停止）</button>

<button class="btn" onclick="pick.click()">📁 选已有文件（语音备忘录/相册）</button>
<input type="file" id="pick" accept="image/*,audio/*" multiple>

<div id="log"></div>
<div class="note">录音需要 https —— 若这里打不开麦克风，说明你是用 http 进来的，
请改用 <b>tailscale serve</b> 给出的 https 地址。图片上传不受影响。</div>

<script>
const $=id=>document.getElementById(id), log=$('log');
function say(t,ok){{const d=document.createElement('div');d.className='row '+(ok?'ok':'err');d.textContent=t;log.prepend(d);}}
async function up(file,name){{
  const fd=new FormData();
  fd.append('vehicle',$('veh').value); fd.append('phase',$('ph').value); fd.append('tag',$('tag').value);
  fd.append('file',file,name||file.name||'capture');
  say('上传中… '+(name||file.name||'capture'),true);
  try{{
    const r=await fetch('/v1/capture',{{method:'POST',body:fd}});
    const j=await r.json();
    if(j.error) say('❌ '+j.error,false); else say('✅ '+j.saved+'  ('+j.kb+' KB)',true);
  }}catch(e){{ say('❌ 上传失败：'+e,false); }}
}}
for(const id of ['cam','pick']) $(id).onchange=e=>{{ for(const f of e.target.files) up(f); e.target.value=''; }};

let mr=null,chunks=[];
$('recBtn').onclick=async()=>{{
  if(mr&&mr.state==='recording'){{ mr.stop(); return; }}
  if(!navigator.mediaDevices||!navigator.mediaDevices.getUserMedia){{
    say('❌ 这个浏览器不给录音（多半是走了 http，不是安全上下文）',false); return; }}
  try{{
    const s=await navigator.mediaDevices.getUserMedia({{audio:true}});
    mr=new MediaRecorder(s); chunks=[];
    mr.ondataavailable=e=>chunks.push(e.data);
    mr.onstop=()=>{{ s.getTracks().forEach(t=>t.stop());
      const b=new Blob(chunks,{{type:mr.mimeType||'audio/mp4'}});
      up(b,'rec.'+((mr.mimeType||'').includes('webm')?'webm':'m4a'));
      $('recBtn').textContent='🎙 录异响（按一次开始，再按停止）'; $('recBtn').classList.remove('rec'); }};
    mr.start(); $('recBtn').textContent='⏹ 录音中… 点此停止'; $('recBtn').classList.add('rec');
  }}catch(e){{ say('❌ 打不开麦克风：'+e,false); }}
}};
</script></body></html>"##,
        opts = opts,
        phases = (0..=9).map(|i| format!("<option value=\"P{i}\">P{i}</option>")).collect::<String>())
}
