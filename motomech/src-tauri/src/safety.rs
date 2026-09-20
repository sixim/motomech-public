//! 安全门禁判定器 —— 把「展示用的门禁文本」变成「能对一个动作下判决」
//!
//! 设计要点：
//! 1. **确定性**：纯规则匹配，不经过任何模型。同样的输入永远同样的判决。
//! 2. **fail-closed**：前置条件无法证明已满足时，门禁保持关闭。
//!    这跟代码里别处的习惯相反 —— 但在安全上，「不确定」必须等同于「不行」。
//! 3. **BLOCK 不返回执行步骤**：判决里只有「为什么不行」和「怎样才算可以」，
//!    没有任何一句是操作指引。MCP 调用方拿不到，因为根本没生成。

use crate::platform::{Platform, Vehicle};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub enum Level { Block, Gate, Warn }

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self { Level::Block => "BLOCK", Level::Gate => "GATE", Level::Warn => "WARN" }
    }
    fn rank(&self) -> u8 { match self { Level::Block => 3, Level::Gate => 2, Level::Warn => 1 } }
}

#[derive(Debug, Serialize)]
pub struct Verdict {
    pub level: String,
    pub rule_id: String,
    pub reason: String,
    /// 怎样才算可以 —— 注意这是「解锁条件」不是「操作步骤」
    pub unblock: String,
    pub basis: String,
    /// 需要逐项确认的清单（GATE 用）
    pub confirm: Vec<String>,
    /// 命中的关键词，让调用方看得见为什么被判中
    pub matched: String,
}

#[derive(Debug, Serialize)]
pub struct SafetyResult {
    pub vehicle_id: String,
    pub action: String,
    pub current_phase: String,
    /// 最高级别：BLOCK / GATE / WARN / OK
    pub verdict: String,
    /// 是否允许返回执行步骤。BLOCK 时为 false —— 这是给调用方的硬开关
    pub may_return_steps: bool,
    pub findings: Vec<Verdict>,
    /// 判定时无法证明已满足、因而按 fail-closed 处理的前置条件
    pub assumed_unmet: Vec<String>,
    /// 本车适用的规则条数（通用 + 专属）与专属条数
    pub rules_applicable: usize,
    pub rules_vehicle_specific: usize,
    /// 覆盖度提示 —— 一台没有专属规则的车，不能让人以为门禁覆盖了它
    pub coverage_note: Option<String>,
}

/// 关键词匹配 —— 与适用性过滤同一套办法。
/// CJK 按子串（中文没有词边界），ASCII 短词按词边界（否则 "can" 会命中 "cancel"）。
fn hit(hay_lower: &str, tokens: &[&str], needle: &str) -> bool {
    let n = needle.to_lowercase();
    if n.is_empty() { return false; }
    if n.is_ascii() && n.len() < 4 {
        tokens.iter().any(|w| *w == n)
    } else {
        hay_lower.contains(&n)
    }
}

fn phase_ord(p: &str) -> i32 {
    p.trim_start_matches(['P', 'p']).parse::<i32>().unwrap_or(-1)
}

