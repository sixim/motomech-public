// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! motomech —— 一个二进制，四种模式（形态对齐 somadisk）
//!
//!   motomech            启动桌面 App
//!   motomech serve      HTTP API（headless，给 soma 等 agent 调）
//!   motomech mcp        stdio MCP server（给 Claude Code / Hermes 调）
//!   motomech <子命令>    CLI
//!
//! 数据真相源始终是文件；SQLite 只是可重建索引。

mod cli;
use motomech_lib::{db, index, platform};

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use platform::Platform;

fn main() {
    let args = Cli::parse();
    if let Err(e) = run(args) {
        eprintln!("❌ {e:#}");
        std::process::exit(1);
    }
}

fn run(args: Cli) -> Result<()> {
    match args.command {
        // 无子命令 → 桌面 App
        None => {
            motomech_lib::run();
            Ok(())
        }
        Some(Commands::Safety { action, selftest }) => {
            let p = Platform::new(args.root.clone())?;
            if selftest {
                let t = motomech_lib::safety::selftest(&p, args.vehicle.as_deref())?;
                println!("门禁回归用例：{}/{} 通过", t.passed, t.total);
                for f in &t.failures { println!("  ❌ {f}"); }
                if t.failures.is_empty() { println!("✅ 既没漏拦，也没误报"); }
                else { std::process::exit(1); }
                return Ok(());
            }
            if action.trim().is_empty() {
                anyhow::bail!("要判定什么动作？用法: motomech safety \"打火试试\"  或  --selftest");
            }
            let r = motomech_lib::safety::evaluate(&p, args.vehicle.as_deref(), &action)?;
            print!("{}", motomech_lib::safety::render(&r));
            // BLOCK 时以非零退出码收场 —— 让脚本也能拦住，不只是人眼看
            if r.verdict == "BLOCK" { std::process::exit(2); }
            Ok(())
        }
        Some(Commands::Briefing { phase }) => {
            let p = Platform::new(args.root.clone())?;
            let ph = if phase.trim().is_empty() {
                motomech_lib::workbench::build(&p, args.vehicle.as_deref(), None)?.current_phase
            } else { phase };
            print!("{}", motomech_lib::safety::briefing(&p, &ph)?);
            Ok(())
        }
        Some(Commands::Parts { days_per_phase }) => {
            let p = Platform::new(args.root.clone())?;
            let pp = motomech_lib::parts::plan(&p, args.vehicle.as_deref(), days_per_phase)?;
            print!("{}", motomech_lib::parts::render(&pp));
            Ok(())
        }
        Some(Commands::Tool { id, args: kv }) => {
            let p = Platform::new(args.root.clone())?;
            if id.trim().is_empty() {
                for t in motomech_lib::tools::load(&p)? {
                    let tag = if t.kind == "link" { "🔗 只能给链接" } else { "⚙️  可直接调用" };
                    println!("{tag}  [{}级] {}", t.source_level, t.id);
                    println!("      {}", t.desc);
                    println!("      参数: {}\n", t.params.join(" "));
                }
                return Ok(());
            }
            let mut m = std::collections::HashMap::new();
            for a in &kv {
                match a.split_once('=') {
                    Some((k, v)) => { m.insert(k.to_string(), v.to_string()); }
                    None => anyhow::bail!("参数要写成 k=v 的形式，收到「{a}」"),
                }
            }
            let r = motomech_lib::tools::call(&p, &id, &m)?;
            print!("{}", motomech_lib::tools::render(&r));
            Ok(())
        }
        Some(Commands::Scripts) => {
            let p = Platform::new(args.root.clone())?;
            println!("平台根 {}\n", p.root.display());
            for s in motomech_lib::tools::scripts(&p)? {
                println!("  {}", s.cmd);
                if !s.desc.is_empty() { println!("      {}", s.desc); }
            }
            println!("\n（只列出，不代为执行 —— 从 App 里 shell out 会开一个很大的攻击面）");
            Ok(())
        }
        Some(Commands::Spec { query }) => {
            let p = Platform::new(args.root.clone())?;
            let r = motomech_lib::api::specs(&p, args.vehicle.as_deref(), &query)?;
            println!("车辆: {}\n", r.vehicle);
            println!("{}", motomech_lib::api::render_specs(&r));
            Ok(())
        }
        Some(Commands::Acquire) => {
            let p = Platform::new(args.root.clone())?;
            let v = p.resolve_vehicle(args.vehicle.as_deref())?;
            println!("车辆：{}\n", v.id);
            let a = motomech_lib::acquire::build(&p, &v)?;
            print!("{}", motomech_lib::acquire::render(&a));
            Ok(())
        }
        Some(Commands::Script) => {
            let p = Platform::new(args.root.clone())?;
            let v = p.resolve_vehicle(args.vehicle.as_deref())?;
            let r = motomech_lib::script::check(&p, &v)?;
            print!("{}", motomech_lib::script::render(&r));
            Ok(())
        }
        Some(Commands::Circuit { query, all }) => {
            let p = Platform::new(args.root.clone())?;
            let v = p.resolve_vehicle(args.vehicle.as_deref())?;
            let q = if query.trim().is_empty() { None } else { Some(query.as_str()) };
            let r = motomech_lib::circuit::build(&p, &v, q, all)?;
            print!("{}", motomech_lib::circuit::render(&r));
            Ok(())
        }
        Some(Commands::Crossval) => {
            let p = Platform::new(args.root.clone())?;
            let v = p.resolve_vehicle(args.vehicle.as_deref())?;
            println!("车辆：{}\n", v.id);
            let r = motomech_lib::crossval::build(&p, &v)?;
            print!("{}", motomech_lib::crossval::render(&r));
            Ok(())
        }
        Some(Commands::Gaps) => {
            let p = Platform::new(args.root.clone())?;
            let r = motomech_lib::gaps::find(&p, args.vehicle.as_deref())?;
            print!("{}", motomech_lib::gaps::render(&r));
            Ok(())
        }
        Some(Commands::Coldstart { vin, apply }) => {
            let p = Platform::new(args.root.clone())?;
            let c = if vin.trim().is_empty() {
                motomech_lib::coldstart::from_vehicle(&p, args.vehicle.as_deref(), apply)?
            } else {
                motomech_lib::coldstart::run(&p, &vin, apply)?
            };
            print!("{}", motomech_lib::coldstart::render(&c));
            Ok(())
        }
        Some(Commands::Research { cmd }) => {
            use motomech_lib::research as R;
            let p = Platform::new(args.root.clone())?;
            let v = args.vehicle.as_deref();
            match cmd {
                cli::ResearchCmd::Brief { gap_id } =>
                    print!("{}", R::render_brief(&R::brief(&p, v, &gap_id)?)),
                cli::ResearchCmd::Add { gap_id, url, quote, note, key } => {
                    let (c, fresh) = R::add(&p, v, &gap_id, &url, &quote, &note, &key)?;
                    println!("{} 线索 {}（C 级 · 不可用于作业）\n   {}",
                             if fresh { "✅ 已收" } else { "幂等：已存在" }, c.id, c.url);
                }
                cli::ResearchCmd::List => print!("{}", R::render_clues(&R::load(&p, v)?)),
            }
            Ok(())
        }
        Some(Commands::Diag { tree }) => {
            let p = Platform::new(args.root.clone())?;
            if tree.trim().is_empty() {
                for (id, sym) in motomech_lib::diag::list(&p)? {
                    println!("  {id}\n    {sym}");
                }
            } else {
                print!("{}", motomech_lib::diag::render(
                    &motomech_lib::diag::load(&p, args.vehicle.as_deref(), &tree)?));
            }
            Ok(())
        }
        Some(Commands::Symptom { text }) => {
            let p = Platform::new(args.root.clone())?;
            print!("{}", motomech_lib::symptom::render(
                &motomech_lib::symptom::diagnose(&p, args.vehicle.as_deref(), &text)?));
            Ok(())
        }
        Some(Commands::Vehicles) => cmd_vehicles(args.root),
        Some(Commands::Doctor) => cmd_doctor(args.root, args.vehicle.as_deref()),
        Some(Commands::Index { all }) => cmd_index(args.root, args.vehicle.as_deref(), all),
        Some(Commands::Search { query, include_inapplicable, include_low_tier, limit }) =>
            cmd_search(args.root, args.vehicle.as_deref(), &query, include_inapplicable,
                       include_low_tier, limit),
        Some(Commands::Serve { host, port }) => cmd_serve(args.root, host, port),
        Some(Commands::Mcp) => cmd_mcp(args.root),
        Some(Commands::Providers { set, no_probe }) => {
            let p = Platform::new(args.root.clone())?;
            if let Some(id) = set {
                motomech_lib::provider::set_default(&p.root, &id)?;
                println!("✅ 默认通道已改为 {id}");
            }
            let (list, def) = motomech_lib::provider::load(&p.root)?;
            if no_probe {
                for x in &list {
                    println!("{} {:<12} [{}] {}", if x.id == def { "→" } else { " " },
                             x.id, x.kind, x.model.as_deref().unwrap_or("(未指定)"));
                }
            } else {
                let h = motomech_lib::provider::health(&list);
                print!("{}", motomech_lib::provider::render_health(&h, &def));
            }
            Ok(())
        }
        Some(Commands::Ask { question, provider, show_prompt }) =>
            cmd_ask(args.root, args.vehicle.as_deref(), &question, provider.as_deref(), show_prompt),
        Some(Commands::Workbench { phase, json }) =>
            cmd_workbench(args.root, args.vehicle.as_deref(), phase.as_deref(), json),
    }
}

