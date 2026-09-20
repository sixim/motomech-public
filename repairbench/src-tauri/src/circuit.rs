//! 电路图定位（D3）—— 回答「本车这个系统的电路图是哪一页」
//!
//! ## 为什么要单独做一层
//!
//! 电路图页**进不了全文检索**：它们是图，OCR 只捞到零散标签。
//! 实测用「wiring」检索 haynes-4598，命中 p261–288 的次数是 **0** ——
//! 28 页彩色电路图对 app 是隐形的，有等于没有。
//!
//! ## 为什么默认过滤年份，而且不适用的要显式索取
//!
//! 同一台 GS，2004–2006 / 2007 / 2008-on 是**三套不同的图**。
//! 拿错档去背刺插头、量电压，量的是另一代车的线 ——
//! 这不只是查错资料，是会伤人或烧件。
//!
//! 所以这里照搬手册检索那套纪律：**默认只出适用本车的**，
//! 并且**说出挡掉了几组**（不说的话，「只有 3 页」会被误读成「书里只有这些」）。

use crate::platform::{Platform, Vehicle};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct Group {
    pub model: String,
    pub model_codes: Vec<String>,
    pub year_from: i64,
    pub year_to: i64,
    pub pages: Vec<i64>,
    pub book_pages: Vec<String>,
    pub systems: Vec<String>,
    pub applicable: bool,
    /// 每页的具体内容（有就给，没有就空）
    pub detail: Vec<(i64, String)>,
    /// 页图绝对路径，与 pages 一一对应；没找到的为空串
    pub images: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub vehicle_id: String,
    pub model: String,
    pub year: i64,
    pub groups: Vec<Group>,
    /// 因不适用本车而挡掉的组数 —— **必须说出来**
    pub filtered_out: usize,
    pub query: Option<String>,
    pub notes: Vec<String>,
}

fn sv(v: &serde_yaml_ng::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(String::from)
}
fn list(v: &serde_yaml_ng::Value, k: &str) -> Vec<String> {
    v.get(k).and_then(|x| x.as_sequence())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

// 适用性判定全部委派 `applicability::judge` —— 全项目唯一一份。
// 此前这里自己写了一份裸子串匹配，于是「SPO**RT**STER」含 RT，
// 一台哈雷拿到了 BMW R1200 RT 的电路图。判定只有一份，才不会再有下一个 Sportster。

pub fn build(p: &Platform, v: &Vehicle, query: Option<&str>, include_other: bool) -> Result<Report> {
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let vmodel = sv(&doc, "model").unwrap_or_default();
    let vyear = doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0);
    let _ = (&vmodel, vyear);

    let f = p.root.join("knowledge").join("电路图索引.yml");
    let idx: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(&f).with_context(|| format!("读不到 {}", f.display()))?)?;

    // 页图在哪：从索引库里拿该手册的页图路径（页目录形态才有）
    let conn = crate::db::open(&p.index_db())?;
    let img_of = |page: i64| -> String {
        conn.query_row(
            "SELECT COALESCE(mp.image_path,'') FROM manual_pages mp JOIN manuals m ON m.id=mp.manual_id \
             WHERE m.vehicle_id=?1 AND m.title=?2 AND mp.page_no=?3",
            rusqlite::params![&v.id, idx.get("manual").and_then(|x| x.as_str()).unwrap_or(""), page],
            |r| r.get::<_, String>(0)).unwrap_or_default()
    };

    // ── 第一道闸:厂牌 ──
    // 索引声明它画的是哪个厂牌的车。厂牌对不上,一组都不给「适用」。
    // fail-closed:任一侧缺 make 也当不适用 —— 宁可让人显式 --all,
    // 也不要把别的厂牌的电路图标成「✅ 适用本车」。
    let ident = crate::applicability::ident_from(&doc);
    let imake = sv(&idx, "make");

    let mut groups = Vec::new();
    let mut filtered_out = 0usize;
    let q = query.map(|s| s.to_lowercase());

    if let Some(seq) = idx.get("diagrams").and_then(|x| x.as_sequence()) {
        for g in seq {
            let years: Vec<i64> = g.get("years").and_then(|x| x.as_sequence())
                .map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            let (yf, yt) = (years.first().copied().unwrap_or(0), years.get(1).copied().unwrap_or(9999));
            let model = sv(g, "model").unwrap_or_default();
            let verdict = crate::applicability::judge(&ident, &crate::applicability::Declares {
                make: imake.clone(),
                model: Some(model.clone()),
                years: Some((yf, yt)),
                generations: vec![],
            });
            let applicable = verdict.ok();

            let systems = list(g, "systems");
            // 关键词过滤：车系名、系统名、页号都算命中
            if let Some(qq) = &q {
                let pages_s: Vec<String> = g.get("pages").and_then(|x| x.as_sequence())
                    .map(|a| a.iter().filter_map(|x| x.as_i64()).map(|n| n.to_string()).collect())
                    .unwrap_or_default();
                let hay = format!("{} {} {}", model.to_lowercase(),
                                  systems.join(" ").to_lowercase(), pages_s.join(" "));
                if !hay.contains(qq.as_str()) { continue; }
            }
            if !applicable && !include_other { filtered_out += 1; continue; }

            let pages: Vec<i64> = g.get("pages").and_then(|x| x.as_sequence())
                .map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            let detail: Vec<(i64, String)> = g.get("detail").and_then(|x| x.as_mapping())
                .map(|m| m.iter().filter_map(|(k, val)| {
                    Some((k.as_i64()?, val.as_str()?.to_string()))
                }).collect()).unwrap_or_default();
            let images = pages.iter().map(|p| img_of(*p)).collect();

            groups.push(Group {
                model, model_codes: list(g, "model_codes"),
                year_from: yf, year_to: yt,
                book_pages: list(g, "book_pages"),
                systems, applicable, detail, images,
                note: sv(g, "note"), pages,
            });
        }
    }

    let mut notes = Vec::new();
    if let Some(m) = &imake {
        let mk = crate::applicability::judge(&ident, &crate::applicability::Declares {
            make: Some(m.clone()), ..Default::default() });
        if !mk.ok() {
            notes.push(format!("🔴 **{}** —— 一组都不适用。\
                电路图是用来背刺插头、量电压的,拿错厂牌的图会伤人。", mk.reason()));
        }
    }
    if groups.iter().all(|g| !g.applicable) && !groups.is_empty() {
        notes.push(format!(
            "🔴 没有一组电路图适用 {vyear} 年的「{vmodel}」—— 上面列出的都是别的车系或别的年款。\
             **不要拿它们当本车的图用。**"));
    }
    if let Some(r) = idx.get("rotation").and_then(|x| x.as_i64()) {
        notes.push(format!("ℹ️ 图是横向印在竖页上的，看之前把图转 {r}°。"));
    }
    notes.push(concat!(
        "ℹ️ 本表只是**定位器**：线色、端子号、线径一律以页图为准。",
        "表里不复述任何数值 —— 复述就会和原图脱节，而人只会读复述的那份。").to_string());

    Ok(Report { vehicle_id: v.id.clone(), model: vmodel, year: vyear,
                groups, filtered_out, query: query.map(String::from), notes })
}

