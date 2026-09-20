//! 症状 → 原因推理 —— 诊断的最上游一环
//!
//! 边界(和 diag 排查树分工):
//!   symptom:「我只观察到一个现象」→ 按本车真实故障频率排出候选原因,导向排查树
//!   diag:   「已知是充电问题」→ 一步步定位
//!
//! 纪律:候选**不是我拍的**。每条来自 known_issues(其 symptoms/complaints/level
//! 都在 VEHICLE.yml 里声明),频次是 NHTSA 投诉真实计数,排序由数据决定。
//! 匹配不到就说匹配不到 —— 不硬凑一个原因。

use crate::platform::Platform;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Cause {
    pub item: String,
    pub level: String,          // A/C:证据强度
    pub complaints: i64,        // NHTSA 投诉计数(本车真实频次)
    pub matched_symptoms: Vec<String>,
    pub note: String,
    pub diag_tree: Option<String>,   // 有排查树就导过去
    /// 匹配强度:命中的症状词数(用于排序的次级键)
    pub match_score: usize,
}

#[derive(Debug, Serialize)]
pub struct SymptomResult {
    pub vehicle: String,
    pub query: String,
    pub causes: Vec<Cause>,
    pub note: String,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
}

/// 字符 bigram 集合。中文没有空格、口语爱夹「有/的/了」,
/// 靠分词或子串都脆(「后轮有异响」≠「后轮异响」)。
/// bigram 重叠对这类噪声稳健:两者共享「后轮」「异响」。
fn bigrams(s: &str) -> std::collections::HashSet<String> {
    let cs: Vec<char> = norm(s).chars().collect();
    let mut out = std::collections::HashSet::new();
    if cs.len() == 1 { out.insert(cs[0].to_string()); }
    for w in cs.windows(2) { out.insert(w.iter().collect()); }
    out
}

/// 一个登记症状与用户输入是否算命中:
/// 子串(任一方向)命中,或 bigram 重叠达到该症状的一半以上。
fn hits(user: &str, ub: &std::collections::HashSet<String>, sym: &str) -> bool {
    let ns = norm(sym);
    let nu = norm(user);
    if nu.contains(&ns) || ns.contains(&nu) { return true; }
    let sb = bigrams(sym);
    if sb.is_empty() { return false; }
    let overlap = sb.iter().filter(|g| ub.contains(*g)).count();
    // 半数以上 bigram 命中 = 说的是同一件事。短症状(2-3字)要求全中。
    overlap * 2 >= sb.len()
}

pub fn diagnose(p: &Platform, vehicle: Option<&str>, symptom: &str) -> Result<SymptomResult> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let empty = Vec::new();
    let mut causes: Vec<Cause> = Vec::new();

    for e in doc.get("known_issues").and_then(|x| x.as_sequence()).unwrap_or(&empty) {
        let syms: Vec<String> = e.get("symptoms").and_then(|x| x.as_sequence())
            .map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        // 双向子串匹配:用户说的词 ⊂ 某症状,或某症状 ⊂ 用户说的
        let ub = bigrams(symptom);
        let matched: Vec<String> = syms.iter()
            .filter(|sy| hits(symptom, &ub, sy)).cloned().collect();
        if matched.is_empty() { continue; }
        causes.push(Cause {
            item: e.get("item").and_then(|x| x.as_str()).unwrap_or("").into(),
            level: e.get("level").and_then(|x| x.as_str()).unwrap_or("?").into(),
            complaints: e.get("complaints").and_then(|x| x.as_i64()).unwrap_or(0),
            matched_symptoms: matched.clone(),
            note: e.get("note").and_then(|x| x.as_str()).unwrap_or("").trim()
                .chars().take(160).collect(),
            diag_tree: e.get("diag_tree").and_then(|x| x.as_str()).map(String::from),
            match_score: matched.len(),
        });
    }

    // 排序:先按 A 级优先(证据强),再按投诉频次(本车真实多发),再按匹配词数
    causes.sort_by(|a, b| {
        let la = if a.level == "A" { 0 } else { 1 };
        let lb = if b.level == "A" { 0 } else { 1 };
        la.cmp(&lb)
            .then(b.complaints.cmp(&a.complaints))
            .then(b.match_score.cmp(&a.match_score))
    });

    let note = if causes.is_empty() {
        format!("「{symptom}」没匹配到本车已登记的故障模式。\
                 这**不等于没问题** —— 只说明它不在 30 条 NHTSA 投诉归纳出的已知模式里。\
                 可 repair.manual_search 查手册,或 repair.diag 看现有排查树。")
    } else {
        "候选按证据强度(A级优先)+ 本车投诉频次排序。频次是 NHTSA 真实计数,不是猜。".into()
    };
    Ok(SymptomResult { vehicle: v.id, query: symptom.into(), causes, note })
}

pub fn render(r: &SymptomResult) -> String {
    let mut o = format!("症状:{}\n车辆:{}\n\n", r.query, r.vehicle);
    if r.causes.is_empty() {
        o.push_str(&r.note);
        o.push('\n');
        return o;
    }
    o.push_str(&format!("可能的原因({}个,按可能性排序):\n\n", r.causes.len()));
    for (i, c) in r.causes.iter().enumerate() {
        o.push_str(&format!("{}. [{}级 · NHTSA {} 条投诉] {}\n",
            i+1, c.level, c.complaints, c.item));
        o.push_str(&format!("   命中症状:{}\n", c.matched_symptoms.join("、")));
        if !c.note.is_empty() { o.push_str(&format!("   {}\n", c.note)); }
        match &c.diag_tree {
            Some(t) => o.push_str(&format!("   → 有排查树:repair.diag {t}\n")),
            None => o.push_str("   → 暂无专门排查树;先按上面的说明查\n"),
        }
        o.push('\n');
    }
    o.push_str(&format!("{}\n", r.note));
    o
}
