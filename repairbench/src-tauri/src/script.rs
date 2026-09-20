//! Vlog 脚本体检（E6）
//!
//! ## 为什么脚本需要体检，而笔记不需要
//!
//! 作业单里写错一个扭力值，错的是你一个人，而且拧的时候多半会觉得不对劲。
//! **视频里念错一个扭力值，会被观众照抄。** 同一个错误被复制成千份，
//! 而且没有任何人会回来核对 —— 这是整个项目里错误代价最高的地方。
//!
//! 所以：脚本里出现的每个技术数字，都要能在**已核实规格**里找到对应。
//! 找不到的不一定是错的，但**必须被指出来**，由人决定是删掉、改口，还是去核实。
//!
//! ## 它不做什么
//!
//! 不改脚本、不替你决定。只做一件事：把「说不出出处的数字」摆到你面前。
//! 沉默地放行才是这个项目要防的那种失败。

use crate::platform::{Platform, Vehicle};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Serialize)]
pub struct Claim {
    pub line_no: usize,
    /// 原文里的那个数字（含单位）
    pub text: String,
    /// 整行上下文，便于判断它到底在说什么
    pub context: String,
    /// verified / unsourced / conflicted
    pub state: String,
    /// 命中的规格（verified 时给）
    pub matched: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScriptReport {
    pub file: String,
    pub title: String,
    pub claims: Vec<Claim>,
    pub verified: usize,
    pub unsourced: usize,
    pub conflicted: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub vehicle_id: String,
    pub dir: String,
    pub scripts: Vec<ScriptReport>,
    pub notes: Vec<String>,
}

/// 抽出「像技术数字」的片段：数字 + 单位。
///
/// 刻意**只认带单位的**。纯数字满篇都是（集数、年份、秒数、"十年"），
/// 全报出来等于没报 —— 天天误报的检查器会被人无视，那时真正的错值也一起放行。
/// 这是门禁那条教训的同一版本。
fn extract_numbers(line: &str) -> Vec<String> {
    const UNITS: &[&str] = &["Nm", "nm", "mm", "cm", "kg", "km", "L", "l", "ml", "V", "v",
                             "A", "W", "kW", "bar", "psi", "Ω", "kΩ", "℃", "°C", "rpm", "Ah"];
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            // 数字本体：允许小数点、区间号、千分位
            while i < chars.len() && (chars[i].is_ascii_digit()
                  || chars[i] == '.' || chars[i] == '–' || chars[i] == '-' || chars[i] == '~') {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            // 跳过紧跟的空格
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' { j += 1; }
            let tail: String = chars[j..].iter().take(4).collect();
            if let Some(u) = UNITS.iter().find(|u| tail.starts_with(**u)) {
                out.push(format!("{num} {u}"));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// 数字是否出现在某条已核实规格的值里。
/// 只比数字部分 —— 单位写法千变万化（Nm / N·m / 牛米），比单位会制造误报。
fn num_of(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '–' || *c == '-' || *c == '~')
        .collect()
}

/// 从 item 名推单位族（规格库把单位放在名字上）。
fn unit_family(item: &str) -> String {
    let c = item.to_lowercase();
    for (suf, fam) in [("_nm","nm"),("_kohms","kohm"),("_ohms","ohm"),("_mm","mm"),
                       ("_psi","psi"),("_kg","kg"),("_w","w"),("_a","a"),("_v","v"),
                       ("_l","l"),("_km","km"),("_bar","bar")] {
        if c.ends_with(suf) { return fam.into(); }
    }
    String::new()
}

/// 脚本里那个单位属于哪个族。
fn claim_family(unit: &str) -> String {
    match unit.trim() {
        "Nm" | "nm" => "nm", "mm" => "mm", "cm" => "mm",
        "Ω" => "ohm", "kΩ" => "kohm",
        "L" | "l" | "ml" => "l", "V" | "v" => "v", "A" => "a",
        "W" | "kW" => "w", "bar" => "bar", "psi" => "psi",
        "kg" => "kg", "km" => "km", _ => "",
    }.to_string()
}

pub fn check(p: &Platform, v: &Vehicle) -> Result<Report> {
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let raw = doc.get("vlog_path").and_then(|x| x.as_str()).unwrap_or("");
    let dir = if let Some(rest) = raw.strip_prefix("~/") {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest)
    } else { std::path::PathBuf::from(raw) }.join("脚本");

    // 已核实值集合 + 冲突 canonical 集合
    let conn = crate::db::open(&p.index_db())?;
    // (数字, 单位族, 规格描述)。单位族防止「20 Nm」配上「20W/50 机油黏度」——
    // 2026-09-18 反向验证抓到的：只比数字会把任何同值项当成佐证。
    let mut verified_nums: Vec<(String, String, String)> = Vec::new();
    {
        let mut st = conn.prepare(
            "SELECT item, value FROM specs_usable WHERE vehicle_id=?1")?;
        let rows = st.query_map([&v.id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?)))?;
        for row in rows.flatten() {
            // 值里可能有多个数字（"0.7–0.9"、"初 23 → 终 32"）—— 全都收
            let mut cur = String::new();
            for ch in row.1.chars() {
                if ch.is_ascii_digit() || ch == '.' { cur.push(ch); }
                else {
                    if !cur.is_empty() {
                        verified_nums.push((cur.clone(), unit_family(&row.0), format!("{} = {}", row.0, row.1)));
                    }
                    cur.clear();
                }
            }
            if !cur.is_empty() {
                verified_nums.push((cur, unit_family(&row.0), format!("{} = {}", row.0, row.1)));
            }
        }
    }
    // 冲突检测：比的是**冲突的那几个数值**，不是英文 canonical 名。
    //
    // 2026-09-19 监工指出：原来按 canonical 名在正文里搜，
    // 而脚本是中文的、正文里根本不会出现 `side_stand_pivot_bolt` 这种串 ——
    // 那段检测对真实脚本是**死代码**，永远不会触发。
    // 真正要拦的是「主播念出了一个有争议的数值」。
    let mut conflicted: HashSet<String> = HashSet::new();          // canonical 名（仍保留）
    let mut conflict_vals: Vec<(String, String, String)> = Vec::new(); // (数字, 单位族, canonical)
    {
        let mut st = conn.prepare(
            "SELECT c.canonical, c.values_csv, \
                    (SELECT item FROM specs WHERE vehicle_id=c.vehicle_id AND canonical=c.canonical LIMIT 1) \
             FROM spec_conflicts c WHERE c.vehicle_id=?1")?;
        let rows = st.query_map([&v.id], |r| Ok((
            r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2).unwrap_or_default())))?;
        for (canon, vals, item) in rows.flatten() {
            conflicted.insert(canon.clone());
            let fam = unit_family(&item);
            for part in vals.split(',') {
                let d: String = part.chars().filter(|c| c.is_ascii_digit() || *c=='.').collect();
                if !d.is_empty() { conflict_vals.push((d, fam.clone(), canon.clone())); }
            }
        }
    }

