//! 新车冷启动 —— 给一个 VIN，把「这台车是什么、会怎么坏」先问出来
//!
//! 这是「自己找需求」的上游：差距引擎能说清缺什么，冷启动负责把
//! 能自动拿到的那部分一次性拿回来，剩下的原样交还给人。
//!
//! 三条纪律（四轮监工换来的，一条都不能松）：
//!
//! 1. **不自我认证。** 生成的档案里，NHTSA 官方接口返回的算 A 级；
//!    推断出来的（发动机代号、适用性排除项）一律留空并标为待核实。
//!    宁可空着让差距引擎报出来，也不要填一个看起来像样的猜测。
//!
//! 2. **不覆盖已有档案。** 新车直接建目录（没有东西可毁）；
//!    已有车辆只产出**提案文件**，由人过目后合并。
//!
//! 3. **不猜车型名，去问词表。** NHTSA 对同一台车有多种写法
//!    （2009 BMW 同时存在 "R 1200 GS" 和 "R1200 GS"，
//!    2014 Harley 的 XL1200 叫 "XL SPORTSTER" 或 "XL1200C"）。
//!    猜四种写法全返回 0 条 —— 实测过。

use crate::{platform::Platform, tools};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize, Clone)]
pub struct ModelMatch { pub name: String, pub confidence: String, pub reason: String }

#[derive(Debug, Serialize)]
pub struct RecallBrief {
    pub id: String, pub component: String,
    pub summary: String, pub consequence: String, pub from_model: String,
}

#[derive(Debug, Serialize)]
pub struct IssueBrief {
    pub components: String, pub count: usize, pub quote: String,
}

#[derive(Debug, Serialize)]
pub struct ColdStart {
    pub vin: Option<String>,
    pub identity: HashMap<String, String>,
    pub vehicle_id: String,
    pub model_matches: Vec<ModelMatch>,
    pub recalls: Vec<RecallBrief>,
    /// 从低置信车型名查到、但原文核对后**不适用本车**的（只作参考，不进档案）
    pub rejected_recalls: Vec<RecallBrief>,
    pub issues: Vec<IssueBrief>,
    pub complaint_total: usize,
    /// 新车：写到哪个目录；已有车：写到哪个提案文件
    pub written_to: String,
    pub is_proposal: bool,
    pub notes: Vec<String>,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// 从 NHTSA 的车型词表里找出与解码结果对应的写法。
/// **只有归一化后完全相同才算高置信**；其余标低置信并如实列出，由人确认。
pub fn resolve_models(make: &str, year: i64, decoded: &str) -> Result<Vec<ModelMatch>> {
    let url = format!(
        "https://api.nhtsa.gov/products/vehicle/models?modelYear={year}&make={}&issueType=r",
        make.replace(' ', "%20"));
    let body = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("motomech/0.1").build()?
        .get(&url).send()?.text()?;
    let doc: serde_json::Value = serde_json::from_str(&body)
        .context("车型词表返回的不是 JSON")?;
    let d = norm(decoded);
    let dtok: Vec<String> = decoded.split_whitespace().map(norm).filter(|t| t.len() >= 4).collect();

    let mut out = Vec::new();
    for m in doc.get("results").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        let name = match m.get("model").and_then(|x| x.as_str()) { Some(s) => s, None => continue };
        let n = norm(name);
        if n == d {
            out.push(ModelMatch { name: name.into(), confidence: "high".into(),
                                  reason: "归一化后与解码车型完全一致".into() });
        } else if n.contains(&d) || d.contains(&n) {
            out.push(ModelMatch { name: name.into(), confidence: "low".into(),
                reason: "包含关系 —— **可能是别的变体**（如 Adventure），用之前请确认".into() });
        } else if dtok.iter().any(|t| n.contains(t.as_str())) {
            out.push(ModelMatch { name: name.into(), confidence: "low".into(),
                reason: "共享词".into() });
        } else if common_prefix(&n, &d) >= 5 {
            out.push(ModelMatch { name: name.into(), confidence: "low".into(),
                reason: format!("共同前缀 {} 位", common_prefix(&n, &d)) });
        }
    }
    Ok(out)
}