pub fn render(r: &Report) -> String {
    let mut o = format!("电路图  {} {} {}\n", r.year, r.model, r.vehicle_id);
    if let Some(q) = &r.query { o.push_str(&format!("检索：{q}\n")); }
    o.push_str(&"─".repeat(62));
    o.push('\n');

    if r.groups.is_empty() {
        o.push_str("\n（没有匹配的电路图组）\n");
    }
    for g in &r.groups {
        let mark = if g.applicable { "✅ 适用本车" } else { "⚠️ 不适用本车" };
        o.push_str(&format!("\n{mark}  {} [{}]  {}–{}\n",
            g.model, g.model_codes.join(" / "), g.year_from,
            if g.year_to >= 9999 { "…".into() } else { g.year_to.to_string() }));
        o.push_str(&format!("   系统：{}\n", g.systems.join("、")));
        for (i, p) in g.pages.iter().enumerate() {
            let bp = g.book_pages.get(i).cloned().unwrap_or_default();
            o.push_str(&format!("   · PDF p{p}  书页 {bp}\n"));
            if let Some((_, d)) = g.detail.iter().find(|(k, _)| k == p) {
                o.push_str(&format!("       {d}\n"));
            }
            if let Some(img) = g.images.get(i) {
                if !img.is_empty() { o.push_str(&format!("       图：{img}\n")); }
            }
        }
        if let Some(n) = &g.note { o.push_str(&format!("   {n}\n")); }
    }

    if r.filtered_out > 0 {
        o.push_str(&format!(concat!(
            "\n⚠️ 另有 **{} 组**电路图属于别的车系/年款，已按适用性挡住。\n",
            "   不说出来的话，「只有这几页」会被误读成「书里只有这些」。\n",
            "   要看用 `--all`（看之前先问自己：为什么要看别的车的图）。\n"), r.filtered_out));
    }
    for n in &r.notes { o.push_str(&format!("\n{n}\n")); }
    o
}
