//! G4 交叉验证 —— 安全关键项需要两个**独立**来源
//!
//! ## 它和冲突检测是两件事
//!
//! * `spec_conflicts` 抓的是**两个来源打架**：两个值都带出处，所以两个都扣下。
//! * 本模块抓的是**只有一个来源撑着**：值没问题，但没人给它背书。
//!   前者是明摆着的矛盾，后者是**看不见的单点** —— 更危险，因为它长得和
//!   已核实的数据一模一样。
//!
//! 本项目栽过的正是后者：整套规格出自一本手册，而那本手册对不上这台车
//! （2010 车主手册 → 2009 hexhead），16 条 A 级规格全部作废。
//! 当时没有任何机制会说「这些值全靠一个来源」。这个模块就是补那一刀。
//!
//! ## 「独立」怎么算 —— 必须归一化到文档
//!
//! 2026-09-16 实测：本车 118 条已核实规格的 `source_ref` 有十几种写法
//! （`Haynes 4598 · Torque settings`、`Haynes 4598 · Ch2 · 2•2`…），
//! 按 `source_ref` 去重会得出「来源很多」——**全是同一本书的不同章节**。
//! 所以判独立性要把章节/页码剥掉，只留文档身份：
//! `Haynes 4598 · Ch2 · 2•2` → `haynes4598`。
//!
//! ## 为什么是「披露」而不是「扣下」
//!
//! 冲突扣下值是对的（两个值没法选）。单来源**不扣**：
//! 这台车几乎每条规格都只有 Haynes 一本背书，全扣等于把 app 变成空壳。
//! 正确的比例是**让它在用的那一刻显形** —— 你正要拧刹车卡钳时，
//! 旁边写着「⚠️ 单来源」，你自己决定要不要先去核。

use crate::platform::{Platform, Vehicle};
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

/// 安全关键判定词 —— 命中即视为安全关键。
///
/// 选取标准：**错了会伤人或让车失控**。刹车、转向、悬挂、车轮/轴、
/// 终传（后轮可能脱开，本车已知故障模式）、边撑（车会倒）、
/// 发动机内部紧固（缸盖/连杆，松了毁机或抱死后轮）。
///
/// 宁可多圈一点：误标的代价是多看一行提示，漏标的代价是无声的单点。
const CRITICAL: &[&str] = &[
    "brake", "caliper", "disc", "pad", "abs",
    "wheel", "axle", "spindle", "hub", "flange",
    "steering", "handlebar", "fork", "yoke", "clamp",
    "shock", "suspension", "swingarm", "pivot",
    "final_drive", "driveshaft", "side_stand", "centre_stand", "center_stand",
    "cylinder_head", "con_rod", "conrod", "crankshaft", "flywheel", "clutch",
    "tyre", "tire",
];

#[derive(Debug, Serialize, Clone)]
pub struct Item {
    pub canonical: String,
    pub items: Vec<String>,
    pub value: String,
    /// 去重到**文档**之后的独立来源
    pub docs: Vec<String>,
    /// 原始 source_ref（可能同一本书的多个章节）
    pub refs: Vec<String>,
    pub corroborated: bool,
    /// 出处文字里自称「交叉印证/一致」但库里没有第二条结构化记录
    pub claims_only: bool,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub vehicle_id: String,
    pub critical_total: usize,
    pub corroborated: Vec<Item>,
    pub single_source: Vec<Item>,
    /// 全库的独立文档清单 —— 一眼看出「整个档案靠几本书撑着」
    pub all_docs: Vec<(String, usize)>,
    pub notes: Vec<String>,
}

/// 把 source_ref 归一化成文档身份：剥掉章节/页码/说明，只留书名与编号。
///
/// `Haynes 4598 · Ch2 · 2•2（版面下漂一行）` → `haynes4598`
/// `NHTSA 召回 A 级`                          → `nhtsa`
/// 出处 → 文档身份。**只认登记表**（`knowledge/来源文档.yml`）。
///
/// 2026-09-19 监工实测打出的教训：靠「取第一个 ≥3 字母的词」猜文档，
/// 同一本 Haynes 4598 会被认成 5 个不同来源（haynes / bmw / **see** / 4598 / 海恩斯）。
/// 于是同一本书的两条记录被算成「两个独立来源互相印证」，
/// 作业单上那条「⚠️ 安全关键·单来源」**静悄悄消失** —— 正是假绿灯。
/// 我此前声称这套判定「刻意保守，宁可多报单来源」，那是**反的**。
///
/// 现在 fail-closed：匹配不上任何登记文档的，全部归入 `unknown` **一个桶** ——
/// 多个认不出的来源不会互相「印证」。少算独立性的代价是多看一行提示；
/// 多算的代价是该警告的地方不警告。
pub fn doc_id_with(registry: &[(String, Vec<String>)], source_ref: &str) -> String {
    let norm: String = source_ref.to_lowercase().chars()
        .filter(|c| c.is_alphanumeric()).collect();
    for (id, pats) in registry {
        if pats.iter().any(|p| !p.is_empty() && norm.contains(p.as_str())) {
            return id.clone();
        }
    }
    "unknown".to_string()
}