pub fn evaluate(p: &Platform, vehicle: Option<&str>, action: &str) -> Result<SafetyResult> {
    let v: Vehicle = p.resolve_vehicle(vehicle)?;
    let rules_path = p.root.join("knowledge/安全门禁/规则表.yml");
    let raw = std::fs::read_to_string(&rules_path)
        .with_context(|| format!("读不到规则表: {}", rules_path.display()))?;
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)?;

    let cur = std::fs::read_to_string(v.dir.join("PROGRESS.txt")).ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "P1".to_string());
    let cur_ord = phase_ord(&cur);

    // 未执行的召回 —— 在 open_recalls 里就等于「未执行」
    let vdoc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    // **与 workbench 用同一个解析器** —— 两处各写各的，就会对同一个文件给出相反答案
    let (recalls, recall_problems) = crate::workbench::parse_recalls(&vdoc);
    let open_recalls: Vec<String> = recalls.into_iter().map(|r| r.id).collect();

    let low = action.to_lowercase();
    let tokens: Vec<&str> = low.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty()).collect();

    let mut findings: Vec<Verdict> = Vec::new();
    let (mut n_applicable, mut n_specific) = (0usize, 0usize);
    let mut assumed_unmet: Vec<String> = Vec::new();

    let empty = Vec::new();
    for r in doc.get("rules").and_then(|x| x.as_sequence()).unwrap_or(&empty) {
        let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let lvl = match r.get("level").and_then(|x| x.as_str()).unwrap_or("") {
            "BLOCK" => Level::Block, "GATE" => Level::Gate, "WARN" => Level::Warn,
            _ => continue,
        };
        // 作用域：声明了 vehicles 的规则只对那几台车成立
        if let Some(vs) = r.get("vehicles").and_then(|x| x.as_sequence()) {
            let ok = vs.iter().filter_map(|x| x.as_str()).any(|x| x == v.id);
            if !ok { continue; }
            n_specific += 1;
        }
        n_applicable += 1;

        let mut matched = String::new();
        if let Some(ms) = r.get("match").and_then(|x| x.as_sequence()) {
            for m in ms {
                if let Some(s) = m.as_str() {
                    if hit(&low, &tokens, s) { matched = s.to_string(); break; }
                }
            }
        }
        if matched.is_empty() { continue; }

        // 排除词：这条规则明确不管的语境。
        // 例：「燃油」本该触发汽油作业门禁，但「看一下燃油表读数」只是读数，
        // 要求先摆灭火器就是误报 —— 误报会训练人无视门禁，是真缺陷。
        if let Some(ex) = r.get("exclude_match").and_then(|x| x.as_sequence()) {
            if ex.iter().filter_map(|e| e.as_str()).any(|e| hit(&low, &tokens, e)) {
                continue;
            }
        }

        // ── 前置条件：满足则本条规则解除，否则保持拦住（fail-closed）──
        let mut released = false;
        if r.get("always").and_then(|x| x.as_bool()).unwrap_or(false) {
            released = false;   // 无条件成立，永不解除
        } else if let Some(need) = r.get("unless_phase_reached").and_then(|x| x.as_str()) {
            released = cur_ord >= phase_ord(need);
        } else if let Some(rc) = r.get("unless_recall_done").and_then(|x| x.as_str()) {
            if !recall_problems.is_empty() {
                // 召回档案读不干净 → **不能推断这条召回已执行**。
                // 「读不出来」不等于「没有」，对安全的含义是保持关闭。
                released = false;
                assumed_unmet.push(format!(
                    "召回档案有 {} 处读不出来，无法判断 {rc} 是否已执行 —— 按未执行处理",
                    recall_problems.len()));
            } else {
                released = !open_recalls.iter().any(|x| x == rc);
                if !released {
                    assumed_unmet.push(format!("召回 {rc} 仍在未执行列表中"));
                }
            }
        } else if let Some(t) = r.get("unless_task_done").and_then(|x| x.as_str()) {
            // 任务级完成状态尚未落地 —— 按 fail-closed 一律视为未完成，并如实声明
            released = false;
            assumed_unmet.push(format!(
                "任务「{t}」的完成状态目前无法核实（任务勾选尚未实现），按未完成处理"));
        }
        if released { continue; }

        findings.push(Verdict {
            level: lvl.as_str().to_string(),
            rule_id: id,
            reason: r.get("reason").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
            unblock: r.get("unblock").and_then(|x| x.as_str()).unwrap_or("").trim().to_string(),
            basis: r.get("basis").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            confirm: r.get("require_confirm").and_then(|x| x.as_sequence())
                .map(|l| l.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            matched,
        });
    }

    findings.sort_by(|a, b| level_of(&b.level).rank().cmp(&level_of(&a.level).rank()));
    let top = findings.first().map(|f| f.level.clone()).unwrap_or_else(|| "OK".into());
    let blocked = top == "BLOCK";

    Ok(SafetyResult {
        vehicle_id: v.id.clone(), action: action.to_string(), current_phase: cur,
        verdict: top, may_return_steps: !blocked, findings,
        assumed_unmet,
        rules_applicable: n_applicable,
        rules_vehicle_specific: n_specific,
        coverage_note: if n_specific == 0 {
            Some(format!("⚠️ 本车（{}）没有任何专属门禁规则，上面只用了通用规则。\
                          判定结果为 OK **不等于这台车安全** —— 只等于这张表还没覆盖它。\
                          它的召回、代际缺陷都还没写进规则表。", v.id))
        } else { None },
    })
}

fn level_of(s: &str) -> Level {
    match s { "BLOCK" => Level::Block, "GATE" => Level::Gate, _ => Level::Warn }
}

