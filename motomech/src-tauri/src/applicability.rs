//! 适用性判定 —— **全项目唯一一份**
//!
//! ## 这个项目所有最贵的错误都是这一类
//!
//! * 2026-09-14：整套规格出自 2010 车主手册，而真车是 2009 hexhead。16 条 A 级规格全部作废。
//! * 2026-09-17：电路图按年份分档，2004–2006 / 2007 / 2008-on 是三套不同的图。
//! * 2026-09-19：`circuit` 用裸子串比车系，「SPO**RT**STER」含 RT
//!   → 一台哈雷拿到 BMW R1200 RT 的电路图，还标着「✅ 适用本车」。
//!
//! 三次的形态一样：**资料看起来对得上，实际不是这台车的**。
//! 而且每次都发生在「又写了一份判定逻辑」的地方。所以判定只能有一份。
//!
//! ## 三道闸，顺序不可调换
//!
//! 1. **厂牌** —— 厂牌不同，后面全部免谈。不先过这道闸，车系名就会互相撞
//!    （Sportster 含 RT、ST3 含 ST）。
//! 2. **车系** —— 按**整词**比，绝不用子串。
//! 3. **年份 / 代际** —— 同厂同车系，差一年可能是另一套电路。
//!
//! ## fail-closed
//!
//! 「资源没声明这一维约束」和「车辆身份不明」是两件事：
//! 前者表示不受该维限制（放行），后者表示**无法判断**（不放行）。
//! 把它们混成一件，就会得到「什么都适用」——那正是上面三次错误的温床。

use serde::Serialize;

/// 车辆身份（从 VEHICLE.yml 取）
#[derive(Debug, Clone, Default)]
pub struct VehicleIdent {
    pub make: String,
    pub model: String,
    pub year: i64,
    pub engine_code: String,
}

/// 资源声明它适用于谁。每一维都可以不声明 = 不受该维限制。
#[derive(Debug, Clone, Default)]
pub struct Declares {
    pub make: Option<String>,
    /// 车系名（如 "R1200 GS / GS Adventure"）—— 会被切成词来比
    pub model: Option<String>,
    pub years: Option<(i64, i64)>,
    /// 代际（如 hexhead）—— 与 engine_code 比
    pub generations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Verdict {
    /// 适用
    Applies,
    /// 明确不适用，附原因（要给人看，不能只说「不适用」）
    NotApplies(String),
}

impl Verdict {
    pub fn ok(&self) -> bool { matches!(self, Verdict::Applies) }
    pub fn reason(&self) -> &str {
        match self { Verdict::NotApplies(r) => r, _ => "" }
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 切词。`R1200GS` 会额外产出 `GS` —— 连写的车系标记要能认出来，
/// 但 `XL1200` 不会产出任何车系标记（尾部是数字）。
fn tokens(s: &str) -> Vec<String> {
    let up = s.to_uppercase();
    let mut out: Vec<String> = up.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty()).map(String::from).collect();
    for t in out.clone() {
        if let Some(pos) = t.find(|c: char| c.is_ascii_digit()) {
            let tail: String = t[pos..].trim_start_matches(|c: char| c.is_ascii_digit()).into();
            if !tail.is_empty() { out.push(tail); }
        }
    }
    out
}

/// 车系标记：GS / RT / ST / R / S。取**最长**的那个，
/// 否则 "R" 会在 "R1200 RT" 上抢先命中，把 RT 的图发给 R 的车。
fn family_tag(model: &str) -> Option<String> {
    let t = tokens(model);
    let mut tags: Vec<&str> = ["GS", "RT", "ST", "R", "S"].into_iter()
        .filter(|x| t.iter().any(|y| y == x)).collect();
    tags.sort_by_key(|x| std::cmp::Reverse(x.len()));
    tags.first().map(|x| x.to_string())
}

/// 裁决。**这是全项目唯一的入口。**
pub fn judge(v: &VehicleIdent, d: &Declares) -> Verdict {
    // ① 厂牌闸
    if let Some(m) = &d.make {
        if v.make.trim().is_empty() {
            return Verdict::NotApplies(format!(
                "本车档案没声明厂牌，而资料声明自己是「{m}」的 —— 判不了就不放行"));
        }
        let (a, b) = (norm(m), norm(&v.make));
        if !(a.contains(&b) || b.contains(&a)) {
            return Verdict::NotApplies(format!(
                "厂牌对不上：资料是「{m}」的，本车是「{}」", v.make));
        }
    }
    // ② 车系闸 —— 整词比，不用子串
    if let Some(m) = &d.model {
        match family_tag(m) {
            Some(tag) => {
                if !tokens(&v.model).iter().any(|x| *x == tag) {
                    return Verdict::NotApplies(format!(
                        "车系对不上：资料是「{m}」，本车是「{}」", v.model));
                }
            }
            // 资料声明了车型但认不出车系标记 —— 退回整串归一化比对
            None => {
                if !norm(&v.model).contains(&norm(m)) && !norm(m).contains(&norm(&v.model)) {
                    return Verdict::NotApplies(format!(
                        "车型对不上：资料是「{m}」，本车是「{}」", v.model));
                }
            }
        }
    }
    // ③ 年份闸
    if let Some((from, to)) = d.years {
        if v.year <= 0 {
            return Verdict::NotApplies(format!(
                "本车档案没有年份，而资料只覆盖 {from}–{to} —— 判不了就不放行"));
        }
        if v.year < from || v.year > to {
            return Verdict::NotApplies(format!(
                "年份不覆盖：资料是 {from}–{to}，本车是 {}", v.year));
        }
    }
    // ④ 代际闸
    if !d.generations.is_empty() {
        if v.engine_code.trim().is_empty() {
            return Verdict::NotApplies(
                "本车档案没有发动机代号，无法判代际 —— 判不了就不放行".into());
        }
        let ec = norm(&v.engine_code);
        if !d.generations.iter().any(|g| ec.contains(&norm(g)) || norm(g).contains(&ec)) {
            return Verdict::NotApplies(format!(
                "代际对不上：资料出自 {:?} 代，本车是「{}」", d.generations, v.engine_code));
        }
    }
    Verdict::Applies
}

/// 从 VEHICLE.yml 的 Value 取身份
pub fn ident_from(doc: &serde_yaml_ng::Value) -> VehicleIdent {
    let g = |k: &str| doc.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    VehicleIdent {
        make: g("make"), model: g("model"),
        year: doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0),
        engine_code: g("engine_code"),
    }
}
