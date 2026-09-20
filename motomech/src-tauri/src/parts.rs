//! 零件与采购清单 —— 回答「今天必须下单的是哪几样」
//!
//! 倒排的核心：最晚下单日 = 该阶段预计开工日 − 到货周期。
//!
//! 「每阶段预计天数」是个**假设**，不是查来的事实。所以它是命令行参数、
//! 而且每次输出都把用到的值印在最上面 —— 藏在代码里的常数会被当成事实读，
//! 印出来的假设才会被质疑和修正。

use crate::platform::{Platform, Vehicle};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct Part {
    pub id: String,
    pub name: String,
    pub spec: Option<String>,
    pub spec_ref: Option<String>,
    pub part_number: Option<String>,
    pub part_number_level: Option<String>,
    pub qty: i64,
    pub lead_time_days: i64,
    pub needed_phase: String,
    pub status: String,
    pub why: String,
    pub blocked_by: Option<String>,
    /// 等你拍板（选型偏好）—— 与 blocked_by 是两回事
    pub needs_choice: Option<String>,
    pub est_cost: Option<f64>,
    /// 距今还有多少天必须下单。负数 = 已经晚了
    pub order_within_days: i64,
    pub urgency: String,
}

#[derive(Debug, Serialize)]
pub struct Blocker {
    pub id: String, pub what: String, pub why: String,
    pub phase: String, pub unblocks: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PartsPlan {
    pub vehicle_id: String,
    pub current_phase: String,
    /// 印出来的假设：每阶段预计天数
    pub assumed_days_per_phase: i64,
    pub parts: Vec<Part>,
    pub blockers: Vec<Blocker>,
    pub est_total: f64,
    pub unknown_cost_count: usize,
}

fn phase_ord(p: &str) -> i64 { p.trim_start_matches(['P','p']).parse().unwrap_or(0) }

fn get_str(v: &serde_yaml_ng::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn plan(p: &Platform, vehicle: Option<&str>, days_per_phase: i64) -> Result<PartsPlan> {
    let v: Vehicle = p.resolve_vehicle(vehicle)?;
    let f = v.dir.join("50_零件/采购清单.yml");
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(&f).with_context(|| format!("读不到 {}", f.display()))?)?;

    let cur = std::fs::read_to_string(v.dir.join("PROGRESS.txt")).ok()
        .map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        .unwrap_or_else(|| "P1".into());
    let cur_ord = phase_ord(&cur);

    let empty = Vec::new();
    let mut parts: Vec<Part> = doc.get("items").and_then(|x| x.as_sequence()).unwrap_or(&empty)
        .iter().map(|e| {
            let needed = get_str(e, "needed_phase").unwrap_or_else(|| "P9".into());
            let lead = e.get("lead_time_days").and_then(|x| x.as_i64()).unwrap_or(0);
            // 距该阶段开工还有多少天（按假设的每阶段天数）
            let days_to_phase = (phase_ord(&needed) - cur_ord).max(0) * days_per_phase;
            let within = days_to_phase - lead;
            let blocked = get_str(e, "blocked_by");
            let choice = get_str(e, "needs_choice");
            let status = get_str(e, "status").unwrap_or_else(|| "未订".into());
            let urgency = if status == "已到" || status == "不需要" { "完成".to_string() }
                else if blocked.is_some() { "被挡".to_string() }
                else if choice.is_some() && within <= 14 { "待拍板".to_string() }
                else if status == "已订" { "在途".to_string() }
                else if within <= 0 { "立即".to_string() }
                else if within <= 7 { "本周".to_string() }
                else { "稍后".to_string() };
            Part {
                id: get_str(e, "id").unwrap_or_default(),
                name: get_str(e, "name").unwrap_or_default(),
                spec: get_str(e, "spec"),
                spec_ref: get_str(e, "spec_ref"),
                part_number: get_str(e, "part_number"),
                part_number_level: get_str(e, "part_number_level"),
                qty: e.get("qty").and_then(|x| x.as_i64()).unwrap_or(1),
                lead_time_days: lead,
                needed_phase: needed, status,
                why: get_str(e, "why").unwrap_or_default(),
                blocked_by: blocked, needs_choice: choice,
                est_cost: e.get("est_cost").and_then(|x| x.as_f64()),
                order_within_days: within, urgency,
            }
        }).collect();

    // 倒排：最晚下单日越早越靠前；同样紧急时，到货周期长的先下单
    let rank = |u: &str| match u {
        "立即"=>0, "本周"=>1, "待拍板"=>2, "被挡"=>3, "稍后"=>4, "在途"=>5, _=>6 };
    parts.sort_by(|a, b| rank(&a.urgency).cmp(&rank(&b.urgency))
        .then(a.order_within_days.cmp(&b.order_within_days))
        .then(b.lead_time_days.cmp(&a.lead_time_days)));

    let blockers: Vec<Blocker> = doc.get("blockers_for_user").and_then(|x| x.as_sequence())
        .unwrap_or(&empty).iter().map(|e| Blocker {
            id: get_str(e, "id").unwrap_or_default(),
            what: get_str(e, "what").unwrap_or_default(),
            why: get_str(e, "why").unwrap_or_default(),
            phase: get_str(e, "phase").unwrap_or_default(),
            unblocks: e.get("unblocks").and_then(|x| x.as_sequence())
                .map(|l| l.iter().filter_map(|u| u.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        }).collect();

    let est_total: f64 = parts.iter().filter_map(|p| p.est_cost).sum();
    let unknown = parts.iter().filter(|p| p.est_cost.is_none() && p.status != "不需要").count();

    Ok(PartsPlan { vehicle_id: v.id, current_phase: cur,
        assumed_days_per_phase: days_per_phase, parts, blockers, est_total,
        unknown_cost_count: unknown })
}

pub fn render(pp: &PartsPlan) -> String {
    let mut o = format!("车辆 {} · 当前阶段 {}\n", pp.vehicle_id, pp.current_phase);
    o.push_str(&format!("假设：每阶段约 {} 天（这是假设不是事实，用 --days-per-phase 改）\n\n",
        pp.assumed_days_per_phase));

    let icon = |u: &str| match u {
        "立即"=>"🔴", "本周"=>"🟡", "待拍板"=>"🤔", "被挡"=>"🚧",
        "在途"=>"🚚", "完成"=>"✅", _=>"⚪" };
    let mut last = String::new();
    for p in &pp.parts {
        if p.urgency != last {
            let hdr = match p.urgency.as_str() {
                "立即" => "── 🔴 现在就下单（再等就卡住关键路径）──",
                "本周" => "── 🟡 本周内下单 ──",
                "待拍板" => "── 🤔 等你拍板：规格是确定的，选型是你的偏好 ──",
                "被挡" => "── 🚧 下不了单：等你先做完一件事 ──",
                "在途" => "── 🚚 已订，在途 ──",
                "完成" => "── ✅ 已到 / 不需要 ──",
                _      => "── ⚪ 稍后 ──",
            };
            o.push_str(&format!("\n{hdr}\n"));
            last = p.urgency.clone();
        }
        o.push_str(&format!("{} {} ×{}", icon(&p.urgency), p.name, p.qty));
        if let Some(s) = &p.spec { o.push_str(&format!("  [{s}]")); }
        o.push_str(&format!("\n   {} 用 · 到货约 {} 天", p.needed_phase, p.lead_time_days));
        if p.urgency == "立即" || p.urgency == "本周" {
            o.push_str(&format!(" · 最晚 {} 天内下单", p.order_within_days.max(0)));
        }
        if let Some(c) = p.est_cost { o.push_str(&format!(" · 约 ${c:.0}")); }
        o.push('\n');
        if let Some(r) = &p.spec_ref { o.push_str(&format!("   规格出处：{r}\n")); }
        if p.spec.is_some() && p.spec_ref.is_none() {
            o.push_str("   ⚠️ 有规格但无出处 —— 不要据此下单\n");
        }
        if let Some(c) = &p.needs_choice {
            o.push_str(&format!("   🤔 {}\n", c.trim().replace('\n', "\n      ")));
        }
        if let Some(b) = &p.blocked_by {
            o.push_str(&format!("   🚧 {}\n", b.trim().replace('\n', "\n      ")));
        }
    }

    if !pp.blockers.is_empty() {
        o.push_str("\n\n═══ 只有你能做的事（挡住了上面的采购）═══\n");
        for b in &pp.blockers {
            o.push_str(&format!("\n☐ [{}] {}\n   为什么：{}\n", b.phase, b.what, b.why));
            if !b.unblocks.is_empty() {
                o.push_str(&format!("   做完解锁：{}\n", b.unblocks.join("、")));
            }
        }
    }
    o.push_str(&format!("\n\n已知价格合计约 ${:.0}；另有 {} 项价格未知（多数是件号还没查到）。\n",
        pp.est_total, pp.unknown_cost_count));
    o
}
