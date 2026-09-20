//! 工作台数据聚合 —— 前端与 CLI 共用
//!
//! 核心原则：**只查安全视图**（`specs_usable` / `manual_pages_applicable`），
//! 绝不直接碰底表。过滤是数据库的责任，不是每个调用方的自觉。

use crate::{db, platform::Platform};
use anyhow::{Context, Result};
use rusqlite::Connection;
use crate::crossval;
use serde::Serialize;
use serde_yaml_ng::Value;

#[derive(Serialize)]
pub struct HardRule { pub id: String, pub rule: String }

#[derive(Serialize)]
pub struct Recall {
    pub id: String, pub component: String,
    pub severity: String, pub note: String,
}

#[derive(Serialize)]
pub struct Gate { pub level: String, pub rule: String }

#[derive(Serialize)]
pub struct Task {
    pub id: String, pub name: String,
    pub detail: Option<String>, pub hint: Option<String>,
    pub source: Option<String>, pub done: bool,
    /// 本步要用的已核实规格,构建时从 specs_usable 实时内联(冲突项标待定)。
    /// 不在作业单里硬编数字 —— 那样迟早和规格库脱节。
    pub specs: Vec<TaskSpec>,
    /// YAML 里声明的规格关键词,DB 打开后解析成上面的 specs。不序列化。
    #[serde(skip)]
    pub spec_keys: Vec<String>,
}

#[derive(Serialize)]
pub struct TaskSpec {
    /// 交叉验证状态（G4）：安全关键且只有一个出版方背书时标出来。
    /// 不扣值 —— 这是披露，让你在拧下去之前知道它没人背书。
    pub corroboration: String,
    pub label: String, pub value: String, pub source: String, pub status: String,
}

#[derive(Serialize)]
pub struct PhaseSummary {
    pub id: String, pub name: String,
    pub touch_vehicle: bool, pub task_count: usize, pub is_current: bool,
}

#[derive(Serialize)]
pub struct Workbench {
    pub vehicle_id: String,
    pub make: String,
    pub model: String,
    pub year: i64,
    pub engine_code: String,
    pub vin: Option<String>,
    pub hard_rules: Vec<HardRule>,
    pub open_recalls: Vec<Recall>,
    pub phases: Vec<PhaseSummary>,
    pub current_phase: String,
    pub current_name: String,
    pub current_summary: String,
    pub gates: Vec<Gate>,
    pub tasks: Vec<Task>,
    /// 可用于作业的规格条数（**来自 specs_usable 视图**，C/D 级不可能出现在这里）
    pub specs_usable: i64,
    /// 待核实条数 —— 如实展示「我们还不知道多少」
    pub specs_pending: i64,
    pub manual_pages: i64,
    pub manual_excluded: i64,
    pub photo_count: i64,
    /// 请求的阶段是否被门禁扣留（超前于当前阶段，且其内容触发 BLOCK）
    pub phase_locked: bool,
    /// 扣留原因。未扣留时为空
    pub phase_lock_reason: String,
    pub warnings: Vec<String>,
}

fn sv(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string())
}


/// 解析 `open_recalls` —— **workbench 与 safety 必须用同一个**。
///
/// 监工 2026-09-15：两处各自解析，结果对同一个文件给出相反答案 ——
/// 把 open_recalls 写成最自然的字符串列表 `- 15V141000` 时，
/// workbench 造出 5 条 id 为空的记录（界面显示「🔴 未执行召回 5 项」），
/// 而 safety 把它们 filter_map 掉（列表为空 → 致命 BLOCK 静默解除）。
/// **同一程序两处读同一文件给出相反答案**，比任何一处单独错都危险。
///
/// 返回 (记录, 解析问题)。**有问题就不能当作「没有召回」** ——
/// 那是「读不出来」，对安全的含义是 fail-closed。