fn cmd_vehicles(root: Option<String>) -> Result<()> {
    let p = Platform::new(root)?;
    let current = std::fs::read_to_string(p.root.join(".current-vehicle"))
        .ok()
        .map(|s| s.trim().to_string());
    println!("平台: {}", p.root.display());
    for v in p.list_vehicles()? {
        let mark = if Some(&v) == current.as_ref() { "  ← 当前" } else { "" };
        println!("  · {v}{mark}");
    }
    Ok(())
}

fn cmd_doctor(root: Option<String>, vehicle: Option<&str>) -> Result<()> {
    let p = Platform::new(root)?;
    println!("平台根   : {}", p.root.display());
    println!("车辆目录 : {}", p.vehicles_dir().display());
    println!("索引库   : {}", p.index_db().display());

    // 硬约束：每个操作都打印当前车辆，绝不猜
    let mut doctor_ok = true;
    match p.resolve_vehicle(vehicle) {
        Ok(v) => {
            println!("当前车辆 : {}", v.id);
            println!("  VEHICLE.yml : {}", yn(v.yml().exists()));
            println!("  手册资料    : {}", yn(v.manuals_dir().exists()));
            println!("  影像        : {}", yn(v.photos_dir().exists()));
            println!("  规格数据    : {}", yn(v.specs_dir().exists()));
        }
        Err(e) => { println!("当前车辆 : ❌ {e}"); doctor_ok = false; }
    }

    // 符号链接披露：不禁止（外置盘链接进来是合理用法，且能建链接的人
    // 本来就能直接改 VEHICLE.yml），但**必须说出来** ——
    // 监工 2026-09-15 指出：目标内容改动 git 完全无感，普通目录做不到这点。
    // 禁它不增加 access 安全性，但不说它会损失 auditability。
    if let Ok(list) = p.list_vehicles() {
        for id in &list {
            let d = p.vehicles_dir().join(id);
            if std::fs::symlink_metadata(&d).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                let target = std::fs::read_link(&d).map(|t| t.display().to_string())
                    .unwrap_or_else(|_| "?".into());
                println!("⚠️  车辆 {id} 是符号链接 → {target}");
                println!("    内容不在本仓库里，**git 看不到它的改动** —— 版本可追溯性到此为止");
            }
        }
    }

    let conn = db::open(&p.index_db())?;
    match db::db_schema_version(&conn) {
        Some(v) if v == db::SCHEMA_VERSION => println!("索引库 schema v{v}"),
        Some(v) => println!("索引库 schema v{v} ⚠️ 程序需要 v{}（须删 .index/ 重建）", db::SCHEMA_VERSION),
        None => println!("索引库 schema 版本未知"),
    }
    for (t, n) in db::table_counts(&conn)? {
        println!("  {t:<24} {n:>6}");
    }

    // 索引是否已陈旧：真相源是文件，文件改了而索引没重建，查询结果就是过期的。
    // 此前 doctor 对这种情况一律报绿 —— 监工 2026-09-14-b 抓到 MCP 写完日志不重索引，
    // doctor 却全绿。假绿灯比红灯更危险。
    //
    // 做法：拿「上次建索引的时刻」与每个源文件的 mtime 比。
    // 判据统一，不依赖各表是否都有 indexed_at 列（manuals 的路径列叫 file_path 不叫
    // source_path —— 第一版就栽在这里：SQL prepare 失败被 `if let Ok` 吞掉，
    // 查了 0 个文件却打印「✅ 均不比索引新」。**吞掉错误正是假绿灯的来源。**）
    {
        let built_at: Option<String> = conn.query_row(
            "SELECT MAX(indexed_at) FROM specs", [], |r| r.get(0)).ok().flatten();
        match built_at {
            None => println!("索引新鲜度 : ⚠️  索引里没有任何时间戳，无法判断（跑 `index --all`）"),
            Some(built) => {
                let mut paths: Vec<String> = Vec::new();
                let mut sql_err: Vec<String> = Vec::new();
                for sql in ["SELECT DISTINCT source_path FROM specs WHERE source_path IS NOT NULL",
                            "SELECT DISTINCT file_path   FROM manuals WHERE file_path IS NOT NULL",
                            "SELECT DISTINCT source_path FROM log_entries WHERE source_path IS NOT NULL"] {
                    match conn.prepare(sql).and_then(|mut st| {
                        let v = st.query_map([], |r| r.get::<_, String>(0))?
                                  .collect::<std::result::Result<Vec<_>, _>>()?;
                        Ok(v)
                    }) {
                        Ok(v) => paths.extend(v),
                        // 查不动就说查不动，绝不当成「没有陈旧文件」
                        Err(e) => sql_err.push(format!("{sql} → {e}")),
                    }
                }
                paths.sort(); paths.dedup();
                // ⚠️ 时间必须解析后比较，不能比字符串：
                // indexed_at 存的是 UTC（…T23:38:46+00:00），而文件 mtime 取本地时间，
                // 本地时间的字符串永远「小于」UTC 字符串 —— 第一版就这么恒报绿。
                let built_dt = chrono::DateTime::parse_from_rfc3339(&built)
                    .map(|d| d.with_timezone(&chrono::Utc));
                let built_dt = match built_dt {
                    Ok(d) => d,
                    Err(e) => {
                        println!("索引新鲜度 : ❌ 索引时间戳解析失败（{built} → {e}），结果不可信");
                        return Ok(());
                    }
                };
                let mut stale: Vec<String> = Vec::new();
                for path in &paths {
                    match std::fs::metadata(path) {
                        Ok(md) => if let Ok(m) = md.modified() {
                            let mt: chrono::DateTime<chrono::Utc> = m.into();
                            if mt > built_dt {
                                stale.push(format!("{} 改于 {}，索引建于 {}",
                                    std::path::Path::new(path).file_name()
                                        .map(|x| x.to_string_lossy().to_string())
                                        .unwrap_or_else(|| path.clone()),
                                    mt.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S%.3f"),
                                    built_dt.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S%.3f")));
                            }
                        },
                        Err(_) => stale.push(format!("{path} —— 源文件已不存在，索引里还留着")),
                    }
                }
                if !sql_err.is_empty() {
                    println!("索引新鲜度 : ❌ 检查本身失败了，结果不可信：");
                    for e in &sql_err { println!("    · {e}"); }
                } else if stale.is_empty() {
                    println!("索引新鲜度 : ✅ {} 个源文件均不比索引新（索引建于 {}）",
                             paths.len(), built_dt.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S%.3f"));
                } else {
                    println!("索引新鲜度 : ⚠️  {}/{} 处陈旧（跑 `motomech index --all` 重建）",
                             stale.len(), paths.len());
                    for x in &stale { println!("    · {x}"); }
                }
            }
        }
    }

    // 过滤规则是否已过期：YAML 改了排除项但没重新索引，过滤结果仍是旧的
    // ⚠️ 这里三层 `if let Ok` 曾把「VEHICLE.yml 坏了」变成「这一行干脆不打印」，
    //    而 doctor 仍以 0 退出 —— 同一个文件 workbench 是直接拒绝解析的。
    //    体检工具对着一个坏掉的真相源报健康，是最坏的一种假绿灯（监工 2026-09-14-c）。
    if let Ok(v) = p.resolve_vehicle(vehicle) {
        let parsed = std::fs::read_to_string(v.yml())
            .map_err(|e| format!("读不到 {}: {e}", v.yml().display()))
            .and_then(|raw| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&raw)
                .map_err(|e| format!("{} 解析失败: {e}", v.yml().display())));
        match parsed {
            Err(msg) => {
                println!("\n适用性规则  ❌ 无法检查 —— {msg}");
                println!("            车辆档案是真相源，它坏了，下面所有结论都不可信");
                doctor_ok = false;
            }
            Ok(doc) => {
                // 空文件解析成 null，合法 YAML 但缺关键字段同样不可用 ——
                // 体检工具对着一个没内容的真相源报健康，仍然是假绿灯（监工 2026-09-15）
                let missing: Vec<&str> = ["id", "make", "model", "year", "engine_code"]
                    .into_iter().filter(|k| doc.get(*k).is_none()).collect();
                if !missing.is_empty() {
                    println!("\n车辆档案  ❌ 缺关键字段：{} —— 档案还不可用", missing.join(" "));
                    doctor_ok = false;
                }
                let cur: Vec<String> = doc.get("applicability_excludes")
                    .and_then(|x| x.as_sequence())
                    .map(|l| l.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let want = index::excludes_fingerprint(&cur);
                let have: Option<String> = conn.query_row(
                    "SELECT excludes_fp FROM vehicles WHERE id=?1", [&v.id], |r| r.get(0)).ok();
                match have {
                    Some(h) if h == want => println!("\n适用性规则  ✅ 与 VEHICLE.yml 一致"),
                    Some(_) => println!("\n适用性规则  ⚠️ **已过期** —— VEHICLE.yml 的排除项改过但未重新索引\n            过滤结果仍是旧的，跑 `motomech index --all` 更新"),
                    None => println!("\n适用性规则  ⚠️ 未记录指纹（该车尚未索引）"),
                }
            }
        }
    }

    // 安全视图是否在位
    let views: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='view'
         AND name IN ('specs_usable','manual_pages_applicable')", [], |r| r.get(0))?;
    println!("安全视图    {} (specs_usable / manual_pages_applicable)",
             if views == 2 { "✅ 2/2" } else { "❌ 缺失" });

    println!("\n提示：索引可删可重建 —— 文件才是真相源。");
    if !doctor_ok {
        // 体检不通过就以非零退出 —— 让脚本和 CI 也能看见，不只是人眼
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_index(root: Option<String>, vehicle: Option<&str>, all: bool) -> Result<()> {
    let p = Platform::new(root)?;
    let mut conn = db::open(&p.index_db())?;

    let targets: Vec<String> = if all {
        p.list_vehicles()?
    } else {
        vec![p.resolve_vehicle(vehicle)?.id]
    };

    let t0 = std::time::Instant::now();
    let mut tot = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for id in &targets {
        let v = p.resolve_vehicle(Some(id))?;
        println!("车辆: {}", v.id); // 硬约束：打印当前车辆
        let st = index::index_vehicle(&mut conn, &p, &v)?;
        println!("  排除项 {:<3} 规格A {:<3} 待核实 {:<3} 手册 {:<2}(不适用 {}) 页 {:<4} 照片 {:<3} 日志 {}",
                 st.excludes, st.specs_verified, st.specs_pending,
                 st.manuals, st.manuals_excluded, st.manual_pages, st.photos, st.logs);
        tot.0 += st.excludes; tot.1 += st.specs_verified; tot.2 += st.specs_pending;
        tot.3 += st.manuals; tot.4 += st.manual_pages; tot.5 += st.photos;
        tot.6 += st.logs; tot.7 += st.manuals_excluded;
    }
    if targets.len() > 1 {
        println!("\n合计 {} 台车 | 排除项 {} 规格A {} 待核实 {} 手册 {}(不适用 {}) 页 {} 照片 {} 日志 {}",
                 targets.len(), tot.0, tot.1, tot.2, tot.3, tot.7, tot.4, tot.5, tot.6);
    }
    println!("耗时 {:.1}s  索引库 {}", t0.elapsed().as_secs_f64(), p.index_db().display());
    if !all && p.list_vehicles()?.len() > 1 {
        println!("提示：本次只重建了这一台。删库后完整恢复请用 `index --all`");
    }
    Ok(())
}

/// 手册检索 —— **默认排除不适用本车的资料**。
/// 这是 A5 适用性过滤的消费端：没有它，excludes 表就只是写进去没人读。
fn cmd_search(root: Option<String>, vehicle: Option<&str>, query: &str,
              include_inapplicable: bool, include_low_tier: bool, limit: usize) -> Result<()> {
    // 全部委派给 api 层 —— CLI 此前有自己一套检索实现，
    // 那正是「同一查询三条通道说法不同」的根源（监工 2026-09-14-b / 09-15）。
    let p = Platform::new(root)?;
    let r = motomech_lib::api::manual_search_full(
        &p, vehicle, query, limit as i64, include_low_tier, include_inapplicable)?;
    println!("车辆: {}\n检索: {}\n", r.vehicle, r.query);
    print!("{}", motomech_lib::api::render_search(&r));
    println!();
    Ok(())
}

fn cmd_serve(root: Option<String>, host: String, port: u16) -> Result<()> {
    let p = Platform::new(root)?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        use axum::{extract::Query, routing::{get, post}, Json, Router};
        use motomech_lib::{api, parts, safety, workbench};
        use std::collections::HashMap;

        let root_s = p.root.display().to_string();
        // 每个 handler 各拿一份 root：Platform 不是 Send/Sync 友好的长驻对象，
        // 每次请求现建一个，代价可以忽略（只是路径解析），换来无共享状态。
        macro_rules! pf {
            ($r:expr) => {
                match Platform::new(Some($r)) {
                    Ok(x) => x,
                    Err(e) => return Json(serde_json::json!({"error": e.to_string()})),
                }
            };
        }
        macro_rules! reply {
            ($e:expr) => {
                match $e {
                    Ok(v) => Json(serde_json::to_value(v).unwrap_or_default()),
                    Err(e) => Json(serde_json::json!({"error": format!("{:#}", e)})),
                }
            };
        }
        fn v<'a>(q: &'a HashMap<String, String>) -> Option<&'a str> {
            q.get("vehicle").map(|s| s.as_str())
        }

        let mut app = Router::new()
            .route("/health", get(|| async { Json(serde_json::json!({"ok": true})) }));

        // ── 只读 ──────────────────────────────────────────
        let r = root_s.clone();
        app = app.route("/v1/vehicles", get(move || {
            let r = r.clone();
            async move { let pf = pf!(r); reply!(api::vehicles(&pf)) }
        }));

        let r = root_s.clone();
        app = app.route("/v1/workbench", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                reply!(workbench::build(&pf, v(&q), q.get("phase").map(|s| s.as_str())))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/specs", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let query = q.get("q").map(|s| s.as_str()).unwrap_or("");
                reply!(api::specs(&pf, v(&q), query))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/search", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let query = q.get("q").map(|s| s.as_str()).unwrap_or("");
                let lim = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(5);
                let low = q.get("include_low_tier").map(|s| s == "true" || s == "1").unwrap_or(false);
                reply!(api::manual_search_opts(&pf, v(&q), query, lim, low))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/recalls", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move { let pf = pf!(r); reply!(api::recalls(&pf, v(&q))) }
        }));

        let r = root_s.clone();
        app = app.route("/v1/known_issues", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move { let pf = pf!(r); reply!(api::known_issues(&pf, v(&q))) }
        }));

        let r = root_s.clone();
        app = app.route("/v1/parts", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let d = q.get("days_per_phase").and_then(|s| s.parse().ok()).unwrap_or(7);
                reply!(parts::plan(&pf, v(&q), d))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/briefing", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let ph = match q.get("phase") {
                    Some(x) if !x.trim().is_empty() => x.clone(),
                    _ => match workbench::build(&pf, v(&q), None) {
                        Ok(w) => w.current_phase,
                        Err(e) => return Json(serde_json::json!({"error": format!("{e:#}")})),
                    },
                };
                match safety::briefing(&pf, &ph) {
                    Ok(t) => Json(serde_json::json!({"phase": ph, "markdown": t})),
                    Err(e) => Json(serde_json::json!({"error": format!("{e:#}")})),
                }
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/gaps", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move { let pf = pf!(r); reply!(motomech_lib::gaps::find(&pf, v(&q))) }
        }));

        let r = root_s.clone();
        app = app.route("/v1/symptom", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let t = q.get("text").map(|s| s.as_str()).unwrap_or("");
                reply!(motomech_lib::symptom::diagnose(&pf, v(&q), t))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/diag", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                match q.get("tree").map(|s| s.as_str()).filter(|s| !s.is_empty()) {
                    Some(t) => reply!(motomech_lib::diag::load(&pf, v(&q), t)),
                    None => reply!(motomech_lib::diag::list(&pf)),
                }
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/vlog", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move { let pf = pf!(r); reply!(api::vlog(&pf, v(&q))) }
        }));

        let r = root_s.clone();
        app = app.route("/v1/log", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let lim = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(10);
                reply!(api::log_read(&pf, v(&q), lim))
            }
        }));

        // ── 安全门禁：GET 是查询（判定一个动作），不改变任何状态 ──
        let r = root_s.clone();
        app = app.route("/v1/safety", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let action = q.get("action").map(|s| s.as_str()).unwrap_or("");
                if action.trim().is_empty() {
                    return Json(serde_json::json!({"error":"缺少 action 参数：要判定什么动作？"}));
                }
                reply!(safety::evaluate(&pf, v(&q), action))
            }
        }));

        // ── 作业单：**先过门禁**，BLOCK 时不返回步骤 ──
        // 与 MCP 的 repair.job_card 同源：拦截在 workbench::build 里，
        // 不是在各调用点分别写一遍（那样必然漏一条）。
        let r = root_s.clone();
        app = app.route("/v1/job_card", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let intent = q.get("intent").map(|s| s.as_str()).unwrap_or("");
                if intent.trim().is_empty() {
                    return Json(serde_json::json!({
                        "error":"缺少 intent 参数 —— 门禁要拿它做判定，不能省"}));
                }
                let sr = match safety::evaluate(&pf, v(&q), intent) {
                    Ok(x) => x,
                    Err(e) => return Json(serde_json::json!({"error": format!("{e:#}")})),
                };
                if !sr.may_return_steps {
                    return Json(serde_json::json!({
                        "blocked": true, "tasks": [],
                        "safety": sr,
                        "text": safety::render(&sr),
                    }));
                }
                match workbench::build(&pf, v(&q), q.get("phase").map(|s| s.as_str())) {
                    Ok(w) => Json(serde_json::json!({
                        "blocked": w.phase_locked,
                        "phase_locked": w.phase_locked,
                        "phase_lock_reason": w.phase_lock_reason,
                        "phase": w.current_phase, "gates": w.gates, "tasks": w.tasks,
                        "safety": sr,
                    })),
                    Err(e) => Json(serde_json::json!({"error": format!("{e:#}")})),
                }
            }
        }));

        // ── 写：日志追加。POST，且必须带幂等键 ──
        let r = root_s.clone();
        app = app.route("/v1/log/append", post(move |Json(body): Json<serde_json::Value>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let g = |k: &str| body.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                reply!(api::log_append(&pf, body.get("vehicle").and_then(|x| x.as_str()),
                                       &g("text"), &g("phase"), &g("idempotency_key")))
            }
        }));

        // ── GUI 新能力的 HTTP 对应（浏览器路径必须和 Tauri invoke 同源）──
        let r = root_s.clone();
        app = app.route("/v1/circuit", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                let all = q.get("all").map(|s| s == "true" || s == "1").unwrap_or(false);
                let qq = q.get("query").map(|s| s.as_str()).filter(|s| !s.trim().is_empty());
                reply!(pf.resolve_vehicle(v(&q))
                    .and_then(|veh| motomech_lib::circuit::build(&pf, &veh, qq, all)))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/crossval", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                reply!(pf.resolve_vehicle(v(&q))
                    .and_then(|veh| motomech_lib::crossval::build(&pf, &veh)))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/script_check", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                reply!(pf.resolve_vehicle(v(&q))
                    .and_then(|veh| motomech_lib::script::check(&pf, &veh)))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/acquire", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = pf!(r);
                reply!(pf.resolve_vehicle(v(&q))
                    .and_then(|veh| motomech_lib::acquire::build(&pf, &veh)))
            }
        }));

        // ── 手机直采（Tailscale 直连）──────────────────────────────
        // 进程仍只绑 127.0.0.1；对外由 `tailscale serve --bg <port>` 代理，
        // 访问控制交给 Tailscale ACL，本程序不自己发明认证。
        use motomech_lib::capture;
        let r = root_s.clone();
        app = app.route("/capture", get(move |q: Query<HashMap<String,String>>| {
            let r = r.clone();
            async move {
                let pf = match Platform::new(Some(r)) { Ok(x) => x, Err(e) =>
                    return axum::response::Html(format!("<p>平台打不开：{e}</p>")) };
                let list = pf.list_vehicles().unwrap_or_default();
                let cur = q.get("vehicle").cloned()
                    .or_else(|| pf.resolve_vehicle(None).ok().map(|v| v.id))
                    .unwrap_or_default();
                axum::response::Html(capture::page(&list, &cur))
            }
        }));

        let r = root_s.clone();
        app = app.route("/v1/capture", post(move |
            axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
            mut mp: axum::extract::Multipart|
        {
            let r = r.clone();
            async move {
                // 纵深防御：万一有人把 --host 改成 0.0.0.0，公网地址也进不来
                if !capture::is_trusted(peer.ip()) {
                    return Json(serde_json::json!({"error":
                        format!("来源 {} 不可信 —— 只接受回环与 Tailscale(100.64.0.0/10)", peer.ip())}));
                }
                let pf = match Platform::new(Some(r)) { Ok(x) => x,
                    Err(e) => return Json(serde_json::json!({"error": format!("{e:#}")})) };
                let (mut veh, mut phase, mut tag) = (String::new(), String::new(), String::new());
                let (mut name, mut mime, mut data) = (String::new(), String::new(), Vec::new());
                loop {
                    let field = match mp.next_field().await {
                        Ok(Some(f)) => f, Ok(None) => break,
                        Err(e) => return Json(serde_json::json!({"error":
                            format!("表单解析失败：{e}")})),
                    };
                    let fname = field.name().unwrap_or("").to_string();
                    if fname == "file" {
                        name = field.file_name().unwrap_or("capture").to_string();
                        mime = field.content_type().unwrap_or("").to_string();
                        match field.bytes().await { Ok(b) => data = b.to_vec(),
                            Err(e) => return Json(serde_json::json!({"error":
                                format!("读取文件失败：{e}")})) }
                    } else {
                        let val = field.text().await.unwrap_or_default();
                        match fname.as_str() {
                            "vehicle" => veh = val, "phase" => phase = val,
                            "tag" => tag = val, _ => {}
                        }
                    }
                }
                let vopt = if veh.trim().is_empty() { None } else { Some(veh.as_str()) };
                match capture::save(&pf, vopt, &phase, &tag, &name, &mime, &data) {
                    Ok(s) => Json(serde_json::json!({
                        "saved": s.path.file_name().map(|x| x.to_string_lossy().to_string()),
                        "dir": s.path.parent().map(|x| x.to_string_lossy().to_string()),
                        "kind": if s.is_image { "image" } else { "audio" },
                        "kb": s.bytes / 1024})),
                    Err(e) => Json(serde_json::json!({"error": format!("{e:#}")})),
                }
            }
        }));

        let app = app.layer(axum::extract::DefaultBodyLimit::max(64*1024*1024))
                      .layer(tower_http_cors());
        let addr = format!("{host}:{port}");
        println!("HTTP API 监听 http://{addr}");
        for line in [
            "GET  /health",
            "GET  /v1/vehicles",
            "GET  /v1/workbench?vehicle=&phase=",
            "GET  /v1/specs?q=&vehicle=          只返回 A/B 级且已核实",
            "GET  /v1/search?q=&limit=&vehicle=  排除不适用本车的资料；默认只出 A/B 级（include_low_tier=true 放行 C）",
            "GET  /v1/recalls?vehicle=",
            "GET  /v1/known_issues?vehicle=",
            "GET  /v1/parts?days_per_phase=&vehicle=",
            "GET  /v1/briefing?phase=&vehicle=",
            "GET  /v1/log?limit=&vehicle=",
            "GET  /v1/vlog?vehicle=              选题/动画/素材（读纪录片目录，只读）",
            "GET  /v1/gaps?vehicle=              差距清单：系统自己知道还不知道什么",
            "GET  /v1/symptom?text=&vehicle=    症状→原因",
            "GET  /v1/diag?tree=&vehicle=       排查树（不带 tree 则列出）",
            "GET  /v1/safety?action=&vehicle=    动作判定 BLOCK/GATE/WARN/OK",
            "GET  /v1/job_card?intent=&phase=    **BLOCK 时不返回步骤**",
            "POST /v1/log/append                 需 idempotency_key",
            "GET  /v1/circuit?query=&all=        电路图定位（默认只出适用本车年款的）",
            "GET  /v1/crossval?vehicle=          交叉验证：安全关键项有无两个独立来源",
            "GET  /v1/script_check?vehicle=      Vlog 脚本体检",
            "GET  /v1/acquire?vehicle=           资料获取计划（只定位不下载）",
            "GET  /capture                       **手机采集页**（拍照/录音，经 Tailscale 直连）",
            "POST /v1/capture                    手机上传（multipart；只收回环与 100.64/10）",
        ] { println!("  {line}"); }
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
        Ok::<(), anyhow::Error>(())
    })
}