/// 给人看的渲染（CLI 与 MCP 共用，保证两边说的是同一句话）
pub fn render(r: &SafetyResult) -> String {
    let mut out = format!("车辆 {} · 当前阶段 {}\n动作：{}\n\n", r.vehicle_id, r.current_phase, r.action);
    if r.findings.is_empty() {
        out.push_str("✅ OK —— 规则表里没有命中任何门禁。\n\
                      注意：没命中不等于安全，只等于「这张表没覆盖」。\n");
        if let Some(c) = &r.coverage_note { out.push_str(&format!("\n{c}\n")); }
        return out;
    }
    let icon = |l: &str| match l { "BLOCK" => "⛔", "GATE" => "🚧", _ => "⚠️" };
    out.push_str(&format!("判决：{} {}\n\n", icon(&r.verdict), r.verdict));
    for f in &r.findings {
        out.push_str(&format!("{} {} [{}]  （命中「{}」）\n{}\n→ 怎样才算可以：{}\n出处：{}\n",
            icon(&f.level), f.level, f.rule_id, f.matched, f.reason, f.unblock, f.basis));
        if !f.confirm.is_empty() {
            out.push_str("需逐项确认：\n");
            for c in &f.confirm { out.push_str(&format!("  ☐ {c}\n")); }
        }
        out.push('\n');
    }
    if !r.may_return_steps {
        out.push_str("⛔ 本次判定为 BLOCK，**不提供执行步骤** —— 先解除上述条件。\n");
    }
    for a in &r.assumed_unmet { out.push_str(&format!("· fail-closed：{a}\n")); }
    if let Some(c) = &r.coverage_note { out.push_str(&format!("\n{c}\n")); }
    out
}


#[derive(Debug, Serialize)]
pub struct SelfTest { pub total: usize, pub passed: usize, pub failures: Vec<String> }