/// 读登记表。读不到就返回空表 —— 那样所有出处都落进 `unknown`，
/// 全部按单来源处理。**fail-closed**：登记表缺失不该让独立性凭空变多。
pub fn load_registry(p: &Platform) -> Vec<(String, Vec<String>)> {
    let f = p.root.join("knowledge").join("来源文档.yml");
    let txt = match std::fs::read_to_string(&f) { Ok(t) => t, Err(_) => return vec![] };
    let doc: serde_yaml_ng::Value = match serde_yaml_ng::from_str(&txt) { Ok(d) => d, Err(_) => return vec![] };
    doc.get("documents").and_then(|x| x.as_sequence()).map(|a| a.iter().filter_map(|d| {
        let id = d.get("id")?.as_str()?.to_string();
        let pats: Vec<String> = d.get("match").and_then(|m| m.as_sequence())
            .map(|l| l.iter().filter_map(|x| x.as_str())
                .map(|x| x.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect())
                .collect()).unwrap_or_default();
        Some((id, pats))
    }).collect()).unwrap_or_default()
}

fn is_critical(item: &str, canonical: &str, extra: &[String], exempt: &[String]) -> bool {
    let hay = format!("{item} {canonical}").to_lowercase();
    if exempt.iter().any(|e| hay.contains(&e.to_lowercase())) { return false; }
    if extra.iter().any(|e| hay.contains(&e.to_lowercase())) { return true; }
    CRITICAL.iter().any(|k| hay.contains(k))
}