/// 从 YAML task 的 `specs:` 列表取关键词(每个是 canonical 或 item 子串)
fn task_spec_keys(t: &Value) -> Vec<String> {
    t.get("specs").and_then(|x| x.as_sequence())
        .map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn parse_recalls(doc: &Value) -> (Vec<Recall>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let seq = match doc.get("open_recalls") {
        None => return (out, problems),                      // 没有这个字段：留给 checked_at 机制去管
        Some(Value::Null) => return (out, problems),
        Some(v) => match v.as_sequence() {
            Some(l) => l,
            None => {
                problems.push("open_recalls 不是列表 —— 无法判断本车有哪些未执行召回".into());
                return (out, problems);
            }
        },
    };
    for (i, e) in seq.iter().enumerate() {
        if let Some(id) = e.as_str() {
            // 纯字符串写法：能拿到编号，可用；但缺少严重度与说明
            out.push(Recall {
                id: id.trim().to_string(),
                component: String::new(),
                severity: "unknown".into(),
                note: String::new(),
            });
            problems.push(format!(
                "第 {} 条召回写成了纯字符串「{}」—— 编号可用，但缺 component/severity/note，                 界面无法显示后果",
                i + 1, id.trim()));
            continue;
        }
        match sv(e, "id") {
            Some(id) if !id.trim().is_empty() => out.push(Recall {
                id: id.trim().to_string(),
                component: sv(e, "component").unwrap_or_default(),
                severity: sv(e, "severity").unwrap_or_else(|| "unknown".into()),
                note: sv(e, "note").unwrap_or_default(),
            }),
            _ => problems.push(format!(
                "第 {} 条召回读不出编号 —— **无法判断它是哪条召回**，                 依赖召回的门禁会按 fail-closed 保持关闭",
                i + 1)),
        }
    }
    (out, problems)
}


/// 研究状态：某项是否**查过**，以及查的日期。
///
/// 「查过了，没有」与「从来没查过」对安全的含义相反，而空列表分不出来。
/// 这个结构就是用来分的。
///
/// 日期**必须是 YYYY-MM-DD**。监工 2026-09-15：原来只判「非空」，
/// 于是 `recalls_checked_at: "从来没查过"` 反而把「从未查过」的警告消掉了。
#[derive(Debug, Serialize, Clone)]
pub struct ResearchStatus {
    pub recalls_checked_at: Option<String>,
    pub known_issues_checked_at: Option<String>,
    pub problems: Vec<String>,
}

pub fn research_status(doc: &Value) -> ResearchStatus {
    let mut problems = Vec::new();
    let mut get = |k: &str| -> Option<String> {
        let raw = doc.get(k)?;
        // YAML 的 2026-09-14 会被解析成日期标量，也可能是字符串
        let t = raw.as_str().map(|x| x.trim().to_string())
            .or_else(|| match raw {
                Value::String(x) => Some(x.trim().to_string()),
                _ => serde_yaml_ng::to_string(raw).ok().map(|x| x.trim().to_string()),
            })?;
        if t.is_empty() || t == "~" || t == "null" { return None; }
        let ok = t.len() == 10
            && t.as_bytes().iter().enumerate().all(|(i, b)| match i {
                4 | 7 => *b == b'-',
                _ => b.is_ascii_digit(),
            });
        if !ok {
            problems.push(format!(
                "{k} 的值「{t}」不是 YYYY-MM-DD 日期 —— 按「没查过」处理。随手写一句话就能消掉安全警告，那这个字段等于没有"));
            return None;
        }
        Some(t)
    };
    let r = get("recalls_checked_at");
    let k = get("known_issues_checked_at");
    ResearchStatus { recalls_checked_at: r, known_issues_checked_at: k, problems }
}

/// 本车在门禁规则表里有没有专属规则。没有 = 门禁只用通用规则覆盖它。
pub fn vehicle_specific_rule_count(p: &Platform, vehicle_id: &str) -> usize {
    let f = p.root.join("knowledge/安全门禁/规则表.yml");
    let doc: Value = match std::fs::read_to_string(&f).ok()
        .and_then(|t| serde_yaml_ng::from_str(&t).ok()) { Some(d) => d, None => return 0 };
    let empty = Vec::new();
    doc.get("rules").and_then(|x| x.as_sequence()).unwrap_or(&empty).iter()
        .filter(|r| r.get("vehicles").and_then(|x| x.as_sequence())
            .map(|vs| vs.iter().filter_map(|x| x.as_str()).any(|x| x == vehicle_id))
            .unwrap_or(false))
        .count()
}

pub fn build(p: &Platform, vehicle: Option<&str>, phase: Option<&str>) -> Result<Workbench> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)
        .context("VEHICLE.yml 解析失败")?;

    let hard_rules: Vec<HardRule> = doc.get("hard_rules").and_then(|x| x.as_sequence()).map(|l| {
        l.iter().map(|e| HardRule {
            id: sv(e, "id").unwrap_or_default(),
            rule: sv(e, "rule").unwrap_or_default(),
        }).collect()
    }).unwrap_or_default();

    let (open_recalls, recall_problems) = parse_recalls(&doc);

    // 流程定义（跨车通用）
    let flow_path = p.root.join("knowledge/flows/长期停放复活.yml");
    let flow: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(&flow_path)
        .with_context(|| format!("读不到流程定义 {}", flow_path.display()))?)?;
    let empty = vec![];
    let phase_list = flow.get("phases").and_then(|x| x.as_sequence()).unwrap_or(&empty);

    // 阶段 id 归一化：大小写、前后空格、P08 这类写法都收进来。
    // 归一化之后仍然找不到 → **明确报错**，绝不静默返回一个空工作台。
    // （2026-09-14 自攻实测：`phase=p8` 此前返回 phase_locked=false、tasks=0 ——
    //   它回答的是「未锁定」而不是「没有这个阶段」，是个误导性答案。）
    let norm = |s: &str| -> String {
        let t = s.trim().trim_start_matches(['P','p']);
        match t.parse::<i32>() { Ok(n) => format!("P{n}"), Err(_) => s.trim().to_uppercase() }
    };
    let cur_id = phase.map(|s| norm(s))
        .or_else(|| std::fs::read_to_string(v.dir.join("PROGRESS.txt")).ok()
                       .map(|s| norm(&s)).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "P1".to_string());
    if !phase_list.iter().any(|ph| sv(ph, "id").as_deref() == Some(cur_id.as_str())) {
        let all: Vec<String> = phase_list.iter().filter_map(|ph| sv(ph, "id")).collect();
        anyhow::bail!("没有阶段「{}」。本流程的阶段是：{}", cur_id, all.join(" "));
    }

    let phases: Vec<PhaseSummary> = phase_list.iter().map(|ph| {
        let id = sv(ph, "id").unwrap_or_default();
        PhaseSummary {
            is_current: id == cur_id,
            touch_vehicle: ph.get("touch_vehicle").and_then(|x| x.as_bool()).unwrap_or(false),
            task_count: ph.get("tasks").and_then(|x| x.as_sequence()).map(|l| l.len()).unwrap_or(0),
            name: sv(ph, "name").unwrap_or_default(),
            id,
        }
    }).collect();

    let cur = phase_list.iter().find(|ph| sv(ph, "id").as_deref() == Some(cur_id.as_str()));
    let (gates, tasks, cur_name, cur_summary) = match cur {
        Some(ph) => (
            ph.get("gates").and_then(|x| x.as_sequence()).map(|l| l.iter().map(|g| Gate {
                level: sv(g, "level").unwrap_or_else(|| "WARN".into()),
                rule: sv(g, "rule").unwrap_or_default(),
            }).collect()).unwrap_or_default(),
            ph.get("tasks").and_then(|x| x.as_sequence()).map(|l| l.iter().map(|t| Task {
                id: sv(t, "id").unwrap_or_default(),
                name: sv(t, "name").unwrap_or_default(),
                detail: sv(t, "detail"), hint: sv(t, "hint"), source: sv(t, "source"),
                done: false, specs: vec![], spec_keys: task_spec_keys(t),
            }).collect()).unwrap_or_default(),
            sv(ph, "name").unwrap_or_default(),
            sv(ph, "summary").unwrap_or_default(),
        ),
        None => (vec![], vec![], String::new(), String::new()),
    };

    // ── 并入本车专有作业项 ────────────────────────────────────
    //
    // 通用流程只放任何摩托车都适用的项；某台车的召回、某代的设计缺陷
    // 放在该车 VEHICLE.yml 的 phase_tasks 里。这样隔离是结构性的 ——
    // 另一台车的档案里没有这些项，就不可能拿到它们。
    // （监工 2026-09-14-c：此前 BMW 的召回写在通用流程里，
    //   `workbench -v 2014-HD-XL1200` 返回了三条 BMW 召回并带 A 级徽章。）
    // phase_tasks 现在承载安全（车辆专有的召回检查在这里），写坏了必须出声。
    // 监工 2026-09-15：8 种写坏方式全部静默降级、退出码 0。
    let mut tasks = tasks;
    let mut task_problems: Vec<String> = Vec::new();
    match doc.get("phase_tasks") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_mapping() {
            None => task_problems.push("phase_tasks 不是映射 —— 本车的专有作业项全部没有生效".into()),
            Some(m) => {
                for (k, _) in m {
                    let ks = k.as_str().unwrap_or("");
                    if !phase_list.iter().any(|ph| sv(ph, "id").as_deref() == Some(ks)) {
                        task_problems.push(format!(
                            "phase_tasks 里有未知阶段「{ks}」—— 挂在它下面的作业项永远不会出现"));
                    }
                }
                match m.get(Value::from(cur_id.as_str())) {
                    None | Some(Value::Null) => {}
                    Some(x) => match x.as_sequence() {
                        None => task_problems.push(format!(
                            "phase_tasks.{cur_id} 不是列表 —— 本阶段的专有作业项没有生效")),
                        Some(list) => {
                            let mut seen: Vec<String> = Vec::new();
                            for (i, t) in list.iter().enumerate() {
                                let name = sv(t, "name").unwrap_or_default();
                                if name.trim().is_empty() {
                                    task_problems.push(format!(
                                        "phase_tasks.{cur_id} 第 {} 条没有 name —— 会显示成空行，等于没有",
                                        i + 1));
                                    continue;
                                }
                                let id = sv(t, "id").unwrap_or_default();
                                if !id.is_empty() && seen.contains(&id) {
                                    task_problems.push(format!(
                                        "phase_tasks.{cur_id} 里 id「{id}」重复 —— 无法分辨是哪一条"));
                                }
                                if !id.is_empty() { seen.push(id.clone()); }
                                tasks.push(Task {
                                    id, name,
                                    detail: sv(t, "detail"), hint: sv(t, "hint"), source: sv(t, "source"),
                                    done: false, specs: vec![], spec_keys: task_spec_keys(t),
                                });
                            }
                        }
                    },
                }
            }
        },
    }

    // ── 阶段门禁 ───────────────────────────────────────────────
    //
    // 监工 2026-09-14-b 抓到的绕过：门禁此前只校验 `intent` 字符串，
    // 对 `phase` 参数零校验 —— 在 P1 请求 `phase=P8` 就能拿到「点火」步骤，
    // 而同样的意图用 intent 表达会被 BLOCK。MCP / CLI --phase / HTTP ?phase= 三条路同样通。
    //
    // 拦截放在这里而不是各调用点，是因为这是三条路唯一的共同必经之处。
    // 判定对象是**阶段自己的内容**（名称 + 摘要 + 作业项），不是调用方自称的意图 ——
    // 调用方说什么不重要，它要拿到的东西是什么才重要。
    let real_cur = std::fs::read_to_string(v.dir.join("PROGRESS.txt")).ok()
        .map(|s| norm(&s)).filter(|s| !s.is_empty())
        .unwrap_or_else(|| "P1".to_string());
    let ord = |x: &str| x.trim_start_matches(['P','p']).parse::<i32>().unwrap_or(0);
    let mut phase_locked = false;
    let mut phase_lock_reason = String::new();
    let mut gates = gates;
    if ord(&cur_id) > ord(&real_cur) {
        let probe = format!("{} {} {}", cur_name, cur_summary,
            tasks.iter().map(|t| format!("{} {}", t.name,
                t.detail.clone().unwrap_or_default())).collect::<Vec<_>>().join(" "));
        // ⚠️ 这里**不能**吞错误。
        // 监工 2026-09-14-c：原来写的是 `if let Ok(sr) = ...`，
        // 规则表读不到时判定直接跳过，`workbench --phase P9` 一声不吭发出「路试」——
        // 一个本该 fail-closed 的门禁，实现成了 fail-open。
        let sr = crate::safety::evaluate(p, Some(&v.id), &probe)
            .with_context(|| format!(
                "阶段 {cur_id} 超前于 {real_cur}，需要过安全门禁，但门禁判定本身失败了 —— 按 fail-closed 处理，不返回作业步骤"))?;
        {
            if !sr.may_return_steps {
                phase_locked = true;
                let hits = sr.findings.iter().filter(|f| f.level == "BLOCK")
                    .map(|f| format!("{}（命中「{}」）：{}", f.rule_id, f.matched,
                         f.unblock.lines().next().unwrap_or("")))
                    .collect::<Vec<_>>().join("；");
                phase_lock_reason = format!(
                    "阶段 {cur_id} 超前于当前阶段 {real_cur}，且其作业内容触发 BLOCK —— \
                     作业步骤已扣留。{hits}");
                // 步骤不返回。门禁与作业项仍然给，让人知道这个阶段存在、卡在哪
                tasks = Vec::new();
                gates.push(Gate { level: "BLOCK".into(), rule: phase_lock_reason.clone() });
            }
        }
    }

    // ── 数据库：只查视图 ──
    let conn: Connection = db::open(&p.index_db())?;
    let q = |sql: &str| -> i64 {
        conn.query_row(sql, [&v.id], |r| r.get(0)).unwrap_or(0)
    };
    let specs_usable  = q("SELECT COUNT(*) FROM specs_usable WHERE vehicle_id=?1");
    let specs_pending = q("SELECT COUNT(*) FROM specs WHERE vehicle_id=?1 AND pending=1");
    let manual_pages  = q("SELECT COUNT(*) FROM manual_pages_applicable WHERE vehicle_id=?1");
    let manual_excluded = q("SELECT COUNT(*) FROM manuals WHERE vehicle_id=?1 AND applicable=0");
    let photo_count   = q("SELECT COUNT(*) FROM photos WHERE vehicle_id=?1");

    // ── ⑥ 作业单可执行化：把已核实规格实时内联进每一步 ──
    // 不在 YAML/代码里硬编数字 —— 只声明「本步要用哪条规格」的关键词，
    // 值一律现查 specs_usable（A/B 级、非 pending、无冲突）。
    // 有冲突 → 从 specs_usable 消失，落到 spec_conflicts，本步标「待定·有冲突」并拒绝给值。
    // 查无 → 明说「规格库里没有」，而不是让这一步看起来没有规格要求。
    // G4：一次算出「安全关键且只有一个出版方背书」的集合。
    // 判定逻辑只有 crossval 一份 —— 分成两份迟早漂移，而漂移那天没人会发现。
    // fail-open 的写法在这里是可接受的：算不出来就不标，**绝不反过来说「已交叉验证」**。
    let single_crit = crossval::single_source_criticals(p, &conn, &v.id, &doc)
        .unwrap_or_default();
    let corro = |canon: &str| -> String {
        if single_crit.contains(canon) { "⚠️ 安全关键·单来源".to_string() } else { String::new() }
    };

    for t in tasks.iter_mut() {
        for key in &t.spec_keys {
            // 解析走 specref —— **全项目唯一一份**。
            // 此前这里和 diag.rs 各写一份，同一个「冲突退化成缺失」的 bug 犯了两次
            // （2026-09-16 修 workbench，2026-09-19 在 diag 原样复活）。
            match crate::specref::resolve(&conn, &v.id, key) {
                crate::specref::SpecState::Verified { item, canonical, value, level, source_ref, page } => {
                    t.specs.push(TaskSpec {
                        source: crate::specref::source_label(&level, &source_ref, page),
                        corroboration: corro(&canonical),
                        label: item, value, status: "已核实".into(),
                    });
                }
                crate::specref::SpecState::Conflicted { values, sources, .. } => {
                    t.specs.push(TaskSpec {
                        label: key.clone(),
                        value: format!("冲突值：{values}（已扣下，不可用）"),
                        source: sources,
                        status: "待定·有冲突".into(),
                        corroboration: String::new(),
                    });
                }
                crate::specref::SpecState::Missing => {
                    t.specs.push(TaskSpec {
                        label: key.clone(),
                        value: "—".into(),
                        source: "规格库无此项".into(),
                        status: "未收录".into(),
                        corroboration: String::new(),
                    });
                }
            }
        }
    }

    // ── 警告：如实暴露状态，不粉饰 ──
    let mut warnings = Vec::new();
    for pb in &recall_problems {
        warnings.push(format!("🔴 召回档案有问题：{pb}"));
    }
    for pb in &task_problems {
        warnings.push(format!("🔴 本车专有作业项有问题：{pb}"));
    }

    // 「查过了，没有」与「从来没查过」是两件事，空列表把它们混成了一件。
    // 监工 2026-09-15：哈雷的档案里 hard_rules / open_recalls / known_issues 全是 0，
    // 工作台却一个字都不提 —— 看上去像一台干净的车，实际上是**没人研究过的车**。
    // 所以要求档案显式声明查过的日期；没有声明就当作没查过，并且说出来。
    {
        let rs = research_status(&doc);
        for pb in &rs.problems { warnings.push(format!("🔴 {pb}")); }
        if rs.recalls_checked_at.is_none() {
            warnings.push(format!(concat!(
                "🔴 **本车的召回从未查过**（档案里没有合法的 recalls_checked_at 日期）。",
                "open_recalls 现在是 {} 条 —— 那是「没查」不是「没有」。",
                "任何依赖召回的门禁都拿不到判断依据，只能放行。",
                "查法：motomech tool nhtsa.recalls make=<厂> model=<型> year={}"),
                open_recalls.len(),
                doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0)));
        }
        if rs.known_issues_checked_at.is_none() {
            warnings.push(concat!(
                "🔴 **本车的故障模式从未查过**（档案里没有合法的 known_issues_checked_at 日期）",
                "—— 同样是「没查」不是「没有」。查法：motomech tool nhtsa.complaints"
            ).to_string());
        }
        if hard_rules.is_empty() {
            warnings.push(concat!(
                "🔴 **本车没有任何硬红线**。硬红线是「违反会毁件」的不可调换约定；",
                "一台停放多年的车不可能一条都没有 —— 这说明档案还是骨架。"
            ).to_string());
        }
        // 门禁覆盖度也要进工作台 —— 此前它只在 safety 的输出里，
        // 而人看的是工作台（监工 2026-09-15：三通道全缺）
        if vehicle_specific_rule_count(p, &v.id) == 0 {
            warnings.push(format!(concat!(
                "🔴 **门禁规则表里没有本车（{}）的专属规则**，只有通用规则覆盖它。",
                "召回、代际缺陷这类一车一款的约束都还没写进去 —— ",
                "门禁判 OK 不等于这台车安全，只等于这张表还没覆盖它。"), v.id));
        }
    }

    if doc.get("vin").and_then(|x| x.as_str()).is_none() {
        warnings.push("VIN 未采集 —— 无法查询精确配置与召回执行状态".to_string());
    }
    let crit = open_recalls.iter().filter(|r| r.severity == "critical").count();
    if crit > 0 {
        warnings.push(format!("{crit} 项未执行的关键召回 —— 骑行前必须确认"));
    }
    if specs_pending > 0 {
        warnings.push(format!("{specs_pending} 项规格待核实，不可用于作业"));
    }

    Ok(Workbench {
        vehicle_id: v.id.clone(),
        make: sv(&doc, "make").unwrap_or_default(),
        model: sv(&doc, "model").unwrap_or_default(),
        year: doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0),
        engine_code: sv(&doc, "engine_code").unwrap_or_default(),
        vin: sv(&doc, "vin"),
        hard_rules, open_recalls, phases, phase_locked, phase_lock_reason,
        current_phase: cur_id, current_name: cur_name, current_summary: cur_summary,
        gates, tasks,
        specs_usable, specs_pending, manual_pages, manual_excluded, photo_count,
        warnings,
    })
}
