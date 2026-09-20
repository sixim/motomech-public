//! motomech —— 维修助理平台
//!
//! 模块定义在 lib crate，让 Tauri 前端命令与 CLI 共用同一套数据层。

pub mod api;
pub mod applicability;
pub mod acquire;
pub mod ask;
pub mod capture;
pub mod circuit;
pub mod coldstart;
pub mod crossval;
pub mod db;
pub mod diag;
pub mod gaps;
pub mod index;
pub mod mcp;
pub mod parts;
pub mod research;
pub mod safety;
pub mod script;
pub mod specref;
pub mod symptom;
pub mod platform;
pub mod provider;
pub mod tools;
pub mod workbench;

use platform::Platform;

/// 工作台数据。前端唯一的数据入口。
#[tauri::command]
fn workbench(vehicle: Option<String>, phase: Option<String>) -> Result<workbench::Workbench, String> {
    let p = Platform::new(None).map_err(|e| e.to_string())?;
    workbench::build(&p, vehicle.as_deref(), phase.as_deref()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn vehicles() -> Result<Vec<api::VehicleBrief>, String> {
    let p = Platform::new(None).map_err(|e| e.to_string())?;
    api::vehicles(&p).map_err(|e| format!("{e:#}"))
}

// 下面这些命令全部是 api 层的薄包装 —— GUI 看到的和 MCP/HTTP 看到的
// 必须是同一份数据，包括「已排除 N 份」这类提示。
// （原本想用宏省几行，但宏卫生性让 body 看不见宏内绑定的 p；
//   为省五行代码引入一层看不见的绑定不划算，摊开写。）
fn pf() -> Result<Platform, String> {
    Platform::new(None).map_err(|e| e.to_string())
}
fn emsg(e: anyhow::Error) -> String { format!("{e:#}") }

#[tauri::command]
fn specs(vehicle: Option<String>, q: String) -> Result<api::SpecResult, String> {
    api::specs(&pf()?, vehicle.as_deref(), &q).map_err(emsg)
}

#[tauri::command]
fn search(vehicle: Option<String>, q: String, limit: Option<i64>)
    -> Result<api::SearchResult, String> {
    api::manual_search(&pf()?, vehicle.as_deref(), &q, limit.unwrap_or(8)).map_err(emsg)
}

#[tauri::command]
fn parts(vehicle: Option<String>, days_per_phase: Option<i64>)
    -> Result<parts::PartsPlan, String> {
    parts::plan(&pf()?, vehicle.as_deref(), days_per_phase.unwrap_or(7)).map_err(emsg)
}

#[tauri::command]
fn log(vehicle: Option<String>, limit: Option<usize>) -> Result<api::LogRead, String> {
    api::log_read(&pf()?, vehicle.as_deref(), limit.unwrap_or(20)).map_err(emsg)
}

#[tauri::command]
fn vlog(vehicle: Option<String>) -> Result<api::VlogView, String> {
    api::vlog(&pf()?, vehicle.as_deref()).map_err(emsg)
}

#[tauri::command]
fn known_issues(vehicle: Option<String>) -> Result<serde_json::Value, String> {
    api::known_issues(&pf()?, vehicle.as_deref()).map_err(emsg)
}

#[tauri::command]
fn safety(vehicle: Option<String>, action: String) -> Result<safety::SafetyResult, String> {
    safety::evaluate(&pf()?, vehicle.as_deref(), &action).map_err(emsg)
}

#[tauri::command]
fn briefing(vehicle: Option<String>, phase: Option<String>) -> Result<serde_json::Value, String> {
    let p = Platform::new(None).map_err(|e| e.to_string())?;
    let ph = match phase {
        Some(x) if !x.trim().is_empty() => x,
        _ => workbench::build(&p, vehicle.as_deref(), None)
                .map_err(|e| format!("{e:#}"))?.current_phase,
    };
    let md = safety::briefing(&p, &ph).map_err(|e| format!("{e:#}"))?;
    Ok(serde_json::json!({"phase": ph, "markdown": md}))
}

#[tauri::command]
fn gaps(vehicle: Option<String>) -> Result<gaps::GapReport, String> {
    gaps::find(&pf()?, vehicle.as_deref()).map_err(emsg)
}

#[tauri::command]
fn symptom(vehicle: Option<String>, text: String) -> Result<symptom::SymptomResult, String> {
    symptom::diagnose(&pf()?, vehicle.as_deref(), &text).map_err(emsg)
}

#[tauri::command]
fn diag_list() -> Result<Vec<(String, String)>, String> {
    diag::list(&pf()?).map_err(emsg)
}

#[tauri::command]
fn diag_tree(vehicle: Option<String>, tree: String) -> Result<diag::Tree, String> {
    diag::load(&pf()?, vehicle.as_deref(), &tree).map_err(emsg)
}

// ── 2026-09-19 补齐：这四个能力此前只有 CLI/MCP 有，GUI 一个都拿不到。
//    而用户日常用的正是那个窗口 —— 功能做完了但人看不见，等于没做。
#[tauri::command]
fn circuit(vehicle: Option<String>, query: Option<String>, all: Option<bool>)
    -> Result<circuit::Report, String> {
    let p = pf()?;
    let v = p.resolve_vehicle(vehicle.as_deref()).map_err(emsg)?;
    let q = query.as_deref().filter(|s| !s.trim().is_empty());
    circuit::build(&p, &v, q, all.unwrap_or(false)).map_err(emsg)
}

#[tauri::command]
fn crossval(vehicle: Option<String>) -> Result<crossval::Report, String> {
    let p = pf()?;
    let v = p.resolve_vehicle(vehicle.as_deref()).map_err(emsg)?;
    crossval::build(&p, &v).map_err(emsg)
}

#[tauri::command]
fn script_check(vehicle: Option<String>) -> Result<script::Report, String> {
    let p = pf()?;
    let v = p.resolve_vehicle(vehicle.as_deref()).map_err(emsg)?;
    script::check(&p, &v).map_err(emsg)
}

#[tauri::command]
fn acquire(vehicle: Option<String>) -> Result<acquire::Acquire, String> {
    let p = pf()?;
    let v = p.resolve_vehicle(vehicle.as_deref()).map_err(emsg)?;
    acquire::build(&p, &v).map_err(emsg)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())

        .invoke_handler(tauri::generate_handler![workbench, vehicles, specs, search, parts,
            log, vlog, known_issues, safety, briefing,
            gaps, symptom, diag_list, diag_tree,
            circuit, crossval, script_check, acquire])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
