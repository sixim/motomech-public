//! 诊断排查树 —— 症状 → 测量点 → 判据 → 分叉 → 原因
//!
//! 填的是 known_issues(会怎么坏)与 spec(数值)之间那一环。
//!
//! 两条纪律:
//! 1. 判据里引用 `spec:<canonical>` 的,渲染时替换成**已核实的实际值** ——
//!    树不重复抄数字,否则会和规格库不一致(监工反复抓的那类)。
//!    若该 canonical 正处于冲突(被扣下),明确标注「判据依赖的值当前有冲突」。
//! 2. 标 gated 的步骤涉及拆装,附安全门禁提醒 —— 不在这里放行,只提示去过 safety_check。

use crate::{db, platform::Platform};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Step {
    pub id: String,
    pub check: String,
    pub expect: String,
    pub basis: String,
    /// basis 引用的已核实值(若有);None 表示纯手册判据或该值缺失/冲突
    pub resolved_value: Option<String>,
    pub value_status: String,   // "" | "已核实" | "冲突-已扣下" | "缺失"
    /// basis 写 `circuit:<PDF页>` 时，解析出的页图路径与适用性。
    /// 万用表测量点最需要的就是「这根线在图上哪儿」——
    /// 而电路图**按年份分档**，指错档比不给还危险，所以这里要判适用性。
    pub circuit_page: Option<i64>,
    pub circuit_image: Option<String>,
    pub circuit_note: String,
    pub if_pass: Option<String>,
    pub if_fail: Option<String>,
    pub next: Option<String>,
    pub conclusion_on_fail: Option<String>,
    pub note: Option<String>,
    pub gated: bool,
}

#[derive(Debug, Serialize)]
pub struct Tree {
    pub id: String,
    pub symptom: String,
    pub severity: String,
    pub applies: bool,          // 本树代际是否匹配本车
    pub applies_note: String,
    pub steps: Vec<Step>,
    pub cross_check: Vec<String>,
    pub related_issues: Vec<String>,
}

fn sv(v: &serde_yaml_ng::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
fn slist(v: &serde_yaml_ng::Value, k: &str) -> Vec<String> {
    v.get(k).and_then(|x| x.as_sequence())
        .map(|l| l.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).collect())
        .unwrap_or_default()
}

/// 把 basis 里的 `spec:<key>` 解析成实际已核实值。
///
/// **解析本身不在这里** —— 委派 `specref::resolve`，全项目唯一一份。
/// 此前这里自己写了一份，于是 2026-09-16 在 workbench 修好的
/// 「冲突退化成缺失」bug，2026-09-19 在这里原样复活。
fn resolve_spec(conn: &rusqlite::Connection, vehicle: &str, basis: &str)
    -> (Option<String>, String)
{
    let key = match basis.strip_prefix("spec:") {
        Some(c) => c.trim(),
        None => return (None, String::new()),   // 非 spec 引用（手册直引）
    };
    match crate::specref::resolve(conn, vehicle, key) {
        crate::specref::SpecState::Verified { value, .. } => (Some(value), "已核实".into()),
        crate::specref::SpecState::Conflicted { .. } => (None, "冲突-已扣下".into()),
        crate::specref::SpecState::Missing => (None, "缺失".into()),
    }
}

