//! 问答(RAG) —— soma / 其他 agent / 人 问一句,答案**只锚在已核实数据上**
//!
//! 这一步最大的风险:LLM 会自信地编扭力值,哪怕系统提示叫它别编。
//! 所以答案分两层,永不混:
//!   1. facts:检索到的**已核实规格 + 手册出处**,原样带页码。这是硬事实,不经模型。
//!   2. answer:模型把这些事实串成人话,**明确标为「生成」**。
//! 即使模型把数字说错,facts 就在旁边、带 A/B 级出处和页码,人一眼能对。
//!
//! 硬约束(与全系统一致):
//! - 注入模型的上下文**只含 specs_usable + 适用手册 + known_issues** —— C/D 进不来
//! - 命中冲突的规格,明确告诉模型「这个值有冲突,别给数」
//! - 检索不到相关事实时,**不调模型编答案** —— 直说不知道,给下一步

use crate::{api, db, platform::Platform, provider};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Fact {
    pub kind: String,          // spec / manual / issue / conflict
    pub text: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct AskResult {
    pub vehicle: String,
    pub question: String,
    pub facts: Vec<Fact>,      // 权威层:原样,带出处
    pub answer: Option<String>,// 生成层:模型把 facts 串成话;无模型/无事实时为 None
    pub answer_source: String, // 生成用的 provider,或说明为什么没生成
    pub reasoning: Option<String>,
    pub warnings: Vec<String>,
}


/// 中文问法 → 规格 item 英文关键词。用户中文问,规格 item 是英文键,靠这桥接。
fn expand_terms(p: &Platform, question: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let f = p.root.join("knowledge/规格别名.yml");
    if let Some(doc) = std::fs::read_to_string(&f)
        .ok().and_then(|t| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&t).ok()) {
        if let Some(m) = doc.get("aliases").and_then(|x| x.as_mapping()) {
            for (k, val) in m {
                let cn = k.as_str().unwrap_or("");
                if !cn.is_empty() && question.contains(cn) {
                    match val {
                        serde_yaml_ng::Value::String(en) => terms.push(en.clone()),
                        serde_yaml_ng::Value::Sequence(seq) => for e in seq {
                            if let Some(en) = e.as_str() { terms.push(en.to_string()); }
                        },
                        _ => {}
                    }
                }
            }
        }
    }
    // 问题里的英文词也直接用
    for w in question.split(|c: char| !c.is_ascii_alphanumeric()) {
        if w.len() >= 3 { terms.push(w.to_lowercase()); }
    }
    terms.sort(); terms.dedup();
    terms
}

