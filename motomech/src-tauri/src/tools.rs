//! 外部工具层 —— 声明式注册，结果带来源级别
//!
//! 设计的要害在于**不撒谎**：
//!
//! 1. `link` 类工具（RealOEM 这种图形 fiche）机器调不了，就明说调不了、
//!    只给 URL。把它伪装成能调的接口，调用方会以为自己拿到了数据，
//!    实际上什么也没查 —— 那比不提供这个工具糟得多。
//!
//! 2. 每个工具声明自己返回的东西算几级。官方零件库是 A，论坛是 C。
//!    「接入任何 API」因此不会污染准确性机制。
//!
//! 3. 只能调注册表里声明过的 URL 模板，参数做百分号编码后代入。
//!    没有「传一个任意 URL 进来」的入口 —— 那会变成 SSRF。

use crate::platform::Platform;
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct ToolDecl {
    pub id: String,
    /// 显式关闭的工具不可调用（默认开启）
    pub enabled: bool,
    pub kind: String,          // http_json | link
    pub source_level: String,  // A/B/C/D
    pub desc: String,
    pub url: String,
    pub params: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    /// 空结果守卫：查到 0 条时，这里说明「0」到底是「没有」还是「没查到」。
    /// 沉默的 0 是这个项目命名的最坏结果之一，所以它不许沉默。
    pub guard: Option<String>,
    pub id: String,
    pub kind: String,
    /// 这份结果算几级 —— 由工具声明决定，调用方不能改
    pub source_level: String,
    pub url: String,
    /// http_json 才有
    pub data: Option<serde_json::Value>,
    /// link 类：机器没调，这里说明为什么
    pub manual_note: Option<String>,
    pub note: Option<String>,
}

pub fn load(p: &Platform) -> Result<Vec<ToolDecl>> {
    let f = p.root.join("tools.yml");
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(&f).with_context(|| format!("读不到 {}", f.display()))?)?;
    let empty = Vec::new();
    Ok(doc.get("tools").and_then(|x| x.as_sequence()).unwrap_or(&empty).iter().filter_map(|t| {
        let g = |k: &str| t.get(k).and_then(|x| x.as_str()).map(String::from);
        Some(ToolDecl {
            id: g("id")?, kind: g("kind")?,
            enabled: t.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true),
            source_level: g("source_level").unwrap_or_else(|| "D".into()),
            desc: g("desc").unwrap_or_default(), url: g("url")?,
            params: t.get("params").and_then(|x| x.as_sequence())
                .map(|l| l.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            note: g("note").map(|s| s.trim().to_string()),
        })
    }).collect())
}

/// 百分号编码 —— 手写是因为不想为几行代码拉一个依赖。
/// 未保留字符表照 RFC 3986。
fn enc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' =>
                o.push(*b as char),
            b' ' => o.push_str("%20"),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

pub fn call(p: &Platform, id: &str, args: &std::collections::HashMap<String, String>)
    -> Result<ToolResult>
{
    let decls = load(p)?;
    let d = decls.iter().find(|t| t.id == id)
        .with_context(|| format!("没有这个工具：{id}。已注册的：{}",
            decls.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(" ")))?;

    if !d.enabled {
        anyhow::bail!(
            "工具 {id} 已在 tools.yml 里显式关闭（enabled: false）。\
             多半是它需要密钥 —— 看那条的 note 说明怎么启用。\
             **不要把密钥写进文件**，走 macOS 钥匙串。");
    }
    let missing: Vec<&String> = d.params.iter().filter(|k| !args.contains_key(*k)).collect();
    if !missing.is_empty() {
        anyhow::bail!("缺少参数：{}。本工具需要：{}",
            missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "),
            d.params.join(" "));
    }

    // 只替换声明过的参数名，其余原样保留 —— 注册表之外的东西进不来
    let mut url = d.url.clone();
    for k in &d.params {
        url = url.replace(&format!("{{{k}}}"), &enc(args.get(k).unwrap()));
    }

    if d.kind == "link" {
        return Ok(ToolResult {
            id: d.id.clone(), kind: d.kind.clone(), source_level: d.source_level.clone(),
            url,
            data: None, guard: None,
            manual_note: Some(
                "这个来源没有公开 API，机器调不了 —— 上面的链接需要人去打开。\
                 从页面上誊下来的数据仍按本工具声明的级别记录，但誊错一位就是错数据，\
                 誊完请回页面核对一次。".into()),
            note: d.note.clone(),
        });
    }

    let body = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("motomech/0.1")
        .build()?
        .get(&url).send()?.text()?;
    let data: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("返回的不是 JSON（前 200 字符）：{}",
            body.chars().take(200).collect::<String>()))?;

    let guard = empty_guard(&d.id, args, &data);

    Ok(ToolResult {
        id: d.id.clone(), kind: d.kind.clone(), source_level: d.source_level.clone(),
        url, data: Some(data), manual_note: None, note: d.note.clone(), guard,
    })
}

/// NHTSA 官方车型词表 —— 「这台车在 NHTSA 里叫什么」。
fn nhtsa_vocabulary(make: &str, year: &str) -> Result<Vec<String>> {
    let url = format!(
        "https://vpic.nhtsa.dot.gov/api/vehicles/GetModelsForMakeYear/make/{}/modelyear/{}?format=json",
        enc(make), enc(year));
    let body = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("motomech/0.1")
        .build()?.get(&url).send()?.text()?;
    let v: serde_json::Value = serde_json::from_str(&body)?;
    Ok(v.get("Results").and_then(|x| x.as_array()).map(|a| a.iter()
        .filter_map(|m| m.get("Model_Name").and_then(|x| x.as_str()).map(String::from))
        .collect()).unwrap_or_default())
}

