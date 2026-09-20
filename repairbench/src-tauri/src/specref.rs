//! 规格引用解析 —— **全项目唯一一份**
//!
//! ## 为什么必须只有一份
//!
//! 「给一个 key（canonical 或 item 名），这条规格现在是什么状态」这个判断，
//! 此前在 `workbench.rs` 和 `diag.rs` 各写了一遍（共四处 SQL）。
//! 结果是同一个 bug 犯了两次：
//!
//! * 2026-09-16 监工：作业单里 item 名指向冲突规格 → 退化成「未收录」。修了 workbench。
//! * 2026-09-19 监工：排查树里同样的写法 → 退化成「档案里还没有这个值」。**原样复活**。
//!
//! 两次的根因一模一样：有冲突的 item 已从 `specs_usable` 消失，
//! 只按 canonical 比就会 miss，于是「**这个值有争议、已被扣下**」
//! 被说成「**档案里没有这个值**」。对拧扳手的人，这两句话意思完全不同 ——
//! 前者是「别拧，先去裁决」，后者是「去补个数据」。
//!
//! 修一处不够。判定只有一份，才不会再复活第三次。
//!
//! ## 三种状态，缺一不可
//!
//! 冲突和缺失**必须分开**。把它们合并成「没有可用值」看起来更简洁，
//! 但那正是上面两次 bug 的形态。

use rusqlite::Connection;

#[derive(Debug, Clone)]
pub enum SpecState {
    /// 已核实且无冲突 —— 可以用
    Verified {
        item: String,
        canonical: String,
        value: String,
        level: String,
        source_ref: String,
        page: i64,
    },
    /// 同一语义身份有多个矛盾值 —— **值被扣下，一个都不给**
    Conflicted {
        canonical: String,
        values: String,
        sources: String,
    },
    /// 档案里没有这条已核实规格
    Missing,
}

impl SpecState {
    /// 给人看的状态词
    pub fn status(&self) -> &'static str {
        match self {
            SpecState::Verified { .. } => "已核实",
            SpecState::Conflicted { .. } => "待定·有冲突",
            SpecState::Missing => "未收录",
        }
    }
    pub fn value(&self) -> Option<&str> {
        match self { SpecState::Verified { value, .. } => Some(value), _ => None }
    }
    pub fn canonical(&self) -> Option<&str> {
        match self {
            SpecState::Verified { canonical, .. } => Some(canonical),
            SpecState::Conflicted { canonical, .. } => Some(canonical),
            SpecState::Missing => None,
        }
    }
}

/// 把 key 解析成 canonical。
///
/// key 可以是 canonical，也可以是 item 名（**任务和排查树里写 item 名更自然**）。
/// 查的是全量 `specs` 表而不是 `specs_usable` —— 有冲突的项已经从后者消失了，
/// 而我们恰恰需要认出它们。
fn to_canonical(conn: &Connection, vehicle: &str, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT canonical FROM specs \
         WHERE vehicle_id=?1 AND (canonical=?2 OR item=?2 OR item LIKE '%'||?2||'%') LIMIT 1",
        rusqlite::params![vehicle, key],
        |r| r.get::<_, String>(0)).ok()
}

/// 解析一个规格引用。**这是全项目唯一的入口。**
///
/// 顺序是有讲究的：先认 canonical，**再判冲突，最后才取值**。
/// 反过来（先取值、取不到再说冲突）就会重演那两次 bug。
pub fn resolve(conn: &Connection, vehicle: &str, key: &str) -> SpecState {
    let key = key.trim();
    if key.is_empty() { return SpecState::Missing; }
    let canon = match to_canonical(conn, vehicle, key) {
        Some(c) => c,
        None => return SpecState::Missing,
    };
    // 冲突优先：有争议的值一个都不给
    if let Ok((values, sources)) = conn.query_row(
        "SELECT values_csv, sources_csv FROM spec_conflicts WHERE vehicle_id=?1 AND canonical=?2",
        rusqlite::params![vehicle, &canon],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
    {
        return SpecState::Conflicted { canonical: canon, values, sources };
    }
    // 无冲突才取值；仍然查安全视图（C/D 级不可能出现在这里）
    match conn.query_row(
        "SELECT item, value, source_level, COALESCE(source_ref,''), COALESCE(page,0) \
         FROM specs_usable WHERE vehicle_id=?1 AND canonical=?2 LIMIT 1",
        rusqlite::params![vehicle, &canon],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
                r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))
    {
        Ok((item, value, level, source_ref, page)) =>
            SpecState::Verified { item, canonical: canon, value, level, source_ref, page },
        Err(_) => SpecState::Missing,
    }
}

/// 出处的展示形式：`B级·Haynes 4598 · Torque settings p22`
pub fn source_label(level: &str, source_ref: &str, page: i64) -> String {
    if page > 0 { format!("{level}级·{source_ref} p{page}") }
    else if !source_ref.is_empty() { format!("{level}级·{source_ref}") }
    else { format!("{level}级") }
}

/// 从 canonical/item 的后缀补单位。
///
/// 规格库把单位放在**名字**上（`torque.xxx_nm` 的值就是 `8`）。
/// 内联进正文后会变成「另有扭力 8」——**没有单位的扭力值是危险的**。
/// 只在值是纯数字/区间时补，避免「12V 14Ah Nm」这种。
pub fn unit_suffix(name: &str, value: &str) -> &'static str {
    let bare = value.chars().all(|c| c.is_ascii_digit() || ".,-–~/ ".contains(c));
    if !bare { return ""; }
    let c = name.to_lowercase();
    for (suf, unit) in [("_nm", " Nm"), ("_kohms", " kΩ"), ("_ohms", " Ω"),
                        ("_mm", " mm"), ("_psi", " psi"), ("_kg", " kg"),
                        ("_w", " W"), ("_a", " A"), ("_v", " V"),
                        ("_l", " L"), ("_km", " km"), ("_bar", " bar")] {
        if c.ends_with(suf) { return unit; }
    }
    ""
}