    let mut scripts = Vec::new();
    let mut notes = Vec::new();
    if !dir.is_dir() {
        notes.push(format!("🔴 脚本目录不存在：{}", dir.display()));
        return Ok(Report { vehicle_id: v.id.clone(), dir: dir.to_string_lossy().into(),
                           scripts, notes });
    }

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)?.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    files.sort();

    for f in &files {
        let txt = match std::fs::read_to_string(f) { Ok(t) => t, Err(_) => continue };
        let title = txt.lines().find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").to_string())
            .unwrap_or_else(|| f.file_stem().unwrap_or_default().to_string_lossy().into());
        let mut claims = Vec::new();
        for (i, line) in txt.lines().enumerate() {
            // 冲突 canonical 被直接点名 —— 这类值一个都不能上镜
            for c in &conflicted {
                if line.contains(c.as_str()) {
                    claims.push(Claim {
                        line_no: i + 1, text: c.clone(),
                        context: line.trim().chars().take(80).collect(),
                        state: "conflicted".into(), matched: None,
                    });
                }
            }
            for n in extract_numbers(line) {
                let key = num_of(&n);
                let fam = claim_family(n.trim_start_matches(|c: char|
                    c.is_ascii_digit() || c=='.' || c=='–' || c=='-' || c=='~' || c==' '));
                // 单位族必须一致：扭力只跟扭力比。否则「20 Nm」会配上「20W/50 黏度」。
                // 先看它是不是某个**有争议**的值 —— 那类数字一个都不能上镜
                if let Some((_, _, canon)) = conflict_vals.iter()
                    .find(|(d, f, _)| *d == key && (fam.is_empty() || f.is_empty() || *f == fam)) {
                    claims.push(Claim {
                        line_no: i + 1, text: n.clone(),
                        context: line.trim().chars().take(80).collect(),
                        state: "conflicted".into(),
                        matched: Some(format!("{canon}（该规格有多个矛盾来源）")),
                    });
                    continue;
                }
                let hit = verified_nums.iter()
                    .find(|(d, f, _)| *d == key && (fam.is_empty() || *f == fam));
                claims.push(Claim {
                    line_no: i + 1,
                    text: n.clone(),
                    context: line.trim().chars().take(80).collect(),
                    // **不叫「已核实」**：同值不等于同一个件。
                    // 2026-09-18 实测：编造的「边撑 45 Nm」因为库里 telelever 枢轴恰好是 45
                    // 而被判「已核实」—— 那是撒谎。它只能说明「库里有个同值项」。
                    state: if hit.is_some() { "same_value".into() } else { "unsourced".into() },
                    matched: hit.map(|(_, _, d)| d.clone()),
                });
            }
        }
        let verified = claims.iter().filter(|c| c.state == "same_value").count();
        let unsourced = claims.iter().filter(|c| c.state == "unsourced").count();
        let conflicted_n = claims.iter().filter(|c| c.state == "conflicted").count();
        scripts.push(ScriptReport {
            file: f.file_name().unwrap_or_default().to_string_lossy().into(),
            title, claims, verified, unsourced, conflicted: conflicted_n,
        });
    }
    if scripts.is_empty() { notes.push("脚本目录是空的 —— 还没有可体检的脚本".into()); }
    Ok(Report { vehicle_id: v.id.clone(), dir: dir.to_string_lossy().into(), scripts, notes })
}