/// 检索:从已核实规格 + 适用手册 + 已知故障里拉相关事实。全部走安全视图。
fn gather(p: &Platform, vehicle_id: &str, q: &str) -> Result<(Vec<Fact>, Vec<String>)> {
    let conn = db::open(&p.index_db())?;
    let mut facts = Vec::new();
    let mut warnings = Vec::new();
    let terms = expand_terms(p, q);

    // 1) 已核实规格(specs_usable —— C/D 不可能出现)。用中英展开后的关键词逐个查。
    let mut seen_items = std::collections::HashSet::new();
    let mut push_spec = |it: String, val: String, lvl: String, sref: String, page: i64,
                         facts: &mut Vec<Fact>| {
        if seen_items.insert(it.clone()) {
            facts.push(Fact { kind: "spec".into(), text: format!("{it} = {val}"),
                source: format!("{lvl}级 · {sref} p{page}") });
        }
    };
    for term in &terms {
        let like = format!("%{}%", term);
        let mut st = conn.prepare(
            "SELECT item, value, source_level, COALESCE(source_ref,''), COALESCE(page,0)
             FROM specs_usable WHERE vehicle_id=?1 AND lower(item) LIKE ?2 ORDER BY item LIMIT 12")?;
        let rows = st.query_map(rusqlite::params![vehicle_id, like], |r| Ok((
            r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?,
            r.get::<_,String>(3)?, r.get::<_,i64>(4)?))).ok();
        if let Some(rows) = rows {
            for (item, val, lvl, sref, page) in rows.flatten() {
                push_spec(item, val, lvl, sref, page, &mut facts);
            }
        }
        if facts.len() >= 14 { break; }
    }

    // 2) 冲突:相关项若有冲突,明确标注 —— 不给数
    for term in &terms {
        let like = format!("%{}%", term);
        let mut cst = conn.prepare(
            "SELECT canonical, values_csv FROM spec_conflicts
             WHERE vehicle_id=?1 AND (lower(canonical) LIKE ?2 OR lower(items_csv) LIKE ?2)")?;
        let crows = cst.query_map(rusqlite::params![vehicle_id, like],
            |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))).ok();
        if let Some(crows) = crows {
            for (canon, vals) in crows.flatten() {
                if seen_items.insert(format!("conflict:{canon}")) {
                    facts.push(Fact { kind: "conflict".into(),
                        text: format!("{canon} 有互相矛盾的值 [{vals}] —— 未解决,不可给数"),
                        source: "冲突(已扣下)".into() });
                }
            }
        }
    }

    // 3) 适用手册片段(manual_pages_applicable —— 别代资料进不来)
    if let Ok(mut mst) = conn.prepare(
        "SELECT m.title, m.tier, m.page_no, snippet(manual_fts,0,'[',']','…',18)
         FROM manual_fts f JOIN manual_pages_applicable m ON m.id=f.rowid
         WHERE manual_fts MATCH ?1 AND m.vehicle_id=?2 AND m.tier IN ('A','B')
         ORDER BY rank LIMIT 4") {
        // FTS 用问题里的词(去标点)
        let terms: String = q.split(|c: char| !c.is_alphanumeric()
                && !('\u{4e00}'..='\u{9fff}').contains(&c))
            .filter(|w| w.chars().count() >= 2).collect::<Vec<_>>().join(" OR ");
        if !terms.is_empty() {
            if let Ok(rows) = mst.query_map(rusqlite::params![terms, vehicle_id], |r| Ok((
                r.get::<_,String>(0)?, r.get::<_,Option<String>>(1)?,
                r.get::<_,i64>(2)?, r.get::<_,String>(3)?))) {
                for (title, tier, page, snip) in rows.flatten() {
                    facts.push(Fact { kind: "manual".into(),
                        text: snip.replace('\n', " "),
                        source: format!("{}级 · {} p{}", tier.unwrap_or("?".into()), title, page) });
                }
            }
        }
    }

    // 4) 已知故障(读 VEHICLE.yml)
    if let Ok(ki) = api::known_issues(p, Some(vehicle_id)) {
        if let Some(arr) = ki.as_array() {
            for e in arr {
                let item = e.get("item").and_then(|x| x.as_str()).unwrap_or("");
                let syms: Vec<String> = e.get("symptoms").and_then(|x| x.as_array())
                    .map(|l| l.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let hay = format!("{item}{}", syms.join("")).to_lowercase();
                let ql = q.to_lowercase();
                let hit = ql.split(|c: char| !c.is_alphanumeric()
                        && !('\u{4e00}'..='\u{9fff}').contains(&c))
                    .filter(|w| w.chars().count() >= 2)
                    .any(|w| hay.contains(&w.to_lowercase()));
                if hit {
                    facts.push(Fact { kind: "issue".into(),
                        text: format!("{item}（症状:{}）", syms.join("、")),
                        source: format!("{}级 · NHTSA",
                            e.get("level").and_then(|x| x.as_str()).unwrap_or("?")) });
                }
            }
        }
    }

    if facts.is_empty() {
        warnings.push("没检索到相关的已核实事实 —— 不会让模型凭空编答案".into());
    }
    Ok((facts, warnings))
}

pub fn ask(p: &Platform, vehicle: Option<&str>, question: &str, provider_id: Option<&str>)
    -> Result<AskResult>
{
    let v = p.resolve_vehicle(vehicle)?;
    let (facts, mut warnings) = gather(p, &v.id, question)?;

    // 无事实 → 不调模型。宁可说不知道,也不生成可能错的答案。
    if facts.is_empty() {
        return Ok(AskResult {
            vehicle: v.id, question: question.into(), facts,
            answer: None,
            answer_source: "未调用模型:没有可依据的已核实事实".into(),
            reasoning: None,
            warnings: vec![
                "这不等于「没问题」,只说明本地已核实数据里没有相关内容。".into(),
                "可试:repair.manual_search 查全文、repair.symptom 走症状、repair.research.brief 拿检索提纲。".into(),
            ],
        });
    }

    // 有事实 → 注入受限上下文调模型
    let (provs, default) = provider::load(&p.root)?;
    let chosen = provider::pick(&provs, provider_id, &default)?;
    let context = facts.iter().map(|f| format!("- [{}|{}] {}", f.kind, f.source, f.text))
        .collect::<Vec<_>>().join("\n");
    let system = format!(
        "你是 {} 的维修顾问。**只能依据下面提供的已核实事实回答,不得引入任何其他数字或规格。**\n\
         规则:\n\
         1. 扭力值、间隙、容量等具体数字**只能照抄事实里的**,一个字都不能改,也不能补充事实里没有的数字。\n\
         2. 若某项标了「有互相矛盾的值/冲突」,**明确告诉用户该值未定、不可使用**,绝不替它选一个。\n\
         3. 事实不足以回答时,直说「已核实数据里没有这部分」,不要猜。\n\
         4. 简洁,中文。\n\n\
         已核实事实:\n{}", v.id, context);

    match provider::ask(&chosen, &system, question) {
        Ok(a) => {
            warnings.extend(a.warnings);
            Ok(AskResult {
                vehicle: v.id, question: question.into(), facts,
                answer: if a.text.trim().is_empty() { None } else { Some(a.text) },
                answer_source: format!("{} · {}", a.provider, a.model),
                reasoning: a.reasoning, warnings,
            })
        }
        Err(e) => {
            // 模型挂了不致命 —— 事实层照样给,答案层缺席并说明
            warnings.push(format!("模型调用失败:{e:#}。下面的已核实事实仍然可用。"));
            Ok(AskResult {
                vehicle: v.id, question: question.into(), facts,
                answer: None,
                answer_source: format!("模型不可用({})", chosen.id),
                reasoning: None, warnings,
            })
        }
    }
}

pub fn render(r: &AskResult) -> String {
    let mut o = format!("问:{}\n车辆:{}\n\n", r.question, r.vehicle);
    if !r.facts.is_empty() {
        o.push_str("━━━ 已核实事实(权威 · 原样带出处)━━━\n");
        for f in &r.facts {
            let mark = if f.kind == "conflict" { "⚠️ " } else { "" };
            o.push_str(&format!("{}{}\n    [{}]\n", mark, f.text, f.source));
        }
        o.push('\n');
    }
    match &r.answer {
        Some(a) => {
            o.push_str(&format!("━━━ 顾问回答(由模型生成 · {})━━━\n{}\n", r.answer_source, a));
            o.push_str("\n⚠️ 上面这段是生成的。数字以「已核实事实」为准 —— 两者不一致时信事实。\n");
        }
        None => o.push_str(&format!("(无生成回答:{})\n", r.answer_source)),
    }
    for w in &r.warnings { o.push_str(&format!("· {w}\n")); }
    o
}