/// 查到 0 条时，回答「这个 0 是什么意思」。
///
/// 起因（2026-09-16 实测）：`nhtsa.recalls make=ducati model=GT1000 year=2008`
/// 返回 `{"Count":0,"results":[]}`，一个字都没说 GT1000 根本不在 NHTSA 词表里
/// （它登记在「Classic」名下）。调用方是模型，模型读到这个只会得出「没有召回」。
///
/// **fail-closed**：词表本身查不动时，绝不说「没有」，而是说「这个 0 不作数」。
fn empty_guard(id: &str, args: &std::collections::HashMap<String, String>,
                data: &serde_json::Value) -> Option<String> {
    if id != "nhtsa.recalls" && id != "nhtsa.complaints" { return None; }
    let n = data.get("results").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
    if n > 0 { return None; }

    let (make, model, year) = (args.get("make")?, args.get("model")?, args.get("year")?);
    let vocab = match nhtsa_vocabulary(make, year) {
        Ok(v) if !v.is_empty() => v,
        _ => return Some(format!(
            "🔴 **这个 0 不作数。** 查到 0 条，而车型词表此刻也核不动 —— \
             无法区分「{make} {model} {year} 真没有」与「这个名字 NHTSA 不认」。\
             fail-closed：不要把它当成「没有」。稍后重跑 `tool nhtsa.models make={make} year={year}`。")),
    };
    let norm = |x: &str| x.to_lowercase().replace([' ', '-', '_'], "");
    let m = norm(model);
    let hit = vocab.iter().any(|v| { let v = norm(v); v == m || v.contains(&m) || m.contains(&v) });

    if hit {
        Some(format!("✅ 「{model}」在 NHTSA {year} 年 {make} 的词表里（共 {} 个名字），\
                      所以这 0 条是**真的查过了**，不是没查到。", vocab.len()))
    } else {
        Some(format!(
            "🔴 **0 条 ≠ 没有 —— 「{model}」根本不在 NHTSA 的词表里。**\n\
             {make} {year} 在 NHTSA 登记的全部 {} 个名字是：{}。\n\
             这台车多半登记在其中某个名字下（俗名与登记名常常对不上，\
             例如 Ducati GT1000 登记为「Classic」）。\n\
             **在用对名字重查之前，不得声称这台车没有召回/投诉。**",
            vocab.len(), vocab.join(" / ")))
    }
}

pub fn render(r: &ToolResult) -> String {
    let mut o = format!("[{}级] {}\n{}\n", r.source_level, r.id, r.url);
    if let Some(g) = &r.guard { o.push_str(&format!("\n{g}\n")); }
    if let Some(n) = &r.manual_note { o.push_str(&format!("\n⚠️ {n}\n")); }
    if let Some(n) = &r.note { o.push_str(&format!("\n注意：{n}\n")); }
    if let Some(d) = &r.data {
        let s = serde_json::to_string_pretty(d).unwrap_or_default();
        o.push_str(&format!("\n{}\n", s.chars().take(4000).collect::<String>()));
        if s.chars().count() > 4000 { o.push_str("…（截断）\n"); }
    }
    o
}

// ── 本地脚本（阶段 9）────────────────────────────
//
// 计划第七节定过：现有 shell/python 工具**保留**，不重写 —— 它们已经过实战验证
// （Haynes 的 OCR 就是 bookproc.sh 跑出来的）。App 负责让它们可被发现。
//
// **只列出、不代为执行。** 从 MCP 里 shell out 会开一个很大的攻击面：
// 调用方是模型，而模型会读到不受信的内容（手册文本、论坛、照片里的字）。
// 让人自己敲那一行，成本是几秒钟，换掉的是一整类风险。

#[derive(Debug, Serialize)]
pub struct Script {
    pub name: String,
    pub desc: String,
    pub cmd: String,
    pub runnable: bool,
}

pub fn scripts(p: &Platform) -> Result<Vec<Script>> {
    let dir = p.root.join("tools");
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("读不到 {}", dir.display()))?
        .flatten().map(|e| e.path()).collect();
    names.sort();
    for pth in names {
        let name = pth.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.starts_with('_') || name.starts_with('.') { continue; }
        let ext = pth.extension().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        if ext != "sh" && ext != "py" { continue; }
        // 描述取文件头部的第一句说明。
        // shell 是 `# 名字 — 一句话`；python 的说明在 """ 换行之后，
        // 所以不能只看「这一行里有没有破折号」—— 第一版就漏掉了全部 python 脚本。
        let desc = std::fs::read_to_string(&pth).ok()
            .and_then(|s| s.lines().take(8)
                .map(|l| l.trim_start_matches(['#', '"', ' ']).trim())
                .find(|l| !l.is_empty()
                       && !l.starts_with("!/")           // shebang
                       && !l.starts_with("-*-")
                       && (l.contains('—') || l.contains("--") || l.contains(".py")
                           || l.contains(".sh")))
                .map(|l| l.to_string()))
            .unwrap_or_default();
        let runnable = std::fs::metadata(&pth).map(|m| m.len() > 0).unwrap_or(false);
        out.push(Script {
            cmd: format!("{} tools/{}",
                if ext == "sh" { "bash" } else { "python3" }, name),
            name, desc, runnable,
        });
    }
    Ok(out)
}
