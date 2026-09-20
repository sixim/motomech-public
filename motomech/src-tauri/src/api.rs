//! 共用查询层 —— MCP / HTTP / CLI 三处调用同一份实现
//!
//! 为什么要有这一层：监工 2026-09-14-b 抓到「manual_search 已排除 N 份」的提示
//! 只在 CLI 里有，MCP 返回的文本一字未提 —— 同一个查询，两条通道说法不同。
//! 那不是疏忽，是「同一逻辑写了两遍」的必然结果。
//!
//! 所以：查询逻辑只在这里写一次，渲染文本也在这里写一次。
//! 上层（mcp.rs / main.rs 的 serve / CLI）只负责把结果包成各自的协议格式。

use crate::{db, platform::Platform, workbench};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SpecHit {
    pub item: String, pub value: String,
    pub source_level: String, pub source_ref: String, pub page: i64,
}

#[derive(Debug, Serialize)]
pub struct SpecResult {
    pub vehicle: String,
    pub query: String,
    pub specs: Vec<SpecHit>,
    /// 相关的待核实条数 —— 让调用方分得清「查不到」和「还没核实」
    pub pending_related: i64,
    /// 命中的冲突：查不到不一定是没有，可能是多个来源打架
    pub conflicts: Vec<SpecConflict>,
}

#[derive(Debug, Serialize)]
pub struct SpecConflict {
    pub canonical: String,
    pub values: String,
    pub sources: String,
}