fn fetch(url: &str) -> Result<serde_json::Value> {
    let body = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .user_agent("motomech/0.1").build()?
        .get(url).send()?.text()?;
    Ok(serde_json::from_str(&body)?)
}

fn q(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "%20".into(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

/// 用已有车辆的 make/model/year 做研究刷新（没有 VIN 时的入口）。
///
/// 差距引擎会对没查过召回的车说「app 可自己关闭」并给出 tool 命令 ——
/// 但那条命令在车型名对不上时查不到（哈雷的 model 写作
/// "XL1200 (Sportster)"，NHTSA 叫它 "XL SPORTSTER" / "XL1200C"）。
/// 词表解析正是为此存在，所以这条路也要能走。
pub fn from_vehicle(p: &Platform, vehicle: Option<&str>, apply: bool) -> Result<ColdStart> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let vin = doc.get("vin").and_then(|x| x.as_str()).map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "~");
    if let Some(vin) = vin {
        return run(p, &vin, apply);
    }
    let get = |k: &str| doc.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let (make, model) = (get("make"), get("model"));
    let year = doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0);
    if make.is_empty() || year == 0 {
        anyhow::bail!("{} 既没有 VIN，也没有完整的 make/year，无法做研究刷新", v.id);
    }
    // 括号里的俗名（"XL1200 (Sportster)"）对 API 是噪声，但对词表匹配是线索 —— 两种都试
    let plain = model.split('(').next().unwrap_or(&model).trim().to_string();
    let inner: String = model.split('(').nth(1).unwrap_or("")
        .trim_end_matches(')').trim().to_string();
    let probe = if inner.is_empty() { plain.clone() } else { format!("{plain} {inner}") };
    research_keep(p, &v.id, &make, &probe, year, apply,
             vec![format!("没有 VIN，改用档案里的 {make} / {model} / {year} 做研究刷新。\
                           **身份未经 VIN 校验** —— 车型名对不上就查不到，请看下面的词表匹配")])
}

pub fn run(p: &Platform, vin: &str, apply: bool) -> Result<ColdStart> {
    let mut notes: Vec<String> = Vec::new();

    // ── 1. 解码 VIN（A 级）──
    let dec = tools::call(p, "nhtsa.vin_decode",
        &HashMap::from([("vin".to_string(), vin.to_string())]))?;
    let d = dec.data.clone().unwrap_or_default();
    let r0 = d.get("Results").and_then(|x| x.as_array())
        .and_then(|a| a.first()).cloned()
        .context("VIN 解码没有返回结果")?;
    let g = |k: &str| r0.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();

    let err = g("ErrorText");
    if !err.starts_with("0 ") {
        notes.push(format!("⚠️ VIN 解码有告警：{err} —— **校验位没过就不能信这个身份**"));
    }
    let (make, model, year_s) = (g("Make"), g("Model"), g("ModelYear"));
    let year: i64 = year_s.parse().unwrap_or(0);
    if make.is_empty() || year == 0 {
        anyhow::bail!("VIN 解码没给出 Make/ModelYear，无法冷启动。原始返回里 ErrorText = {err}");
    }
    let mut identity = HashMap::new();
    for k in ["Make","Model","ModelYear","Series","EngineCylinders","DisplacementL",
              "EngineHP","BodyClass","PlantCountry","VehicleType","ErrorText"] {
        let val = g(k);
        if !val.is_empty() { identity.insert(k.to_string(), val); }
    }
    return research_inner(p, Some(vin.to_string()), &make, &model, year, apply, notes, identity);
}

fn research_keep(p: &Platform, keep_id: &str, make: &str, model: &str, year: i64,
                 apply: bool, notes: Vec<String>) -> Result<ColdStart> {
    let mut identity = HashMap::new();
    identity.insert("Make".into(), make.to_string());
    identity.insert("Model".into(), model.to_string());
    identity.insert("ModelYear".into(), year.to_string());
    identity.insert("ErrorText".into(), "未经 VIN 校验".into());
    research_inner2(p, None, make, model, year, apply, notes, identity,
                    Some(keep_id.to_string()))
}