/// 跑回归用例。**过度拦截与漏拦同等对待** —— 期望 OK 的用例被判成 BLOCK 也算失败。
pub fn selftest(p: &Platform, vehicle: Option<&str>) -> Result<SelfTest> {
    let path = p.root.join("knowledge/安全门禁/判定用例.yml");
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(&path)
            .with_context(|| format!("读不到用例: {}", path.display()))?)?;
    let empty = Vec::new();
    let cases = doc.get("cases").and_then(|x| x.as_sequence()).unwrap_or(&empty);
    let mut passed = 0usize;
    let mut failures = Vec::new();
    for c in cases {
        let action = c.get("action").and_then(|x| x.as_str()).unwrap_or("");
        let want = c.get("expect").and_then(|x| x.as_str()).unwrap_or("");
        let why = c.get("why").and_then(|x| x.as_str()).unwrap_or("");
        let got = evaluate(p, vehicle, action)?;
        // 只看最终级别是不够的：两条规则 match 词重叠时，删掉其中一条仍然全绿。
        // 监工 2026-09-15 实测删掉任意一条「打火」BLOCK 规则，35/35 照样通过。
        // expect_rules 让用例能指名道姓守住某条规则。
        let want_rules: Vec<String> = c.get("expect_rules").and_then(|x| x.as_sequence())
            .map(|l| l.iter().filter_map(|r| r.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let fired: Vec<&str> = got.findings.iter().map(|f| f.rule_id.as_str()).collect();
        let rule_missing: Vec<&String> = want_rules.iter()
            .filter(|r| !fired.iter().any(|f| f == &r.as_str())).collect();
        if got.verdict == want && rule_missing.is_empty() {
            passed += 1;
        } else if got.verdict == want {
            failures.push(format!(
                "级别对（{want}）但**指名的规则没命中**：缺 {rule_missing:?}\n      \
                 实际命中: {fired:?}\n      动作: {action}\n      理由: {why}"));
        } else {
            let who = got.findings.iter()
                .map(|f| format!("{}[{}]←「{}」", f.level, f.rule_id, f.matched))
                .collect::<Vec<_>>().join(" ");
            failures.push(format!("期望 {want} 实得 {} ｜ {action}\n      理由: {why}\n      命中: {}",
                got.verdict, if who.is_empty() { "（无）".into() } else { who }));
        }
    }
    // ── 阶段维度 ──
    // 光测 action 是不够的：真实绕过走的是 phase 参数。
    let mut total = cases.len();
    let pcases = doc.get("phase_cases").and_then(|x| x.as_sequence()).unwrap_or(&empty);
    let v = p.resolve_vehicle(vehicle)?;
    let progress = v.dir.join("PROGRESS.txt");
    let saved = std::fs::read_to_string(&progress).ok();
    for c in pcases {
        total += 1;
        let phase = c.get("phase").and_then(|x| x.as_str()).unwrap_or("");
        let from  = c.get("from").and_then(|x| x.as_str()).unwrap_or("P1");
        let want  = c.get("expect_locked").and_then(|x| x.as_bool()).unwrap_or(false);
        let why   = c.get("why").and_then(|x| x.as_str()).unwrap_or("");
        // 临时把当前阶段设成 from，测完还原
        std::fs::write(&progress, from)?;
        let got = crate::workbench::build(p, Some(&v.id), Some(phase));
        let res = got.map(|w| (w.phase_locked, w.tasks.len()));
        match res {
            Ok((locked, ntasks)) if locked == want && (!want || ntasks == 0) => passed += 1,
            Ok((locked, ntasks)) => failures.push(format!(
                "phase={phase} from={from}：期望 locked={want} 实得 locked={locked}，tasks={ntasks}\n      理由: {why}")),
            Err(e) => failures.push(format!("phase={phase} from={from}：判定出错 {e:#}")),
        }
    }
    // 还原 PROGRESS.txt —— 自检不得改变真相源
    match saved {
        Some(t) => std::fs::write(&progress, t)?,
        None => { let _ = std::fs::remove_file(&progress); }
    }

    // ── 多车隔离 ──
    // 结构性隔离要有结构性的验证：直接检查另一台车的作业项里
    // 有没有出现本不属于它的字样。
    for c in doc.get("isolation_cases").and_then(|x| x.as_sequence()).unwrap_or(&empty) {
        total += 1;
        let vid = c.get("vehicle").and_then(|x| x.as_str()).unwrap_or("");
        let why = c.get("why").and_then(|x| x.as_str()).unwrap_or("");
        let getlist = |k: &str| -> Vec<String> {
            c.get(k).and_then(|x| x.as_sequence())
                .map(|l| l.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                .unwrap_or_default()
        };
        let (forbid, require) = (getlist("forbid"), getlist("require"));
        // ⚠️ 作业项与召回必须**分开比对**。
        // 监工 2026-09-15：原来两者混在一个 blob 里，于是
        // 删掉 BMW 全部三条召回检查作业项（含「后轮可能脱落」的 15V141000），
        // require 仍然满足 —— 因为召回编号还能从 open_recalls 拿到第二份。
        // **守护作业项的用例，被召回列表顶替了。**
        let mut task_blob = String::new();
        let mut recall_blob = String::new();
        let mut err = None;
        for n in 0..=9 {
            match crate::workbench::build(p, Some(vid), Some(&format!("P{n}"))) {
                Ok(w) => {
                    for t in &w.tasks {
                        task_blob.push_str(&t.name);
                        task_blob.push_str(t.detail.as_deref().unwrap_or(""));
                        task_blob.push_str(t.source.as_deref().unwrap_or(""));
                    }
                    for r in &w.open_recalls {
                        recall_blob.push_str(&r.id); recall_blob.push_str(&r.component);
                    }
                }
                Err(e) => { err = Some(format!("{e:#}")); break; }
            }
        }
        if let Some(e) = err { failures.push(format!("隔离 {vid}：{e}")); continue; }
        // 泄漏要两边都查（作业项或召回里出现别人的东西都算泄漏）
        let both = format!("{task_blob}{recall_blob}");
        let leaked: Vec<&String> = forbid.iter().filter(|w| both.contains(w.as_str())).collect();
        // 存在性**只认作业项** —— 这条用例守的就是作业项还在不在
        let missing: Vec<&String> = require.iter()
            .filter(|w| !task_blob.contains(w.as_str())).collect();
        if leaked.is_empty() && missing.is_empty() { passed += 1; }
        else {
            failures.push(format!("隔离 {vid}：{}{}\n      理由: {why}",
                if leaked.is_empty() { String::new() }
                else { format!("泄漏了不属于它的内容 {leaked:?} ") },
                if missing.is_empty() { String::new() }
                else { format!("缺了本该有的 {missing:?}") }));
        }
    }

    // ── 车辆 id 写法 ──
    for c in doc.get("vehicle_id_cases").and_then(|x| x.as_sequence()).unwrap_or(&empty) {
        total += 1;
        let id = c.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let want_reject = c.get("expect_reject").and_then(|x| x.as_bool()).unwrap_or(true);
        let why = c.get("why").and_then(|x| x.as_str()).unwrap_or("");
        let rejected = p.resolve_vehicle(Some(id)).is_err();
        if rejected == want_reject { passed += 1; }
        else {
            failures.push(format!("车辆id「{id}」：期望{}，实际{}\n      理由: {why}",
                if want_reject { "拒绝" } else { "接受" },
                if rejected { "拒绝" } else { "接受" }));
        }
    }

    Ok(SelfTest { total, passed, failures })
}


/// 取某阶段的「开工前必读」。找不到就如实说找不到，不编。
pub fn briefing(p: &Platform, phase: &str) -> Result<String> {
    let id = phase.trim().to_uppercase();
    let f = p.root.join("knowledge/安全门禁/开工前必读").join(format!("{id}.md"));
    std::fs::read_to_string(&f)
        .with_context(|| format!("没有 {id} 的开工前必读（{}）", f.display()))
}