/// 把 basis 里的 `circuit:<页>` 解析成页图路径。
///
/// 为什么要顺手判适用性：同一台 GS 有 2004–2006 / 2007 / 2008-on 三套**不同**的电路图。
/// 排查树里写一个页号，如果那页不是本车年款的，人照着它去背刺插头
/// 量的是另一代车的线 —— 比不给图更危险。所以对不上就**不给图**，并说清为什么。
fn resolve_circuit(p: &Platform, v: &crate::platform::Vehicle, conn: &rusqlite::Connection,
                   basis: &str) -> (Option<i64>, Option<String>, String) {
    let page: i64 = match basis.strip_prefix("circuit:").and_then(|x| x.trim().parse().ok()) {
        Some(n) => n, None => return (None, None, String::new()),
    };
    // 这一页在不在本车适用的电路图组里
    let applicable = crate::circuit::build(p, v, None, false).ok()
        .map(|r| r.groups.iter().any(|g| g.applicable && g.pages.contains(&page)))
        .unwrap_or(false);
    if !applicable {
        return (Some(page), None, format!(
            "⚠️ p{page} 不在本车适用的电路图里（电路图按年份分档）——\
             **不给图**，照别的年款的图去量线比没有图更危险。用 `circuit` 查本车该看哪页。"));
    }
    let img: Option<String> = conn.query_row(
        "SELECT mp.image_path FROM manual_pages mp JOIN manuals m ON m.id=mp.manual_id \
         WHERE m.vehicle_id=?1 AND mp.page_no=?2 AND mp.image_path IS NOT NULL LIMIT 1",
        rusqlite::params![&v.id, page], |r| r.get(0)).ok();
    (Some(page), img, String::new())
}

fn inline_specs(conn: &rusqlite::Connection, vehicle: &str, text: &str) -> String {
    if !text.contains("spec:") { return text.to_string(); }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("spec:") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 5..];
        // canonical 允许字母数字、点、下划线、连字符
        let end = tail.find(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_' || c == '-'))
                      .unwrap_or(tail.len());
        let canon = &tail[..end];
        out.push_str(&match crate::specref::resolve(conn, vehicle, canon) {
            crate::specref::SpecState::Verified { ref value, ref item, .. } =>
                format!("{}{}", value, crate::specref::unit_suffix(item, value)),
            crate::specref::SpecState::Conflicted { .. } =>
                format!("〔{canon}：**多个来源矛盾，值已扣下，不可用**〕"),
            crate::specref::SpecState::Missing =>
                format!("〔{canon}：档案里还没有这个已核实值〕"),
        });
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

pub fn load(p: &Platform, vehicle: Option<&str>, tree_id: &str) -> Result<Tree> {
    let v = p.resolve_vehicle(vehicle)?;
    let dir = p.root.join("knowledge/排查树");
    let f = dir.join(format!("{tree_id}.yml"));
    // 允许用 id 或文件名找
    let path = if f.exists() { f } else {
        std::fs::read_dir(&dir)?.flatten()
            .map(|e| e.path())
            .find(|pp| pp.extension().map(|x| x=="yml").unwrap_or(false)
                && std::fs::read_to_string(pp).ok()
                    .and_then(|s| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&s).ok())
                    .and_then(|d| sv(&d, "id")).as_deref() == Some(tree_id))
            .with_context(|| format!("没有排查树:{tree_id}"))?
    };
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&std::fs::read_to_string(&path)?)?;
    let conn = db::open(&p.index_db())?;

    // 代际匹配:树声明的 applies_to_generations 要含本车 engine_code
    let vdoc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    // 代际判定同样委派 applicability::judge —— 全项目唯一一份。
    // 树没声明 applies_to_generations = 不受代际限制（放行）；
    // 声明了但本车没有 engine_code = **判不了，不放行**。这两件事不能混。
    let gens = slist(&doc, "applies_to_generations");
    let verdict = crate::applicability::judge(
        &crate::applicability::ident_from(&vdoc),
        &crate::applicability::Declares { generations: gens.clone(), ..Default::default() });
    let applies = verdict.ok();
    let applies_note = if applies { String::new() } else {
        format!("⚠️ {} —— 判据可能不适用,仅供参考", verdict.reason())
    };

    let empty = Vec::new();
    let steps = doc.get("steps").and_then(|x| x.as_sequence()).unwrap_or(&empty).iter().map(|st| {
        let basis = sv(st, "basis").unwrap_or_default();
        let (resolved_value, value_status) = resolve_spec(&conn, &v.id, &basis);
        let il = |t: Option<String>| t.map(|x| inline_specs(&conn, &v.id, &x));
        let (circuit_page, circuit_image, circuit_note) = resolve_circuit(p, &v, &conn, &basis);
        Step {
            id: sv(st, "id").unwrap_or_default(),
            check: inline_specs(&conn, &v.id, &sv(st, "check").unwrap_or_default()),
            expect: inline_specs(&conn, &v.id, &sv(st, "expect").unwrap_or_default()),
            basis, circuit_page, circuit_image, circuit_note,
            resolved_value, value_status,
            if_pass: il(sv(st, "if_pass")), if_fail: il(sv(st, "if_fail")),
            next: sv(st, "next"),
            conclusion_on_fail: sv(st, "conclusion_on_fail"),
            note: il(sv(st, "note")),
            gated: st.get("gated").and_then(|x| x.as_bool()).unwrap_or(false),
        }
    }).collect();

    Ok(Tree {
        id: sv(&doc, "id").unwrap_or_default(),
        symptom: sv(&doc, "symptom").unwrap_or_default(),
        severity: sv(&doc, "severity").unwrap_or_default(),
        applies, applies_note, steps,
        cross_check: slist(&doc, "cross_check"),
        related_issues: slist(&doc, "related_issues"),
    })
}