#[allow(dead_code)]
fn research(p: &Platform, vin: Option<String>, make: &str, model: &str, year: i64,
            apply: bool, notes: Vec<String>) -> Result<ColdStart> {
    let mut identity = HashMap::new();
    identity.insert("Make".into(), make.to_string());
    identity.insert("Model".into(), model.to_string());
    identity.insert("ModelYear".into(), year.to_string());
    identity.insert("ErrorText".into(), "未经 VIN 校验".into());
    research_inner(p, vin, make, model, year, apply, notes, identity)
}

#[allow(clippy::too_many_arguments)]
fn research_inner(p: &Platform, vin: Option<String>, make: &str, model: &str, year: i64,
                  apply: bool, notes: Vec<String>,
                  identity: HashMap<String, String>) -> Result<ColdStart> {
    research_inner2(p, vin, make, model, year, apply, notes, identity, None)
}

#[allow(clippy::too_many_arguments)]
fn research_inner2(p: &Platform, vin: Option<String>, make: &str, model: &str, year: i64,
                  apply: bool, mut notes: Vec<String>,
                  identity: HashMap<String, String>,
                  keep_id: Option<String>) -> Result<ColdStart> {
    let make = make.to_string();
    let model = model.to_string();

    // ── 2. 问词表，不猜写法 ──
    let matches = resolve_models(&make, year, &model).unwrap_or_else(|e| {
        notes.push(format!("⚠️ 车型词表查不到（{e}）—— 退回用解码车型名直查，可能漏"));
        vec![ModelMatch { name: model.clone(), confidence: "low".into(),
                          reason: "词表不可用，直接用解码名".into() }]
    });
    if matches.is_empty() {
        notes.push(format!(
            "🔴 **NHTSA 车型词表里没有与「{model}」对应的写法** —— 召回与投诉都查不到。\
             这不是「这台车没有召回」，是「没查到」。\
             人工确认 NHTSA 对这台车叫什么，再用 motomech tool nhtsa.recalls 手查。"));
    }
    let _names: Vec<String> = matches.iter().map(|m| m.name.clone()).collect();

    // ── 3. 召回（A 级）──
    //
    // ⚠️ 车型名匹配到多个写法时，**不能把它们的结果一股脑收进来**。
    // 实测：用 R1200 RT（另一台车）的名字查 2009 BMW，会返回 09V256000，
    // 而它的原文写着「1,535 MY 2007-2009 **R1200RT** MOTORCYCLES」—— 与 GS 无关。
    // 去重时谁先查到算谁的，本车档案就这么被塞进一条别的车的召回。
    //
    // 判据用 NHTSA 自己的 Summary：它明写覆盖哪些车型。
    //   高置信车型名的结果 → 直接收（接口说这个车型有这条召回）
    //   低置信车型名的结果 → **必须原文提到本车型**才收，否则只作参考
    let self_norm = norm(&model);
    let mut recalls: Vec<RecallBrief> = Vec::new();
    let mut rejected: Vec<RecallBrief> = Vec::new();
    let mut seen = Vec::new();
    // 先跑高置信，让它们优先认领 campaign 号
    let ordered: Vec<&ModelMatch> = matches.iter().filter(|m| m.confidence == "high")
        .chain(matches.iter().filter(|m| m.confidence != "high")).collect();
    for m in ordered {
        let url = format!(
            "https://api.nhtsa.gov/recalls/recallsByVehicle?make={}&model={}&modelYear={year}",
            q(&make), q(&m.name));
        if let Ok(v) = fetch(&url) {
            for x in v.get("results").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                let id = x.get("NHTSACampaignNumber").and_then(|y| y.as_str()).unwrap_or("").to_string();
                if id.is_empty() || seen.contains(&id) { continue; }
                let f = |k: &str| x.get(k).and_then(|y| y.as_str()).unwrap_or("").trim().to_string();
                let summary = f("Summary");
                let brief = RecallBrief {
                    id: id.clone(), component: f("Component"),
                    summary: summary.chars().take(300).collect(),
                    consequence: f("Consequence").chars().take(200).collect(),
                    from_model: m.name.clone(),
                };
                if m.confidence == "high" {
                    seen.push(id); recalls.push(brief);
                } else if norm(&summary).contains(&self_norm) {
                    seen.push(id);
                    notes.push(format!(
                        "召回 {} 来自低置信车型名「{}」，但原文明确提到本车型，已收入",
                        brief.id, m.name));
                    recalls.push(brief);
                } else {
                    seen.push(id); rejected.push(brief);
                }
            }
        }
    }
    if !rejected.is_empty() {
        notes.push(format!(
            "有 {} 条召回是从别的车型名查到的，**原文核对后不适用本车**，已排除出档案：{}",
            rejected.len(),
            rejected.iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(" ")));
    }

    // ── 4. 投诉（A 级原文）──
    let mut by_comp: HashMap<String, (usize, String)> = HashMap::new();
    let mut total = 0usize;
    let mut seen_odi = Vec::new();
    // 投诉**只用高置信车型名** —— 低置信的是别的车，它们的投诉混进来
    // 会让本车的故障画像整个失真（实测混入后 30 条变成 86 条）。
    let hi: Vec<&ModelMatch> = matches.iter().filter(|m| m.confidence == "high").collect();
    if hi.is_empty() {
        let cands: Vec<&str> = matches.iter().map(|m| m.name.as_str()).collect();
        notes.push(format!(
            "🔴 **没有高置信车型名，投诉一条都没查** —— 空的 known_issues 在这里是\
             「查不到」不是「没有」。\n     NHTSA 把 {year} {make} 的这类车叫：{}。\n     \
             档案里的 model 写作「{model}」，与词表对不上。\n     \
             **需要你确认本车是哪个变体**，然后把 VEHICLE.yml 的 model 改成词表里的写法，\
             或补上 VIN（有 VIN 就不用猜）。改完重跑本命令即可。",
            if cands.is_empty() { "（词表里一个都没匹配到）".to_string() }
            else { cands.join(" / ") }));
    }
    for m in &hi {
        let url = format!(
            "https://api.nhtsa.gov/complaints/complaintsByVehicle?make={}&model={}&modelYear={year}",
            q(&make), q(&m.name));
        if let Ok(v) = fetch(&url) {
            for x in v.get("results").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                let odi = x.get("odiNumber").map(|y| y.to_string()).unwrap_or_default();
                if seen_odi.contains(&odi) { continue; }
                seen_odi.push(odi);
                total += 1;
                let comp = x.get("components").and_then(|y| y.as_str()).unwrap_or("?").to_string();
                let sm: String = x.get("summary").and_then(|y| y.as_str()).unwrap_or("")
                    .replace('\n', " ").chars().take(220).collect();
                let e = by_comp.entry(comp).or_insert((0, String::new()));
                e.0 += 1;
                if e.1.len() < sm.len() { e.1 = sm; }
            }
        }
    }
    let mut issues: Vec<IssueBrief> = by_comp.into_iter()
        .map(|(c, (n, qte))| IssueBrief { components: c, count: n, quote: qte }).collect();
    issues.sort_by(|a, b| b.count.cmp(&a.count));

    // ── 5. 生成档案 ──
    // 已有车辆沿用它自己的 id —— 重新编一个会让提案对不上原档案
    let vid = keep_id.unwrap_or_else(|| format!("{year}-{}-{}",
        make.split_whitespace().next().unwrap_or(&make).to_uppercase(),
        model.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()));
    let vdir = p.vehicles_dir().join(&vid);
    let exists = vdir.is_dir();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let vin_s = vin.clone().unwrap_or_else(|| "~".into());
    let yml = render_yaml(&vid, &make, &model, year, &vin_s, &identity, &matches,
                          &recalls, &issues, total, &today);

    let (written_to, is_proposal) = if exists {
        // 已有车辆：只产出提案，绝不覆盖人写过的档案
        let pdir = p.root.join(".proposals");
        std::fs::create_dir_all(&pdir)?;
        let f = pdir.join(format!("{vid}-{today}.yml"));
        std::fs::write(&f, &yml)?;
        notes.push(format!(
            "车辆 {vid} 已存在 —— **没有覆盖**，结果写成提案。\
             自己比对后合并，尤其别丢掉已有的 hard_rules / specs_verified / phase_tasks。"));
        (f.to_string_lossy().to_string(), true)
    } else if apply {
        std::fs::create_dir_all(vdir.join("00_车辆档案"))?;
        for d in ["10_手册资料","20_电路系统","30_诊断扫描","40_维修日志",
                  "50_零件","60_影像","70_规格数据"] {
            std::fs::create_dir_all(vdir.join(d))?;
        }
        std::fs::write(vdir.join("VEHICLE.yml"), &yml)?;
        (vdir.to_string_lossy().to_string(), false)
    } else {
        let pdir = p.root.join(".proposals");
        std::fs::create_dir_all(&pdir)?;
        let f = pdir.join(format!("{vid}-{today}.yml"));
        std::fs::write(&f, &yml)?;
        notes.push("未加 --apply，只写了提案。确认无误后加 --apply 建车辆目录。".into());
        (f.to_string_lossy().to_string(), true)
    };

    notes.push(format!(
        "已自动拿到的：身份（VIN 解码 A 级）、召回 {} 条、投诉 {} 条。\
         **拿不到的**：发动机代际代号、适用性排除项、硬红线、任何规格 —— \
         这些没有官方接口，必须靠手册与人工研究。档案里留空并已登记为待核实，\
         跑 `motomech gaps -v {vid}` 看清单。",
        recalls.len(), total));

    Ok(ColdStart { vin, identity, vehicle_id: vid,
                   model_matches: matches, recalls, rejected_recalls: rejected, issues, complaint_total: total,
                   written_to, is_proposal, notes })
}