/// 工作台 —— 与 GUI 完全相同的数据，文本渲染。
/// 先有它再有界面：GUI 出问题时能立刻分清是数据层还是渲染层。
fn cmd_workbench(root: Option<String>, vehicle: Option<&str>, phase: Option<&str>,
                 json: bool) -> Result<()> {
    let p = Platform::new(root)?;
    let w = motomech_lib::workbench::build(&p, vehicle, phase)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&w)?);
        return Ok(());
    }

    println!("🏍  {} {} {}  [{}]", w.year, w.make, w.model, w.engine_code);
    println!("    VIN: {}", w.vin.clone().unwrap_or_else(|| "— 未采集 —".into()));

    if !w.open_recalls.is_empty() {
        println!("\n🔴 未执行召回 {} 项", w.open_recalls.len());
        for r in &w.open_recalls {
            let m = if r.severity == "critical" { "🔴" } else { "🟡" };
            println!("   {m} {} · {}", r.id, r.component);
            println!("      {}", r.note);
        }
    }

    println!("\n⛔ 项目硬红线（顺序不可调换）");
    for (i, h) in w.hard_rules.iter().enumerate() {
        println!("   {}. {}", i + 1, h.rule);
    }

    let done = w.phases.iter().position(|x| x.is_current).unwrap_or(0);
    let bar: String = w.phases.iter().enumerate()
        .map(|(i, _)| if i < done { '▓' } else if i == done { '▶' } else { '░' }).collect();
    println!("\n阶段  {bar}  {}/{}", done + 1, w.phases.len());
    println!("▶ {} {} —— {}", w.current_phase, w.current_name, w.current_summary);

    if !w.gates.is_empty() {
        println!("\n⚠️  本阶段安全门禁");
        for g in &w.gates { println!("   [{}] {}", g.level, g.rule); }
    }

    println!("\n作业项 {} 个", w.tasks.len());
    for t in &w.tasks {
        println!("   {} {}", if t.done { "☑" } else { "☐" }, t.name);
        if let Some(d) = &t.detail { println!("       {d}"); }
        if let Some(h) = &t.hint   { println!("       要点：{h}"); }
        if let Some(sc) = &t.source { println!("       来源：{sc}"); }
        for sp in &t.specs {
            println!("       · {} = {}  [{}]{}  {}", sp.label, sp.value, sp.status,
                     if sp.corroboration.is_empty() { String::new() } else { format!("  {}", sp.corroboration) },
                     sp.source);
        }
    }

    println!("\n数据  可用规格 {} · 待核实 {} · 适用手册页 {} (排除 {} 份) · 照片 {}",
             w.specs_usable, w.specs_pending, w.manual_pages, w.manual_excluded, w.photo_count);

    if !w.warnings.is_empty() {
        println!("\n⚠️  待办提示");
        for x in &w.warnings { println!("   · {x}"); }
    }
    Ok(())
}