pub fn list(p: &Platform) -> Result<Vec<(String, String)>> {
    let dir = p.root.join("knowledge/排查树");
    if !dir.is_dir() { return Ok(vec![]); }
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir)?.flatten() {
        let pp = e.path();
        if pp.extension().map(|x| x=="yml").unwrap_or(false) {
            if let Ok(d) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(
                &std::fs::read_to_string(&pp)?) {
                if let (Some(id), Some(sym)) = (sv(&d,"id"), sv(&d,"symptom")) {
                    out.push((id, sym));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn render(t: &Tree) -> String {
    let mut o = format!("排查树 · {}\n症状:{}\n严重度:{}\n", t.id, t.symptom, t.severity);
    if !t.applies { o.push_str(&format!("\n{}\n", t.applies_note)); }
    for iss in &t.related_issues { o.push_str(&format!("· {iss}\n")); }
    o.push('\n');
    for (i, s) in t.steps.iter().enumerate() {
        o.push_str(&format!("【步骤 {}】{}{}\n", i+1, s.check,
            if s.gated { "  ⚠️需过安全门禁" } else { "" }));
        // 判据:优先显示已核实实际值
        match s.value_status.as_str() {
            "已核实" => o.push_str(&format!("  判据:{}（已核实值:{}）\n",
                s.expect, s.resolved_value.clone().unwrap_or_default())),
            "冲突-已扣下" => o.push_str(&format!(
                "  判据:{} ⚠️ **它依赖的规格当前有冲突、已被扣下 —— 先解决冲突再用这一步**\n", s.expect)),
            "缺失" => o.push_str(&format!(
                "  判据:{} ⚠️ 依赖的规格还没核实到\n", s.expect)),
            _ => o.push_str(&format!("  判据:{}\n", s.expect)),
        }
        o.push_str(&format!("  出处:{}\n", s.basis));
        if let Some(pg) = s.circuit_page {
            match &s.circuit_image {
                Some(img) => o.push_str(&format!("  电路图:p{pg}  {img}\n")),
                None => o.push_str(&format!("  {}\n", s.circuit_note)),
            }
        }
        if let Some(n) = &s.note { o.push_str(&format!("  说明:{}\n", n)); }
        if let Some(x) = &s.if_pass { o.push_str(&format!("  ✓ 通过:{}\n", x)); }
        if let Some(x) = &s.if_fail { o.push_str(&format!("  ✗ 不通过:{}\n", x)); }
        if let Some(x) = &s.conclusion_on_fail { o.push_str(&format!("  → 结论:{}\n", x)); }
        if s.gated {
            o.push_str("  (动手拆装前先 repair.safety_check 你要做的动作)\n");
        }
        o.push('\n');
    }
    for c in &t.cross_check { o.push_str(&format!("⚠️ 交叉提醒:{c}\n")); }
    o
}
