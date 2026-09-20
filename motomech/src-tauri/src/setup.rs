//! setup.rs —— 非开发者的入门向导（serve 模式下的 `GET /setup`）
//!
//! 三步，全在浏览器里完成，用户不碰终端、不碰路径：
//!   ① 输入 VIN → 后端跑 coldstart → 建好 VEHICLE.yml
//!   ② 拖入手册 PDF（可选）→ 存进该车的 raw/，等 bookproc 处理
//!   ③ 完成，给出接入 AI 助手的下一步
//!
//! 故意做成单文件、零外部依赖：和 capture 页一样，车库里网不一定好。

use crate::platform::Platform;
use anyhow::{bail, Result};

/// 把用户的文件名清洗成一个安全的手册目录名（单段、无路径分隔、无点开头）。
fn clean_manual_name(filename: &str) -> String {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "manual".into());
    let cleaned: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '_', '.']).to_string();
    if trimmed.is_empty() { "manual".into() } else { trimmed }
}

/// 保存上传的手册 PDF 到 `<车辆>/10_手册资料/third-party/<手册名>/raw/`。
/// bookproc.sh 正是从这个 raw/ 目录读取。只接受 PDF —— 图片走 /v1/capture。
pub fn save_manual(p: &Platform, vehicle: Option<&str>, filename: &str, data: &[u8])
    -> Result<std::path::PathBuf>
{
    if data.is_empty() { bail!("上传的内容是空的 —— 没写入任何文件"); }
    // 简单魔数校验：PDF 以 "%PDF" 开头。挡住误传的非 PDF。
    if data.len() < 5 || &data[0..4] != b"%PDF" {
        bail!("这不是 PDF 文件（文件头不是 %PDF）。手册请传 PDF；照片请用采集页 /capture");
    }
    // resolve_vehicle 已加固（id 必须是单段目录名且在枚举列表内），dir 不会被 ../ 带出平台。
    let v = p.resolve_vehicle(vehicle)?;
    let name = clean_manual_name(filename);
    let raw = v.manuals_dir().join("third-party").join(&name).join("raw");
    std::fs::create_dir_all(&raw)?;
    let dest = raw.join(format!("{name}.pdf"));
    std::fs::write(&dest, data)?;
    Ok(dest)
}

/// 入门向导页面。故意单文件无依赖。
pub fn page() -> &'static str {
    PAGE
}