pub fn render(r: &Report) -> String {
    let mut o = format!("Vlog 脚本体检  车辆 {}\n{}\n", r.vehicle_id, "─".repeat(60));
    o.push_str(&format!("脚本目录：{}\n", r.dir));
    for n in &r.notes { o.push_str(&format!("\n{n}\n")); }

    for s in &r.scripts {
        let flag = if s.conflicted > 0 { "🔴" }
                   else if s.unsourced > 0 { "⚠️ " }
                   else if s.verified > 0 { "◻️" } else { "✅" };
        o.push_str(&format!("\n{flag} {}  「{}」\n", s.file, s.title));
        o.push_str(&format!("   技术数字 {} 个：库中有同值项 {} · **说不出出处 {}** · 冲突 {}\n",
            s.claims.len(), s.verified, s.unsourced, s.conflicted));
        for c in &s.claims {
            match c.state.as_str() {
                "conflicted" => o.push_str(&format!(
                    "   🔴 第{}行 {} —— **这个数有争议，一个都不能上镜**{}\n      {}\n",
                    c.line_no, c.text,
                    c.matched.as_deref().map(|m| format!("（{m}）")).unwrap_or_default(),
                    c.context)),
                "unsourced" => o.push_str(&format!(
                    "   ⚠️  第{}行 {} —— 规格库里找不到这个数\n      {}\n",
                    c.line_no, c.text, c.context)),
                _ => o.push_str(&format!(
                    "   ◻️ 第{}行 {}  库中同值项 → {}\n      **核对是不是同一个件**；不是就说明这个数是错的\n",
                    c.line_no, c.text, c.matched.as_deref().unwrap_or(""))),
            }
        }
    }
    o.push_str(concat!(
        "\n⚠️ 「库中有同值项」**不等于已核实**：同一个数字可能属于另一个件。\n",
        "   实测例：编造的「边撑 45 Nm」因为 telelever 枢轴恰好是 45 而配上了 ——\n",
        "   所以这里只给线索，判断是不是同一个件的责任在你。\n",
        "ℹ️ 只认**带单位**的数字。纯数字（集数、年份、秒数）不报 ——\n",
        "   天天误报的检查器会被人无视，那时真正的错值也一起放行。\n",
        "⚠️ 「说不出出处」不等于错，但它必须被你看见：删掉、改口，或者去核实。\n",
        "   视频里的数字会被观众照抄，而没有人会回来核对。\n"));
    o
}