fn esc(s: &str) -> String { s.replace('"', "'").replace('\n', " ") }

fn render_yaml(vid: &str, make: &str, model: &str, year: i64, vin: &str,
               identity: &HashMap<String, String>, matches: &[ModelMatch],
               recalls: &[RecallBrief], issues: &[IssueBrief], total: usize,
               today: &str) -> String {
    let mut o = String::new();
    o.push_str(&format!("# {year} {make} {model} — 适用性真相源\n#\n"));
    o.push_str(&format!("# 由 `motomech coldstart --vin {vin}` 于 {today} 生成。\n"));
    o.push_str("#\n# ⚠️ 这是**骨架**，不是完成的档案。\n");
    o.push_str("#    自动拿到的只有：身份、召回、投诉 —— 它们有官方接口。\n");
    o.push_str("#    **拿不到的**：发动机代际、适用性排除项、硬红线、任何规格数值。\n");
    o.push_str("#    那些留空是故意的 —— 填一个看起来像样的猜测，比空着危险得多。\n");
    o.push_str("#    跑 `motomech gaps` 看还缺什么。\n\n");

    o.push_str(&format!("id: {vid}\nmake: {make}\nmodel: \"{}\"\nyear: {year}\n", esc(model)));
    o.push_str(&format!("vin: {vin}                 # A｜NHTSA 解码\n"));
    for (k, label) in [("Series","model_code"), ("EngineHP","power_hp"),
                       ("DisplacementL","displacement_l"), ("EngineCylinders","cylinders"),
                       ("PlantCountry","plant_country")] {
        if let Some(v) = identity.get(k) {
            o.push_str(&format!("{label}: \"{}\"   # A｜NHTSA 解码\n", esc(v)));
        }
    }
    o.push_str("\n# ── 身份里**自动拿不到**的部分 ──\n");
    o.push_str("engine_code: ~                  # 代际代号（hexhead/camhead 这类）无官方接口，靠手册\n");
    o.push_str("mileage: ~\nproduction_date: ~\n\n");

    o.push_str("# ── 硬性排除 ───────────────────────────────────\n");
    o.push_str("# 🔴 **空的**。适用性过滤是本平台防止「拿别代资料修本车」的唯一机制，\n");
    o.push_str("#    而它必须靠人研究这台车的代际分界才能填。\n");
    o.push_str("#    **在填上之前不要往 10_手册资料/ 里放任何手册** —— 过滤器挡不住任何东西。\n");
    o.push_str("applicability_excludes: []\n\n");

    o.push_str("# ── 硬红线 ─────────────────────────────────────\n");
    o.push_str("# 🔴 **空的**。停放多年的车不可能一条都没有，这里空着说明还没研究。\n");
    o.push_str("hard_rules: []\n\n");

    o.push_str("specs_verified: {}\n\n");
    o.push_str("specs_pending:\n");
    o.push_str("  - item: engine_code\n    candidate: ~\n    level: 无\n");
    o.push_str("    note: 代际代号决定所有资料选型，必须先定。查该车型的代际划分\n");
    o.push_str("  - item: applicability_excludes\n    candidate: ~\n    level: 无\n");
    o.push_str("    note: \"**最高优先级** —— 没有它，适用性过滤等于不存在\"\n\n");

    o.push_str("# ── 研究状态 ───────────────────────────────────\n");
    o.push_str(&format!("recalls_checked_at: {today}\nknown_issues_checked_at: {today}\n"));
    o.push_str(&format!("# NHTSA 车型词表匹配到的写法（**这一步不猜，去问词表**）：\n"));
    for m in matches {
        o.push_str(&format!("#   {} [{}] {}\n", m.name, m.confidence, m.reason));
    }
    o.push('\n');

    o.push_str("# ── 未执行召回（A 级 · NHTSA 官方）─────────────\n");
    if recalls.is_empty() {
        o.push_str("open_recalls: []\n");
        o.push_str("# ⚠️ 空 = 按上面那些车型写法**查过了没有**（见 recalls_checked_at）。\n");
        o.push_str("#    如果词表匹配是 low 置信，那更可能是名字没对上 —— 请人工复核。\n\n");
    } else {
        o.push_str("open_recalls:\n");
        for r in recalls {
            o.push_str(&format!("  - id: {}\n    component: \"{}\"\n    severity: unknown\n",
                r.id, esc(&r.component)));
            o.push_str(&format!("    note: \"{}｜后果：{}（匹配自车型写法 {}）\"\n",
                esc(&r.summary), esc(&r.consequence), r.from_model));
        }
        o.push_str("# severity 一律 unknown —— NHTSA 不给分级，需人读 Consequence 后判定\n\n");
    }

    o.push_str(&format!("# ── 已知故障（A 级 · NHTSA 投诉，共 {total} 条）──\n"));
    if issues.is_empty() {
        o.push_str("known_issues: []\n");
    } else {
        o.push_str("known_issues:\n");
        for i in issues.iter().take(8) {
            o.push_str(&format!("  - level: A\n    item: \"{}（{} 条投诉）\"\n",
                esc(&i.components), i.count));
            o.push_str(&format!("    note: \"投诉原话：{}\"\n", esc(&i.quote)));
        }
        o.push_str("# 按投诉部件分组计数，引文取该组最长的一条。**分组与解读是机器做的，\n");
        o.push_str("#  原话才是 A 级** —— 要写进作业单前请读原文。\n");
    }
    o
}