const PAGE: &str = r##"<!doctype html><html lang="zh"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>开始使用 · motomech</title><style>
:root{--bg:#010309;--card:#0a1330;--line:#3d6bff33;--blue:#4a7dff;--bright:#6fa0ff;--txt:#eaf1ff;--dim:#7f92c8;--ok:#7effc0;--err:#ff9aa8}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--txt);font:15px/1.6 -apple-system,system-ui,sans-serif;
padding:20px calc(16px + env(safe-area-inset-right)) calc(28px + env(safe-area-inset-bottom)) calc(16px + env(safe-area-inset-left));
max-width:560px;margin:0 auto}
h1{font-size:20px;margin:0 0 4px} .sub{color:var(--dim);font-size:13px;margin-bottom:24px}
.step{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:18px;margin-bottom:16px;
opacity:.45;transition:opacity .3s} .step.active{opacity:1} .step.done{opacity:.7}
.step h2{font-size:15px;margin:0 0 4px;color:var(--bright)} .step .desc{color:var(--dim);font-size:12px;margin-bottom:12px}
.num{display:inline-block;width:22px;height:22px;line-height:22px;text-align:center;border-radius:50%;
background:var(--blue);color:#fff;font-size:12px;font-weight:700;margin-right:8px}
label{display:block;font-size:12px;color:var(--bright);margin:10px 0 4px}
input[type=text]{width:100%;padding:12px;background:#050b1e;color:var(--txt);border:1px solid var(--line);
border-radius:9px;font-size:16px;letter-spacing:1px}
.btn{display:block;width:100%;padding:15px;margin-top:12px;border-radius:11px;border:1px solid var(--line);
background:#12224d;color:var(--bright);font-size:15px;font-weight:600;text-align:center;cursor:pointer}
.btn:active{background:#1a2f66} .btn:disabled{opacity:.4;cursor:not-allowed}
.drop{margin-top:12px;padding:28px 16px;border:2px dashed var(--line);border-radius:12px;text-align:center;
color:var(--dim);font-size:13px;cursor:pointer} .drop.over{border-color:var(--blue);background:#0d1836;color:var(--bright)}
input[type=file]{display:none}
.card-veh{margin-top:12px;padding:12px;background:#050b1e;border:1px solid var(--line);border-radius:9px;font-size:13px}
.card-veh b{color:var(--bright)} .card-veh .k{color:var(--dim);display:inline-block;width:64px}
#log{font-size:12px} .row{padding:8px 10px;border-radius:8px;margin-top:6px;word-break:break-all}
.row.ok{background:#0a2318;color:var(--ok)} .row.err{background:#2a0e12;color:var(--err)}
.spin{display:inline-block;width:14px;height:14px;border:2px solid var(--line);border-top-color:var(--bright);
border-radius:50%;animation:sp .7s linear infinite;vertical-align:-2px;margin-right:6px}
@keyframes sp{to{transform:rotate(360deg)}}
.note{margin-top:8px;color:var(--dim);font-size:11px} code{background:#050b1e;padding:1px 5px;border-radius:4px}
.skip{color:var(--dim);font-size:12px;text-align:center;margin-top:8px;cursor:pointer;text-decoration:underline}
</style></head><body>
<h1>🏍 开始使用 motomech</h1>
<div class="sub">三步搞定，全程不用碰命令行。</div>

<div class="step active" id="s1">
  <h2><span class="num">1</span>你的车是什么？</h2>
  <div class="desc">输入 VIN（车架号，17 位），我们从 NHTSA 官方数据库自动识别车型。</div>
  <label>VIN</label>
  <input type="text" id="vin" placeholder="如 WB10..." maxlength="17" autocapitalize="characters" autocomplete="off">
  <button class="btn" id="vinBtn">识别车型</button>
  <div id="vinResult"></div>
</div>

<div class="step" id="s2">
  <h2><span class="num">2</span>上传你的手册（可选）</h2>
  <div class="desc">有服务手册 PDF 就拖进来 —— 它只存在你自己的电脑上，永远不上传到网上。
    没有也能用：车型规格已经可以查。</div>
  <div class="drop" id="drop">📄 把 PDF 拖到这里，或点击选择文件</div>
  <input type="file" id="pdf" accept="application/pdf">
  <div id="upLog"></div>
  <div class="skip" id="skipUp">跳过这一步 →</div>
</div>

<div class="step" id="s3">
  <h2><span class="num">3</span>完成！接入你的 AI 助手</h2>
  <div class="desc">在 Claude Desktop / Cursor 的 MCP 配置里加入 motomech，就能让 AI 查你这台车的精确规格：</div>
  <div class="card-veh"><code>motomech mcp</code></div>
  <div class="note">手册 PDF 已存好，接下来在终端跑一次
    <code>tools/bookproc.sh</code> 就能建立全文索引（这一步暂时还需要命令行，正在做成自动）。</div>
</div>

<script>
const $=id=>document.getElementById(id);
let vehicleId='';
function activate(n){for(let i=1;i<=3;i++){const s=$('s'+i);s.classList.remove('active');if(i<n)s.classList.add('done');}$('s'+n).classList.add('active');$('s'+n).scrollIntoView({behavior:'smooth',block:'nearest'});}
function say(host,t,ok){const d=document.createElement('div');d.className='row '+(ok?'ok':'err');d.textContent=t;host.prepend(d);}

$('vinBtn').onclick=async()=>{
  const vin=$('vin').value.trim().toUpperCase();
  const host=$('vinResult');
  if(vin.length<11){host.innerHTML='<div class="row err">VIN 太短，应为 17 位</div>';return;}
  $('vinBtn').disabled=true; host.innerHTML='<div class="row"><span class="spin"></span>查询 NHTSA…</div>';
  try{
    const r=await fetch('/v1/coldstart',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({vin})});
    const j=await r.json();
    if(j.error){host.innerHTML='<div class="row err">❌ '+j.error+'</div>';$('vinBtn').disabled=false;return;}
    vehicleId=j.vehicle_id||'';
    host.innerHTML='<div class="card-veh">✅ <b>'+(j.make||'')+' '+(j.model||'')+' '+(j.year||'')+'</b>'
      +(j.cylinders?'<br><span class="k">气缸</span>'+j.cylinders+' 缸':'')
      +'<br><span class="k">档案</span>'+vehicleId+'</div>';
    $('vinBtn').textContent='重新识别'; $('vinBtn').disabled=false;
    activate(2);
  }catch(e){host.innerHTML='<div class="row err">❌ 请求失败：'+e+'</div>';$('vinBtn').disabled=false;}
};

const drop=$('drop'),pdf=$('pdf'),upLog=$('upLog');
drop.onclick=()=>pdf.click();
['dragenter','dragover'].forEach(ev=>drop.addEventListener(ev,e=>{e.preventDefault();drop.classList.add('over');}));
['dragleave','drop'].forEach(ev=>drop.addEventListener(ev,e=>{e.preventDefault();drop.classList.remove('over');}));
drop.addEventListener('drop',e=>{if(e.dataTransfer.files.length)upload(e.dataTransfer.files[0]);});
pdf.onchange=e=>{if(e.target.files.length)upload(e.target.files[0]);};
$('skipUp').onclick=()=>activate(3);

async function upload(file){
  if(!file.name.toLowerCase().endsWith('.pdf')){say(upLog,'只接受 PDF 文件',false);return;}
  const fd=new FormData(); fd.append('vehicle',vehicleId); fd.append('file',file,file.name);
  say(upLog,'上传中… '+file.name,true);
  try{
    const r=await fetch('/v1/manual/upload',{method:'POST',body:fd});
    const j=await r.json();
    if(j.error){say(upLog,'❌ '+j.error,false);}
    else{say(upLog,'✅ 已保存：'+j.saved+' ('+j.mb+' MB)',true);activate(3);}
  }catch(e){say(upLog,'❌ 上传失败：'+e,false);}
}
</script></body></html>"##;