/// 查规格。只从 specs_usable 视图出 —— C/D 级在 SQL 层就不在结果集里。
pub fn specs(p: &Platform, vehicle: Option<&str>, query: &str) -> Result<SpecResult> {
    let v = p.resolve_vehicle(vehicle)?;
    let conn = db::open(&p.index_db())?;
    let like = format!("%{}%", query.to_lowercase());
    let mut st = conn.prepare(
        "SELECT item,value,source_level,COALESCE(source_ref,''),COALESCE(page,0)
         FROM specs_usable WHERE vehicle_id=?1 AND lower(item) LIKE ?2 ORDER BY item")?;
    let specs: Vec<SpecHit> = st.query_map(rusqlite::params![v.id, like], |r| Ok(SpecHit {
        item: r.get(0)?, value: r.get(1)?, source_level: r.get(2)?,
        source_ref: r.get(3)?, page: r.get(4)?,
    }))?.filter_map(|x| x.ok()).collect();
    let pending_related: i64 = conn.query_row(
        "SELECT COUNT(*) FROM specs WHERE vehicle_id=?1 AND pending=1 AND lower(item) LIKE ?2",
        rusqlite::params![v.id, like], |r| r.get(0)).unwrap_or(0);
    // 冲突：同一语义身份有多个已核实值。查不到时这是关键区分 ——
    // 「没有」和「两个来源打架、都被扣下」对用户是两回事。
    let mut cst = conn.prepare(
        "SELECT canonical, values_csv, sources_csv FROM spec_conflicts
         WHERE vehicle_id=?1 AND (lower(canonical) LIKE ?2 OR lower(items_csv) LIKE ?2)")?;
    let conflicts: Vec<SpecConflict> = cst.query_map(rusqlite::params![v.id, like], |r| Ok(SpecConflict {
        canonical: r.get(0)?, values: r.get(1)?, sources: r.get(2)?,
    }))?.filter_map(|x| x.ok()).collect();
    Ok(SpecResult { vehicle: v.id, query: query.to_string(), specs, pending_related, conflicts })
}

pub fn render_specs(r: &SpecResult) -> String {
    if r.specs.is_empty() {
        let mut t = format!("没有「{}」的已核实规格。", r.query);
        for c in &r.conflicts {
            t.push_str(&format!(
                "\n⚠️ **不是没有 —— 是冲突被扣下了**：{} 有互相矛盾的已核实值 [{}]（来源 {}）。\
                 两个都带出处，解决前一个都不能用。跑 `repair.gaps` 看它，或 `research.brief` 拿检索提纲。",
                c.canonical, c.values, c.sources));
        }
        if r.pending_related > 0 {
            t.push_str(&format!(
                "有 {} 条相关项**待核实**（C/D 级，不可用于作业）—— 不要用其他来源的数字代替。",
                r.pending_related));
        }
        return t;
    }
    let mut t = r.specs.iter().map(|s| format!("{} = {}  [{}级 · {} p{}]",
        s.item, s.value, s.source_level, s.source_ref, s.page))
        .collect::<Vec<_>>().join("\n");
    for c in &r.conflicts {
        t.push_str(&format!("\n⚠️ {} 另有冲突值被扣下 [{}] —— 未解决前不可用", c.canonical, c.values));
    }
    if r.pending_related > 0 {
        t.push_str(&format!("\n\n（另有 {} 条相关项待核实，未列出 —— 它们不可用于作业）",
            r.pending_related));
    }
    t
}

#[derive(Debug, Serialize)]
pub struct ManualHit {
    pub manual: String, pub tier: Option<String>,
    pub page: i64, pub snippet: String,
    /// 这一页的原图路径 —— 双层检索的下半截：
    /// 文本层定位到页，**数值回这张图确认**。页目录形态的手册才有。
    pub image: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub vehicle: String,
    pub query: String,
    pub hits: Vec<ManualHit>,
    /// 被判为不适用本车、因而不可能命中的资料份数
    pub excluded_manuals: i64,
    /// 因来源级别低于 A/B 而未返回的命中数
    pub low_tier_hidden: i64,
    /// 本次是否放行了 C 级
    pub include_low_tier: bool,
}

/// 手册检索。只从 manual_pages_applicable 出 —— 别代资料不可能命中。
pub fn manual_search(p: &Platform, vehicle: Option<&str>, query: &str, limit: i64)
    -> Result<SearchResult>
{
    manual_search_opts(p, vehicle, query, limit, false)
}

/// `include_low_tier=true` 才会返回 C 级资料。
///
/// 监工 2026-09-15 抓到的硬约束违反：手册的 tier 标签改对了，
/// **但代码零处按 tier 过滤** —— 查 torque 时 6/6 命中全是那份
/// 社区网站转录的 REP-ROM（C 级），它是唯一的结果源。
/// 规格侧早就用 `specs_usable` 视图把 C/D 挡住了，手册侧却没有同等待遇。
pub fn manual_search_opts(p: &Platform, vehicle: Option<&str>, query: &str, limit: i64,
                          include_low_tier: bool) -> Result<SearchResult>
{
    manual_search_full(p, vehicle, query, limit, include_low_tier, false)
}

/// `include_inapplicable=true` 会连不适用本车的资料一并返回（危险，仅用于排查）。
///
/// CLI 此前有一套**自己的**检索实现 —— 那正是 api 层本该消灭的漂移，
/// 也是监工抓到「CLI 说已排除 N 份、MCP 一字不提」的同一个根源。现在三条通道都走这里。
pub fn manual_search_full(p: &Platform, vehicle: Option<&str>, query: &str, limit: i64,
                          include_low_tier: bool, include_inapplicable: bool)
    -> Result<SearchResult>
{
    let v = p.resolve_vehicle(vehicle)?;
    let conn = db::open(&p.index_db())?;
    let tier_clause = if include_low_tier { "" } else { " AND m.tier IN ('A','B')" };
    let sql = if include_inapplicable {
        format!(
        "SELECT m.title, m.tier, mp.page_no, snippet(manual_fts,0,'[',']','…',14), mp.image_path
         FROM manual_fts f
         JOIN manual_pages mp ON mp.id=f.rowid
         JOIN manuals m ON m.id=mp.manual_id
         WHERE manual_fts MATCH ?1 AND m.vehicle_id=?2{tier_clause} ORDER BY rank LIMIT ?3")
    } else {
        format!(
        "SELECT m.title, m.tier, m.page_no, snippet(manual_fts,0,'[',']','…',14), m.image_path
         FROM manual_fts f JOIN manual_pages_applicable m ON m.id=f.rowid
         WHERE manual_fts MATCH ?1 AND m.vehicle_id=?2{tier_clause} ORDER BY rank LIMIT ?3")
    };
    let mut st = conn.prepare(&sql).context("检索语句准备失败")?;
    let hits: Vec<ManualHit> = st.query_map(rusqlite::params![query, v.id, limit], |r| Ok(ManualHit {
        manual: r.get(0)?, tier: r.get(1)?, page: r.get(2)?, snippet: r.get(3)?,
        image: r.get(4).ok(),
    })).context("FTS 语法有误？")?.filter_map(|x| x.ok()).collect();

    // 被级别挡住了多少 —— 必须说出来，否则「无结果」会被误读成「资料里没有」
    let low_tier_hidden: i64 = if include_low_tier { 0 } else {
        conn.prepare(
            "SELECT COUNT(*) FROM manual_fts f JOIN manual_pages_applicable m ON m.id=f.rowid
             WHERE manual_fts MATCH ?1 AND m.vehicle_id=?2 AND m.tier NOT IN ('A','B')")
            .and_then(|mut q| q.query_row(rusqlite::params![query, v.id], |r| r.get(0)))
            .unwrap_or(0)
    };
    let excluded_manuals: i64 = conn.query_row(
        "SELECT COUNT(*) FROM manuals WHERE vehicle_id=?1 AND applicable=0",
        [&v.id], |r| r.get(0)).unwrap_or(0);
    Ok(SearchResult { vehicle: v.id, query: query.to_string(), hits, excluded_manuals,
                      low_tier_hidden, include_low_tier })
}

pub fn render_search(r: &SearchResult) -> String {
    let mut t = if r.hits.is_empty() { "无结果".to_string() } else {
        r.hits.iter().map(|h| format!("[{}级] {} p{} — {}{}",
            h.tier.as_deref().unwrap_or("?"), h.manual, h.page,
            h.snippet.replace('\n', " "),
            h.image.as_deref().map(|i| format!("\n          回原图: {i}")).unwrap_or_default()))
            .collect::<Vec<_>>().join("\n")
    };
    if r.low_tier_hidden > 0 {
        t.push_str(&format!(concat!(
            "\n\n⚠️ 另有 {} 处命中来自 **C 级资料**（社区转录/未交叉验证），已按来源分级挡住。",
            "它们不可用于作业 —— 扭力值这类数字尤其不行。",
            "确实要看就显式索取（CLI: `--include-low-tier`；MCP/HTTP: `include_low_tier=true`），",
            "看到了也只能当线索，不能当依据。"), r.low_tier_hidden));
    }
    // 这句话必须三条通道都说 —— 调用方有权知道有资料被挡在门外
    if r.excluded_manuals > 0 {
        t.push_str(&format!(
            "\n\n（已自动排除 {} 份不适用本车的资料 —— 它们不可能出现在上面的结果里。数字请回原图核对。）",
            r.excluded_manuals));
    }
    t
}

#[derive(Debug, Serialize)]
pub struct LogRead { pub total: usize, pub entries: Vec<String>, pub file: String }

pub fn log_read(p: &Platform, vehicle: Option<&str>, limit: usize) -> Result<LogRead> {
    let v = p.resolve_vehicle(vehicle)?;
    let f = v.log_dir().join("LOG.md");
    let raw = std::fs::read_to_string(&f).unwrap_or_default();
    let mut entries: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in raw.lines() {
        if line.starts_with("## ") && !cur.trim().is_empty() {
            entries.push(cur.trim().to_string()); cur.clear();
        }
        cur.push_str(line); cur.push('\n');
    }
    if !cur.trim().is_empty() { entries.push(cur.trim().to_string()); }
    entries.reverse();
    let total = entries.len();
    let shown = entries.into_iter().take(limit)
        // 幂等标记是内部机制，不占用读者的注意力
        .map(|e| e.lines().filter(|l| !l.trim().starts_with("<!-- idem:"))
                  .collect::<Vec<_>>().join("\n"))
        .collect();
    Ok(LogRead { total, entries: shown, file: f.to_string_lossy().to_string() })
}

#[derive(Debug, Serialize)]
pub struct AppendResult { pub written: bool, pub idempotency_key: String, pub file: String }

/// 写日志。**文件是真相源** —— 写 Markdown，不写数据库。
pub fn log_append(p: &Platform, vehicle: Option<&str>, text: &str, phase: &str, key: &str)
    -> Result<AppendResult>
{
    if key.is_empty() || key.chars().any(|c| c.is_whitespace()) || key.len() > 200 {
        anyhow::bail!("idempotency_key 必填，且不得含空白、不超过 200 字节");
    }
    // 幂等键投毒：夹带标记语法可污染别人的键，让别人的真实记录被静默吞掉。
    // 监工 2026-09-14-c：第一版只过滤 text，**漏了 phase** ——
    // `phase: "P1\n<!-- idem:victim -->"` 照样投毒。
    // 所以检查的对象不是「正文」，是**一切会被写进文件的字段**。
    for (label, val) in [("正文", text), ("phase", phase)] {
        let squashed: String = val.chars().filter(|c| !c.is_whitespace()).collect();
        if squashed.contains("<!--idem:") {
            anyhow::bail!("{label} 里不能出现 `<!-- idem:` —— 那是幂等标记语法，\
                           写进去会污染别人的幂等键。请改写。");
        }
    }
    // phase 还要防换行：它只占标题行，夹带换行等于伪造一条新记录
    if phase.contains('\n') || phase.contains('\r') {
        anyhow::bail!("phase 不能含换行 —— 它只占标题的一部分");
    }
    let v = p.resolve_vehicle(vehicle)?;
    let dir = v.log_dir();
    std::fs::create_dir_all(&dir)?;
    let f = dir.join("LOG.md");
    let existing = std::fs::read_to_string(&f).unwrap_or_default();
    let marker = format!("<!-- idem:{key} -->");
    // 整行精确匹配，不做子串扫描
    if existing.lines().any(|l| l.trim() == marker) {
        return Ok(AppendResult { written: false, idempotency_key: key.into(),
                                 file: f.to_string_lossy().to_string() });
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
    let entry = format!("\n## {stamp}{}\n{marker}\n\n{text}\n",
        if phase.is_empty() { String::new() } else { format!(" · {phase}") });
    use std::io::Write;
    let mut h = std::fs::OpenOptions::new().create(true).append(true).open(&f)?;
    h.write_all(entry.as_bytes())?;
    Ok(AppendResult { written: true, idempotency_key: key.into(),
                      file: f.to_string_lossy().to_string() })
}

/// 车辆列表（带发动机代号）
#[derive(Debug, Serialize)]
pub struct VehicleBrief { pub id: String, pub engine_code: Option<String>, pub year: Option<i64> }

pub fn vehicles(p: &Platform) -> Result<Vec<VehicleBrief>> {
    let conn = db::open(&p.index_db())?;
    let mut out = Vec::new();
    for id in p.list_vehicles()? {
        // 「这台车还没索引」是正常状态，「查询坏了」不是 —— 两者必须分开。
        // 一律 unwrap_or(None) 会把后者伪装成前者，正是 doctor 长出假绿灯的同一个毛病。
        let row = conn.query_row(
            "SELECT engine_code, year FROM vehicles WHERE id=?1", [&id],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?)));
        let (engine_code, year) = match row {
            Ok(t) => t,
            Err(rusqlite::Error::QueryReturnedNoRows) => (None, None),
            Err(e) => return Err(anyhow::anyhow!("查车辆 {id} 失败: {e}")),
        };
        out.push(VehicleBrief { id, engine_code, year });
    }
    Ok(out)
}

/// 已知故障模式（读 VEHICLE.yml，文件是真相源）
pub fn known_issues(p: &Platform, vehicle: Option<&str>) -> Result<serde_json::Value> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    Ok(doc.get("known_issues").cloned()
        .map(|k| serde_json::to_value(&k).unwrap_or_default())
        .unwrap_or(serde_json::json!([])))
}

/// 未执行召回
pub fn recalls(p: &Platform, vehicle: Option<&str>) -> Result<Vec<workbench::Recall>> {
    Ok(workbench::build(p, vehicle, None)?.open_recalls)
}

// ── Vlog ────────────────────────────────────────────────────
//
// Vlog 数据留在纪录片项目里（VEHICLE.yml 的 vlog_path），**不搬进平台** ——
// 片子的归片子。这里只读、只索引，不写。

#[derive(Debug, Serialize)]
pub struct Topic {
    pub id: String, pub title: String, pub phase: String,
    /// 一次性素材（拍不到就永远没有），选题库里用 🔴 标
    pub one_shot: bool,
}

#[derive(Debug, Serialize)]
pub struct Animation { pub name: String, pub bytes: u64, pub path: String }

#[derive(Debug, Serialize)]
pub struct VlogView {
    pub vlog_path: String,
    pub exists: bool,
    pub topics: Vec<Topic>,
    pub one_shot_count: usize,
    pub animations: Vec<Animation>,
    pub docs: Vec<String>,
    pub scripts: Vec<String>,
    /// 素材库已打标签的张数
    pub tagged_media: usize,
    pub notes: Vec<String>,
}

pub fn vlog(p: &Platform, vehicle: Option<&str>) -> Result<VlogView> {
    let v = p.resolve_vehicle(vehicle)?;
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(v.yml())?)?;
    let raw = doc.get("vlog_path").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // ~ 展开：YAML 里写的是 ~ 开头的路径
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        dirs_home().join(rest)
    } else { std::path::PathBuf::from(&raw) };

    let mut out = VlogView {
        vlog_path: expanded.to_string_lossy().to_string(),
        exists: expanded.is_dir(),
        topics: vec![], one_shot_count: 0, animations: vec![],
        docs: vec![], scripts: vec![], tagged_media: 0, notes: vec![],
    };
    if !out.exists {
        out.notes.push(format!("vlog_path 不存在：{}（VEHICLE.yml 里写的是 {raw}）",
                               out.vlog_path));
        return Ok(out);
    }

    // 选题库：## Phase N · xxx 下面的 ### [🔴] Sxx「标题」
    let tl = expanded.join("选题库.md");
    if let Ok(t) = std::fs::read_to_string(&tl) {
        let mut phase = String::new();
        for line in t.lines() {
            if let Some(h) = line.strip_prefix("## ") {
                if h.contains("Phase") { phase = h.split('·').next().unwrap_or(h).trim().to_string(); }
                continue;
            }
            if let Some(h) = line.strip_prefix("### ") {
                let one_shot = h.contains('🔴');
                let body = h.replace('🔴', "");
                let body = body.trim();
                let (id, title) = match body.find('「') {
                    Some(i) => (body[..i].trim().to_string(),
                                body[i..].trim_matches(|c| c=='「'||c=='」').to_string()),
                    None => (String::new(), body.to_string()),
                };
                if one_shot { out.one_shot_count += 1; }
                out.topics.push(Topic { id, title, phase: phase.clone(), one_shot });
            }
        }
    } else {
        out.notes.push("没有 选题库.md".into());
    }

    // 图解动画：只列成品 mp4
    let anim = expanded.join("图解动画");
    if anim.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&anim) {
            for e in rd.flatten() {
                let pth = e.path();
                if pth.extension().map(|x| x == "mp4").unwrap_or(false) {
                    out.animations.push(Animation {
                        name: pth.file_name().unwrap_or_default().to_string_lossy().to_string(),
                        bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                        path: pth.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
    out.animations.sort_by(|a, b| a.name.cmp(&b.name));

    for (dir, sink) in [(expanded.clone(), &mut out.docs), (expanded.join("脚本"), &mut out.scripts)] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let pth = e.path();
                if pth.extension().map(|x| x == "md").unwrap_or(false) {
                    sink.push(pth.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
            }
        }
    }
    out.docs.sort(); out.scripts.sort();
    if out.scripts.is_empty() { out.notes.push("脚本/ 还是空的（计划里的 E6）".into()); }

    // 素材标签
    let tags = expanded.join("素材库/tags.json");
    if let Ok(t) = std::fs::read_to_string(&tags) {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(&t) {
            out.tagged_media = j.as_object().map(|o| o.len()).unwrap_or(0);
        }
    }
    Ok(out)
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_default()
}