pub fn render(c: &ColdStart) -> String {
    let mut o = format!("VIN {}\n", c.vin.clone().unwrap_or_default());
    o.push_str(&format!("身份：{} {} {}（{}）\n",
        c.identity.get("ModelYear").cloned().unwrap_or_default(),
        c.identity.get("Make").cloned().unwrap_or_default(),
        c.identity.get("Model").cloned().unwrap_or_default(),
        c.identity.get("ErrorText").cloned().unwrap_or_default()));
    o.push_str(&format!("车辆 id：{}\n\n", c.vehicle_id));
    o.push_str("NHTSA 车型词表匹配（不猜写法，去问词表）：\n");
    for m in &c.model_matches {
        o.push_str(&format!("  [{}] {} —— {}\n", m.confidence, m.name, m.reason));
    }
    o.push_str(&format!("\n召回 {} 条：\n", c.recalls.len()));
    for r in &c.recalls {
        o.push_str(&format!("  {} · {}\n    {}\n", r.id, r.component,
            r.consequence.chars().take(110).collect::<String>()));
    }
    o.push_str(&format!("\n投诉 {} 条，按部件：\n", c.complaint_total));
    for i in c.issues.iter().take(8) {
        o.push_str(&format!("  {:>3} 条  {}\n", i.count, i.components));
    }
    o.push_str(&format!("\n{} → {}\n",
        if c.is_proposal { "提案（未写入车辆目录）" } else { "已建车辆目录" }, c.written_to));
    for n in &c.notes { o.push_str(&format!("\n· {n}\n")); }
    o
}
