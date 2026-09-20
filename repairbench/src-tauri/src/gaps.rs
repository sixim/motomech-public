//! 差距引擎 —— 让系统说得清「我知道自己不知道什么」
//!
//! 这是「自己找需求」的地基。没有它，补资料只能靠人翻文档发现缺口，
//! 而人翻不过来 —— 2026-09-15 那条侧撑扭力冲突（同一本手册两页给 56 和 42）
//! 就是肉眼偶然撞见的，系统本该自己报出来。
//!
//! 三条设计约束，都是四轮监工换来的：
//!
//! 1. **差距要按后果排序，不按数量排序。** 一条安全关键的冲突，
//!    比二十条「待补的改装资料」重要。
//! 2. **每条差距必须写明谁能关掉它** —— app 能自己办的（调 API、重建索引、
//!    交叉验证），和只有人能办的（拍照、打电话、插硬件），性质完全不同。
//! 3. **差距引擎只报告，不自己动手改规格。** 一个能改自己规格库的 AI，
//!    正是「自信的错误扭力值」的生成器。

use crate::{db, platform::Platform, workbench};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub enum Who { App, User, Both }

impl Who {
    fn label(&self) -> &'static str {
        match self { Who::App => "app 可自己关闭", Who::User => "只有你能关闭", Who::Both => "需两边配合" }
    }
}

#[derive(Debug, Serialize)]
pub struct Gap {
    pub id: String,
    pub kind: String,
    /// 0 = 安全关键（错了会伤人或毁件）；1 = 卡关键路径；2 = 一般
    pub severity: u8,
    pub title: String,
    pub detail: String,
    /// 卡住哪个阶段，空 = 不卡
    pub blocks: String,
    pub who: Who,
    /// 怎么关掉它 —— 能给命令就给命令
    pub how: String,
}

#[derive(Debug, Serialize)]
pub struct GapReport {
    pub vehicle_id: String,
    pub gaps: Vec<Gap>,
    pub by_severity: Vec<(String, usize)>,
    pub app_closable: usize,
    pub user_only: usize,
}