/// 允许 Vite dev server（另一端口）取数据。仅监听 127.0.0.1，不对外。
fn tower_http_cors() -> tower_http::cors::CorsLayer {
    tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
}

fn cmd_providers(root: Option<String>) -> Result<()> {
    let p = Platform::new(root)?;
    let (list, default) = motomech_lib::provider::load(&p.root)?;
    println!("配置文件 {}\n", p.root.join("providers.yml").display());
    for pr in &list {
        let mark = if pr.id == default { "★" } else { " " };
        println!("{mark} {:<12} {:<11} {}", pr.id, pr.kind,
                 pr.model.clone().unwrap_or_else(|| "(provider 自身配置)".into()));
        if let Some(n) = &pr.note { println!("    {n}"); }
    }
    println!("\n★ = 默认");
    Ok(())
}

/// 构造系统提示：车辆身份 + 适用性铁律 + 已核实规格 + 准确性铁律。
/// **只注入 A/B 级规格**（来自 specs_usable 视图），C/D 级永远不会进提示词 ——
/// 模型看不到的东西，就不可能把它当成依据说出来。
fn build_system_prompt(p: &Platform, vehicle: Option<&str>) -> Result<String> {
    let w = motomech_lib::workbench::build(p, vehicle, None)?;
    let v = p.resolve_vehicle(vehicle)?;
    let conn = db::open(&p.index_db())?;

    let mut specs = String::new();
    let mut stmt = conn.prepare(
        "SELECT item, value, source_level, COALESCE(source_ref,''), COALESCE(page,0)
         FROM specs_usable WHERE vehicle_id=?1 ORDER BY item")?;
    let rows = stmt.query_map([&v.id], |r| Ok((
        r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
        r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?;
    for row in rows {
        let (item, val, lvl, sref, page) = row?;
        specs.push_str(&format!("  {item} = {val}  [{lvl}级 · {sref}{}]\n",
            if page > 0 { format!(" p{page}") } else { String::new() }));
    }

    let excludes: Vec<String> = {
        let mut st = conn.prepare("SELECT term FROM applicability_excludes WHERE vehicle_id=?1")?;
        let it = st.query_map([&v.id], |r| r.get::<_, String>(0))?;
        it.filter_map(|x| x.ok()).collect()
    };

    Ok(format!(
"你是 {year} {make} {model}（发动机代号 {engine}）的专业维修顾问。这台车停放了十年，正在修复。

【适用性铁律】本车是 {engine}。以下资料**不适用**，绝不可据此作答：
  {excludes}
如果只知道其他代际的数据，必须明说「这是 X 代的数据，不适用本车」，而不是拿来用。

【准确性铁律】
1. 扭力值、间隙、容量、零件号 —— 只用下面列出的已核实数据。**没列出的就说不知道**。
2. 绝不编造看似合理的数字或件号。编一个 P0130 或一个扭力值，比说「不知道」危险得多。
3. 引用数据时带上来源级别和页码。

【已核实规格（A/B 级，可用于作业）】
{specs}
【未执行的召回】
{recalls}
【项目硬红线】
{rules}
用中文回答，简洁。",
        year = w.year, make = w.make, model = w.model, engine = w.engine_code,
        excludes = excludes.join("、"),
        specs = if specs.is_empty() { "  （无）\n".into() } else { specs },
        recalls = w.open_recalls.iter()
            .map(|r| format!("  {} {} —— {}", r.id, r.component, r.note))
            .collect::<Vec<_>>().join("\n"),
        rules = w.hard_rules.iter().enumerate()
            .map(|(i, h)| format!("  {}. {}", i + 1, h.rule))
            .collect::<Vec<_>>().join("\n"),
    ))
}

fn cmd_ask(root: Option<String>, vehicle: Option<&str>, question: &str,
           provider: Option<&str>, _show_prompt: bool) -> Result<()> {
    // 走 RAG 问答模块:事实层(已核实,带出处,不经模型)+ 生成层分开,
    // 无相关事实时不调模型编答案。这是阶段 3「整包灌上下文」写法的升级。
    let p = Platform::new(root)?;
    let r = motomech_lib::ask::ask(&p, vehicle, question, provider)?;
    print!("{}", motomech_lib::ask::render(&r));
    Ok(())
}

fn cmd_mcp(root: Option<String>) -> Result<()> {
    motomech_lib::mcp::serve(root)
}

fn yn(b: bool) -> &'static str { if b { "✅" } else { "—" } }