fn str_list(doc: &serde_yaml_ng::Value, key: &str) -> Vec<String> {
    doc.get(key).and_then(|x| x.as_sequence())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn build(p: &Platform, v: &Vehicle) -> Result<Report> {
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    // 档案可以补充/豁免安全关键判定 —— 关键词是默认值，不是定论
    let extra  = str_list(&doc, "safety_critical_extra");
    let exempt = str_list(&doc, "safety_critical_exempt");

    let registry = load_registry(p);
    let conn: Connection = crate::db::open(&p.index_db())?;
    // 只看可用规格：冲突项已经被 specs_usable 扣下，不在这里重复报
    let mut st = conn.prepare(
        "SELECT canonical, item, value, COALESCE(source_ref,'') \
         FROM specs_usable WHERE vehicle_id=?1 ORDER BY canonical, item")?;
    let rows = st.query_map([&v.id], |r| Ok((
        r.get::<_, String>(0)?, r.get::<_, String>(1)?,
        r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;

    use std::collections::BTreeMap;
    let mut by_canon: BTreeMap<String, Item> = BTreeMap::new();
    let mut doc_count: BTreeMap<String, usize> = BTreeMap::new();

    for row in rows {
        let (canon, item, value, sref) = row?;
        let d = doc_id_with(&registry, &sref);
        if !d.is_empty() { *doc_count.entry(d.clone()).or_insert(0) += 1; }
        if !is_critical(&item, &canon, &extra, &exempt) { continue; }

        let e = by_canon.entry(canon.clone()).or_insert_with(|| Item {
            canonical: canon.clone(), items: vec![], value: value.clone(),
            docs: vec![], refs: vec![], corroborated: false, claims_only: false,
        });
        if !e.items.contains(&item) { e.items.push(item); }
        if !sref.is_empty() && !e.refs.contains(&sref) { e.refs.push(sref.clone()); }
        if !d.is_empty() && !e.docs.contains(&d) { e.docs.push(d); }
        // 出处文字里自称交叉印证，但那是散文不是数据 —— 单独标出来
        if sref.contains("交叉印证") || sref.contains("一致") { e.claims_only = true; }
    }

    let mut corroborated = Vec::new();
    let mut single = Vec::new();
    for (_, mut it) in by_canon {
        it.corroborated = it.docs.len() >= 2;
        // 有第二个文档背书就不是「只有声称」了
        if it.corroborated { it.claims_only = false; }
        if it.corroborated { corroborated.push(it); } else { single.push(it); }
    }
    let critical_total = corroborated.len() + single.len();

    let mut all_docs: Vec<(String, usize)> = doc_count.into_iter().collect();
    all_docs.sort_by(|a, b| b.1.cmp(&a.1));

    let mut notes = Vec::new();
    if registry.is_empty() {
        notes.push("🔴 **来源登记表读不到**（knowledge/来源文档.yml）—— \
            所有出处都按「身份不明」处理，于是全部计为单来源。\
            这是 fail-closed 的结果，不是数据变差了。".to_string());
    }
    if all_docs.iter().any(|(d, _)| d == "unknown") {
        let n = all_docs.iter().find(|(d, _)| d == "unknown").map(|(_, n)| *n).unwrap_or(0);
        notes.push(format!("⚠️ 有 {n} 条规格的出处**匹配不上任何已登记文档**，已全部归入 unknown 一个桶 —— \
            它们之间不会互相「印证」。要让它们参与交叉验证，先在 knowledge/来源文档.yml 里登记。"));
    }
    if all_docs.len() <= 1 {
        notes.push(format!(
            "🔴 **整个档案的已核实规格只靠 {} 个文档撑着。** \
             交叉验证在结构上不可能达成 —— 不是这批数据不好，是**没有第二个来源可比**。\
             本项目栽过一次同样的结构：整套规格出自一本手册，而那本手册对不上这台车。\
             要真正关掉这一项，必须引入第二个独立来源（BMW RepROM / 官方 fiche / 厂商 TSB）。",
            all_docs.len()));
    }
    let claims = single.iter().filter(|x| x.claims_only).count();
    if claims > 0 {
        notes.push(format!(
            "⚠️ 有 {claims} 条安全关键项的出处**文字里自称**「与另一来源一致/交叉印证」，\
             但库里没有第二条结构化记录。散文里的印证机器核不了，也没法被冲突检测保护 —— \
             要算数就把第二来源单独入库成一条 verified 记录。"));
    }
    Ok(Report { vehicle_id: v.id.clone(), critical_total, corroborated, single_source: single, all_docs, notes })
}

pub fn render(r: &Report) -> String {
    let mut o = format!("交叉验证（G4）  车辆 {}\n", r.vehicle_id);
    o.push_str(&"─".repeat(60));
    o.push_str(&format!("\n安全关键项 {} 条：已交叉验证 {} · **单来源 {}**\n",
        r.critical_total, r.corroborated.len(), r.single_source.len()));

    o.push_str("\n【全库独立来源文档】\n");
    for (d, n) in &r.all_docs { o.push_str(&format!("  · {d}  （{n} 条规格）\n")); }

    for n in &r.notes { o.push_str(&format!("\n{n}\n")); }

    if !r.corroborated.is_empty() {
        o.push_str("\n【✅ 已交叉验证 —— 两个以上独立文档】\n");
        for it in &r.corroborated {
            o.push_str(&format!("  {} = {}\n      来源 {}\n",
                it.canonical, it.value, it.docs.join(" + ")));
        }
    }
    if !r.single_source.is_empty() {
        o.push_str("\n【⚠️ 单来源 —— 安全关键但只有一个文档背书】\n");
        o.push_str("  值本身没问题，问题是没人给它背书。安全关键项拧下去之前，\n  \
建议先回原图核一次，或找第二个独立来源。\n\n");
        for it in &r.single_source {
            o.push_str(&format!("  · {} = {}{}\n      {} （{}）\n",
                it.canonical, it.value,
                if it.claims_only { "   ← 出处自称有印证，但库里没有第二条记录" } else { "" },
                it.docs.join(" "), it.items.join(", ")));
        }
    }
    o.push_str("\nℹ️ 单来源**不扣下值**（那会把整个档案清空）——\n   \
这是披露，不是拦截。要拦截的是冲突，那类已经由 specs_usable 扣下了。\n");
    o
}


/// 给作业单用的轻量接口：一次算出「安全关键且单来源」的 canonical 集合。
///
/// 放在这里而不是 workbench 里，是为了**只有一份判定逻辑** ——
/// 关键词表、出版方归一化、档案里的补充/豁免，三处共用同一套。
/// 分成两份迟早会漂移，而漂移的那天没人会发现。
pub fn single_source_criticals(p: &Platform, conn: &Connection, vehicle_id: &str,
                               doc: &serde_yaml_ng::Value)
    -> Result<std::collections::HashSet<String>>
{
    let extra  = str_list(doc, "safety_critical_extra");
    let exempt = str_list(doc, "safety_critical_exempt");
    let registry = load_registry(p);
    let mut st = conn.prepare(
        "SELECT canonical, item, COALESCE(source_ref,'') FROM specs_usable WHERE vehicle_id=?1")?;
    let rows = st.query_map([vehicle_id], |r| Ok((
        r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;

    use std::collections::{HashMap, HashSet};
    let mut docs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut crit: HashSet<String> = HashSet::new();
    for row in rows {
        let (canon, item, sref) = row?;
        if !is_critical(&item, &canon, &extra, &exempt) { continue; }
        crit.insert(canon.clone());
        let d = doc_id_with(&registry, &sref);
        if !d.is_empty() { docs.entry(canon).or_default().insert(d); }
    }
    Ok(crit.into_iter()
        .filter(|c| docs.get(c).map(|s| s.len() < 2).unwrap_or(true))
        .collect())
}