fn sv(v: &serde_yaml_ng::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn find(p: &Platform, vehicle: Option<&str>) -> Result<GapReport> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let mut gaps: Vec<Gap> = Vec::new();
    let empty = Vec::new();

    // ── 0. 交叉验证的发现 —— 系统的自我认知不能裂成两半 ──────
    //
    // 2026-09-19：`crossval` 查出 44 条安全关键项**只有一个来源背书**，
    // 而差距引擎一条都不知道 —— 人看 gaps 会以为「只剩这 23 条」。
    // 差距引擎的定位就是「系统自己知道还不知道什么」，
    // 那它必须知道**别的模块已经知道的事**，否则这份自我认知是假的。
    //
    // 归类：单来源不是「缺资料」，是「这个数没人复核过」。
    // 它比缺资料隐蔽 —— 缺资料你看得见，单来源长得和已核实一模一样。
    if let Ok(cv) = crate::crossval::build(p, &v) {
        if !cv.single_source.is_empty() {
            // 不逐条刷屏：44 条逐条列出会把真正卡路径的差距淹掉。
            // 报一条，把清单指向 crossval。
            let claims = cv.single_source.iter().filter(|x| x.claims_only).count();
            gaps.push(Gap {
                id: "crossval.single_source".into(),
                kind: "单来源".into(),
                severity: 0,
                title: format!("{} 条安全关键规格只有一个来源背书", cv.single_source.len()),
                detail: format!(
                    "全库已核实规格出自 {} 个独立文档{}。\
                     单来源**不是错**，但它没人复核过 —— 而它长得和已交叉验证的数据一模一样。\
                     本项目栽过的正是这个结构：整套规格出自一本手册，而那本手册对不上这台车。{}",
                    cv.all_docs.len(),
                    if cv.all_docs.len() <= 1 { "（**只有一个** —— 交叉验证在结构上不可能达成）" } else { "" },
                    if claims > 0 {
                        format!("\n另有 {claims} 条的出处**文字里自称**「与另一来源一致」，\
                                 但库里没有第二条结构化记录 —— 散文里的印证机器核不了。")
                    } else { String::new() }),
                blocks: String::new(),
                who: Who::Both,
                how: "看清单：repairbench crossval。要真正关掉，需引入第二个独立来源\
                      （BMW RepROM / 官方 fiche / 厂商 TSB），那一步只有你能办".into(),
            });
        }
        // 出处认不出来的，会让交叉验证算不准 —— 这条 app 自己能关
        if let Some((_, n)) = cv.all_docs.iter().find(|(d, _)| d == "unknown") {
            gaps.push(Gap {
                id: "crossval.unknown_source".into(),
                kind: "来源未登记".into(),
                severity: 2,
                title: format!("{n} 条规格的出处匹配不上任何已登记文档"),
                detail: "它们被归入 unknown 一个桶，**彼此之间不会互相印证**（fail-closed）。\
                         代价是这些规格即使真有多个来源也算不出独立性。".into(),
                blocks: String::new(),
                who: Who::App,
                how: "在 knowledge/来源文档.yml 里登记这些文档的 match 串".into(),
            });
        }
    }

    // 脚本体检的发现同理。视频里念错一个数会被观众照抄，
    // 而没有人会回来核对 —— 这类差距的传播面比作业单还大。
    if let Ok(sc) = crate::script::check(p, &v) {
        let unsourced: usize = sc.scripts.iter().map(|x| x.unsourced).sum();
        let conflicted: usize = sc.scripts.iter().map(|x| x.conflicted).sum();
        if conflicted > 0 {
            gaps.push(Gap {
                id: "script.conflicted".into(),
                kind: "脚本含争议数值".into(),
                severity: 0,
                title: format!("Vlog 脚本里有 {conflicted} 处念到了**有争议**的数值"),
                detail: "该规格有多个矛盾来源，值已被规格库扣下 —— \
                         那它更不该出现在视频里。观众会照抄，而没人会回来核对。".into(),
                blocks: String::new(),
                who: Who::User,
                how: "repairbench script 看具体位置；删掉或改口，别念那个数".into(),
            });
        }
        if unsourced > 0 {
            gaps.push(Gap {
                id: "script.unsourced".into(),
                kind: "脚本数值无出处".into(),
                severity: 1,
                title: format!("Vlog 脚本里有 {unsourced} 处技术数字在规格库里找不到"),
                detail: "不一定是错的，但说不出出处。视频里的数字会被观众照抄。".into(),
                blocks: String::new(),
                who: Who::User,
                how: "repairbench script 看具体位置；删掉、改口，或去核实后入库".into(),
            });
        }
    }

    // ── 1. 数值冲突 —— 最高优先级 ──────────────────────────
    // **系统自己检出的**：同一语义身份(canonical)有多个已核实值。
    // 不是「缺资料」，是「资料在打架」，比缺更危险 —— 两个数字都看着有出处。
    // 数据源是 spec_conflicts 视图（入库时按 canonical 自动分组），
    // 不再靠人预判后写进 pending。这是「自己找需求」的实现：
    // 我只管把看到的都录进去，系统自己判它们打不打架。
    if let Ok(conn) = db::open(&p.index_db()) {
        if let Ok(mut st) = conn.prepare(
            "SELECT canonical, values_csv, items_csv, sources_csv FROM spec_conflicts WHERE vehicle_id=?1") {
            let rows = st.query_map([&v.id], |r| Ok((
                r.get::<_,String>(0)?, r.get::<_,String>(1)?,
                r.get::<_,String>(2)?, r.get::<_,String>(3)?))).ok();
            if let Some(rows) = rows {
                for (canon, vals, items, srcs) in rows.flatten() {
                    let safety = ["torque","brake","valve","stand","clearance"]
                        .iter().any(|w| canon.contains(w) || items.contains(w));
                    gaps.push(Gap {
                        id: format!("conflict:{canon}"),
                        kind: "数值冲突".into(),
                        severity: if safety { 0 } else { 1 },
                        title: format!("{canon} 有互相矛盾的已核实值 [{vals}]"),
                        detail: format!("同一语义身份出现多个值 —— 来源：{srcs}（涉及 {items}）。\
                                         两个都带出处、都已回原图，所以两个都被扣下，一个都不能用。"),
                        blocks: String::new(),
                        who: Who::Both,
                        how: "回两处原图各核一次读数；仍冲突就找第三个独立来源（BMW RepROM / 官方 fiche）。\
                              定论后：删掉错的那条 verified 记录，冲突自动消失。".into(),
                    });
                }
            }
        }
    }

    // ── 2. 从没查过的研究项 ────────────────────────────────
    let rs = workbench::research_status(&doc);
    let year = doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0);
    let make = sv(&doc, "make").unwrap_or_default();
    let model = sv(&doc, "model").unwrap_or_default();
    if rs.recalls_checked_at.is_none() {
        gaps.push(Gap {
            id: "research:recalls".into(), kind: "从未研究".into(), severity: 0,
            title: "本车的召回从未查过".into(),
            detail: "空的 open_recalls 是「没查」不是「没有」。依赖召回的门禁拿不到判断依据。".into(),
            blocks: "P9 上路".into(), who: Who::App,
            how: format!("repairbench tool nhtsa.recalls make={make} model=\"{model}\" year={year}"),
        });
    }
    if rs.known_issues_checked_at.is_none() {
        gaps.push(Gap {
            id: "research:issues".into(), kind: "从未研究".into(), severity: 1,
            title: "本车的故障模式从未查过".into(),
            detail: "不知道这台车会怎么坏，就不知道该重点检查什么。".into(),
            blocks: "P1 静态评估".into(), who: Who::App,
            how: format!("repairbench tool nhtsa.complaints make={make} model=\"{model}\" year={year}"),
        });
    }

    // ── 3. 待核实规格（非冲突的）──────────────────────────
    for e in doc.get("specs_pending").and_then(|x| x.as_sequence()).unwrap_or(&empty) {
        let item = sv(e, "item").unwrap_or_default();
        if gaps.iter().any(|g| g.id == format!("conflict:{item}")) { continue; }
        let note = sv(e, "note").unwrap_or_default();
        let level = sv(e, "level").unwrap_or_else(|| "D".into());
        let safety = item.contains("torque") || item.contains("brake") || item.contains("valve");
        let user_only = note.contains("VIN") || note.contains("fiche")
            || note.contains("RealOEM") || note.contains("拍");
        gaps.push(Gap {
            id: format!("pending:{item}"),
            kind: "待核实".into(),
            severity: if safety { 0 } else { 2 },
            title: format!("{item} 还没有 A/B 级出处（现 {level} 级）"),
            detail: note.chars().take(200).collect(),
            blocks: String::new(),
            who: if user_only { Who::User } else { Who::Both },
            how: if user_only { "见 note —— 需要你去采 VIN / 查 fiche / 拍页".into() }
                 else { "续拍手册对应章节，或找第二个独立来源交叉验证".into() },
        });
    }

    // ── 4. 门禁覆盖度 ──────────────────────────────────────
    if workbench::vehicle_specific_rule_count(p, &v.id) == 0 {
        gaps.push(Gap {
            id: "rules:none".into(), kind: "门禁未覆盖".into(), severity: 0,
            title: format!("门禁规则表里没有本车（{}）的专属规则", v.id),
            detail: "只有通用规则覆盖它。门禁判 OK 不等于安全，只等于这张表还没覆盖。".into(),
            blocks: "全阶段".into(), who: Who::Both,
            how: "先关掉「召回从未查过」和「故障模式从未查过」，再把结果写成规则（带 basis）".into(),
        });
    }

    // ── 5. 零件被挡 ────────────────────────────────────────
    if let Ok(pp) = crate::parts::plan(p, Some(&v.id), 7) {
        for part in pp.parts.iter().filter(|x| x.blocked_by.is_some()) {
            gaps.push(Gap {
                id: format!("part:{}", part.id), kind: "零件下不了单".into(), severity: 1,
                title: format!("{} 下不了单", part.name),
                detail: part.blocked_by.clone().unwrap_or_default().chars().take(160).collect(),
                blocks: part.needed_phase.clone(), who: Who::User,
                how: "见左侧说明".into(),
            });
        }
        for b in &pp.blockers {
            gaps.push(Gap {
                id: format!("blocker:{}", b.id), kind: "只有你能做".into(), severity: 1,
                title: b.what.clone(), detail: b.why.clone(),
                blocks: b.phase.clone(), who: Who::User,
                how: format!("解锁：{}", if b.unblocks.is_empty() { "—".into() } else { b.unblocks.join("、") }),
            });
        }
    }

    // ── 6. 手册覆盖 ────────────────────────────────────────
    // **读声明，不推断。** OCR 在这本书上读不出章节头
    // （「Chapter 4」被并进副标题、「1•2」被读成「+2」），
    // 靠它推断会给出错误的覆盖报告 —— 错的差距报告和假绿灯是同一类问题。
    {
        let rawdir = v.dir.join("10_手册资料/third-party/haynes-4598/raw");
        let decl = rawdir.join("覆盖声明.tsv");
        let mut declared: Vec<(String, String)> = Vec::new();   // (文件, 章节)
        if let Ok(t) = std::fs::read_to_string(&decl) {
            for line in t.lines() {
                if line.starts_with('#') || line.starts_with("源文件") || line.trim().is_empty() { continue; }
                let c: Vec<&str> = line.split('\t').collect();
                if c.len() >= 2 { declared.push((c[0].trim().into(), c[1].trim().into())); }
            }
        }
        // raw/ 里有、但没声明的
        let mut undeclared: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&rawdir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if !n.to_lowercase().ends_with(".pdf") { continue; }
                if !declared.iter().any(|(f, _)| f == &n) { undeclared.push(n); }
            }
        }
        if !undeclared.is_empty() {
            undeclared.sort();
            gaps.push(Gap {
                id: "manual:undeclared".into(), kind: "扫描未声明".into(), severity: 1,
                title: format!("{} 份扫描没有声明覆盖范围", undeclared.len()),
                detail: format!("{}\n不知道它们覆盖书里的哪一段，就无法判断手册还缺什么。",
                                undeclared.join("\n")),
                blocks: String::new(), who: Who::Both,
                how: "在 raw/覆盖声明.tsv 里补行：文件名<TAB>章节<TAB>书页起<TAB>书页止".into(),
            });
        }
        if !declared.is_empty() {
            let mut have: Vec<String> = declared.iter().map(|(_, c)| c.clone()).collect();
            have.sort(); have.dedup();
            // 一个声明可能覆盖多章（"5+6+7"）或带前缀（"前置+1"、"2+3"）。
            // 要**展开出全部数字**，不能只取最后一段 ——
            // 否则 "5+6+7" 会被当成只有第 7 章，第 5、6 章凭空变成缺失。
            let nums: Vec<i64> = have.iter()
                .flat_map(|c| c.split(['+', '-', ' ', '、'])
                    .filter_map(|x| x.trim().parse::<i64>().ok())
                    .collect::<Vec<_>>())
                .collect();
            // 章数**不能硬编码成 9**。不同手册章数不同 ——
            // Haynes 4598 是 7 章 + Reference，硬编码 9 会永远报「缺第 8、9 章」。
            // 优先读声明文件头部的 `# total_chapters: N`；没有就用已声明的最大章号兜底。
            let total_ch = std::fs::read_to_string(&decl).ok()
                .and_then(|t| t.lines()
                    .find_map(|l| l.trim_start_matches('#').trim()
                        .strip_prefix("total_chapters:")
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<i64>().ok())))
                .unwrap_or_else(|| nums.iter().copied().max().unwrap_or(0));
            let missing: Vec<i64> = (1..=total_ch).filter(|n| !nums.contains(n)).collect();
            if !missing.is_empty() {
                gaps.push(Gap {
                    id: "manual:chapters".into(), kind: "手册未拍全".into(), severity: 2,
                    title: format!("手册还缺章节：{}",
                        missing.iter().map(|n| format!("第{n}章")).collect::<Vec<_>>().join(" ")),
                    detail: format!("已声明覆盖：{}", have.join("、")),
                    blocks: String::new(), who: Who::User,
                    how: "拍成 PDF 放进 raw/（按 005_ 006_ 顺序命名），在覆盖声明.tsv 补一行，跑 bookproc.sh".into(),
                });
            }
        }
    }

    // 排序：后果优先，同级别下 app 能自己关的排前面（先做能做的）
    gaps.sort_by(|a, b| a.severity.cmp(&b.severity)
        .then_with(|| (a.who != Who::App).cmp(&(b.who != Who::App))));

    let app_closable = gaps.iter().filter(|g| g.who == Who::App).count();
    let user_only = gaps.iter().filter(|g| g.who == Who::User).count();
    let mut by_severity: Vec<(String, usize)> = Vec::new();
    for (lab, lv) in [("安全关键", 0u8), ("卡关键路径", 1), ("一般", 2)] {
        by_severity.push((lab.into(), gaps.iter().filter(|g| g.severity == lv).count()));
    }
    Ok(GapReport { vehicle_id: v.id, gaps, by_severity, app_closable, user_only })
}

pub fn render(r: &GapReport) -> String {
    let mut o = format!("车辆 {}\n", r.vehicle_id);
    o.push_str(&format!("差距 {} 条：{}\n",
        r.gaps.len(),
        r.by_severity.iter().map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>().join(" · ")));
    o.push_str(&format!("其中 app 能自己关闭 {} 条，只有你能关闭 {} 条\n",
        r.app_closable, r.user_only));
    let mut last = 9u8;
    for g in &r.gaps {
        if g.severity != last {
            o.push_str(match g.severity {
                0 => "\n━━━ 🔴 安全关键 —— 错了会伤人或毁件 ━━━\n",
                1 => "\n━━━ 🟡 卡关键路径 ━━━\n",
                _ => "\n━━━ ⚪ 一般 ━━━\n",
            });
            last = g.severity;
        }
        o.push_str(&format!("\n[{}] {}\n", g.kind, g.title));
        if !g.detail.trim().is_empty() {
            o.push_str(&format!("   {}\n", g.detail.trim().replace('\n', "\n   ")));
        }
        if !g.blocks.is_empty() { o.push_str(&format!("   卡住：{}\n", g.blocks)); }
        o.push_str(&format!("   {} —— {}\n", g.who.label(), g.how));
    }
    o
}
