//! 资料获取计划 —— 「这台车该找哪本手册，去哪找」
//!
//! 这是「自我进化」里**合法自动化的那一半**。边界是刻意划的：
//!
//! * **能自动**：查清楚「有没有出版社为这台车出过维修手册」。书目数据
//!   （书名/出版社/年份/ISBN）来自图书馆目录，是 B 级权威三方数据，
//!   查它不涉及任何版权问题 —— 目录不是内容。
//!
//! * **不自动**：下载手册本身。三条理由，每条都足以单独否决：
//!   1. **来源分级**：网上抓来的扫描件是 C/D 级，而作业单只吃 A/B。
//!      自动下载 = 自动往库里灌不可信内容，正是本项目命名的「假绿灯」。
//!   2. **版权**：Haynes / 厂商手册受版权保护。2026-09-16 实测 Internet Archive
//!      上「ducati workshop manual」前 3 条**全部没有许可声明** ——
//!      没有声明就等于权利不明，不明就不能下。
//!   3. **适用性**：同一车型跨年款差异极大（本项目自己就栽过 2009 hexhead
//!      被当成 2010 camhead 的跟头）。抓错一版比没有更危险。
//!
//! 所以本模块的产物是**获取计划**，不是资料：查到什么书、去哪买/借、
//! 拿到实体后走哪条流水线进库。采集那一步交还给人，这是设计不是缺口。

use crate::{platform::{Platform, Vehicle}, tools};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct Book {
    pub title: String,
    pub publisher: String,
    pub year: Option<i64>,
    pub isbn: Option<String>,
    pub pages: Option<i64>,
    /// 这本书是否明确点名了本车型 —— 只靠书名判断，不臆测
    pub names_model: bool,
    /// 库里是否已经有这本 —— 别推荐你已经买了的书
    pub owned: bool,
    /// 有没有合法电子借阅：public / borrowable / printdisabled / no_ebook
    pub ebook_access: String,
    /// 由 ISBN 确定性构造的购买/借阅直达入口（不爬页面，所以不会失效）
    pub buy: Vec<Lead>,
    /// 从书名解析出的适用年段
    pub years: Option<(i64, i64)>,
    /// 覆盖判定：covers / not_covered / unknown
    pub covers: String,
}

#[derive(Debug, Serialize)]
pub struct Lead {
    pub what: String,
    pub url: String,
    pub source_level: String,
}

#[derive(Debug, Serialize)]
pub struct Acquire {
    pub make: String,
    pub model: String,
    pub year: i64,
    /// 已经在库里的手册（别重复买）
    pub already_have: Vec<String>,
    pub queries_tried: Vec<String>,
    pub books: Vec<Book>,
    pub leads: Vec<Lead>,
    /// 图纸来源（爆炸图/件号）—— 和手册分开
    pub diagrams: Vec<Lead>,
    /// 结论：到底该买什么。放在最前面，不让人去一堆书里自己找
    pub verdict: String,
    pub notes: Vec<String>,
    pub next_steps: Vec<String>,
}

fn sv(d: &serde_yaml_ng::Value, k: &str) -> Option<String> {
    d.get(k).and_then(|x| x.as_str()).map(String::from)
}

/// 从书名解析适用年段。维修手册的书名几乎总把年段写在里面：
/// 「'91 to '05」「1970-1993」「(2010 to 2017)」「'04 to '09」。
///
/// **这是本项目最贵的一课换来的检查**：2009 hexhead 被当成 2010 camhead，
/// 16 条规格全部作废重建。书名对、车系对、年份差一代 —— 照样是错的。
fn parse_years(title: &str) -> Option<(i64, i64)> {
    let norm = |y: i64| if y < 100 { if y > 50 { 1900 + y } else { 2000 + y } } else { y };
    // 「'91 to '05」/「91 to 05」/「2010 to 2017」
    let re_to = regex_lite_pairs(title, " to ");
    if let Some((a, b)) = re_to { return Some((norm(a), norm(b))); }
    // 「1970-1993」/「'04-'09」
    let re_dash = regex_lite_pairs(title, "-");
    if let Some((a, b)) = re_dash { return Some((norm(a), norm(b))); }
    None
}

/// 极简两数抽取：找 `sep` 两侧紧邻的年份数字（可带撇号）。
/// 不引正则依赖 —— 这里的模式足够固定。
fn regex_lite_pairs(title: &str, sep: &str) -> Option<(i64, i64)> {
    for (i, _) in title.match_indices(sep) {
        let left: String = title[..i].chars().rev()
            .take_while(|c| c.is_ascii_digit() || *c == '\'' ).collect::<String>()
            .chars().rev().filter(|c| c.is_ascii_digit()).collect();
        let right: String = title[i + sep.len()..].chars()
            .skip_while(|c| *c == '\'')
            .take_while(|c| c.is_ascii_digit()).collect();
        if (left.len() == 2 || left.len() == 4) && (right.len() == 2 || right.len() == 4) {
            if let (Ok(a), Ok(b)) = (left.parse::<i64>(), right.parse::<i64>()) {
                return Some((a, b));
            }
        }
    }
    None
}

/// 由 ISBN 确定性构造购买/借阅入口。
///
/// **为什么用 ISBN 而不是去爬商家页面**：2026-09-16 实测 —— Cyclepedia 的搜索页
/// 返回 HTTP 200 且「ducati」命中 11 次，但抓出真实产品名后**一本 Ducati 手册都没有**
/// （命中全在页脚模板里）；Partzilla / RealOEM / Fowlers 直接 403 封机器人。
/// 「HTTP 200 + 关键词命中」根本不等于「有这台车」。
/// ISBN 是全球唯一书号，用它拼出来的链接是确定性的，不依赖任何页面结构。
fn buy_links(isbn: &str) -> Vec<Lead> {
    vec![
        Lead { what: "图书馆借（先查有没有免费的）".into(),
               url: format!("https://search.worldcat.org/search?q=bn:{isbn}"),
               source_level: "B".into() },
        Lead { what: "二手（维修手册二手最划算，内容不过期）".into(),
               url: format!("https://www.abebooks.com/servlet/SearchResults?isbn={isbn}"),
               source_level: "B".into() },
        Lead { what: "新书".into(),
               url: format!("https://www.amazon.com/dp/{isbn}"),
               source_level: "B".into() },
        Lead { what: "书目详情与电子借阅状态".into(),
               url: format!("https://openlibrary.org/isbn/{isbn}"),
               source_level: "B".into() },
    ]
}

/// 图纸来源 —— 和「手册」是两件事，分开给。
///
/// * **线路图/电路图**：在维修手册里面，没有单独的免费来源。
/// * **爆炸图 + 件号（fiche）**：在线免费，但这些站**全部封机器人**
///   （实测 RealOEM 403、Partzilla 403），所以只能给人去点，机器不抓。
fn official_sources(p: &Platform, make: &str, model: &str) -> (Vec<Lead>, Vec<String>) {
    let f = p.root.join("knowledge").join("官方资料源.yml");
    let txt = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        // fail-closed：表读不到就说读不到，不要静默退回「去 Google 搜」
        Err(e) => return (vec![], vec![format!(
            "🔴 官方资料源注册表读不到（{}：{e}）—— 本节给不出厂商直达入口。", f.display())]),
    };
    let doc: serde_yaml_ng::Value = match serde_yaml_ng::from_str(&txt) {
        Ok(d) => d,
        Err(e) => return (vec![], vec![format!("🔴 官方资料源注册表解析失败：{e}")]),
    };
    let mk = make.to_lowercase();
    let mut out = Vec::new();
    if let Some(list) = doc.get("sources").and_then(|x| x.as_sequence()) {
        for it in list {
            let brands: Vec<String> = it.get("brands").and_then(|x| x.as_sequence())
                .map(|a| a.iter().filter_map(|b| b.as_str().map(|s| s.to_lowercase())).collect())
                .unwrap_or_default();
            // _any = 不限厂牌；否则要厂牌对得上，别把 KTM 的门户推给 Ducati
            if !brands.iter().any(|b| b == "_any" || mk.contains(b.as_str()) || b.contains(&mk)) { continue; }
            let g = |k: &str| it.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            out.push(Lead {
                what: format!("{} —— {}", g("name"), g("gives")),
                url: g("url").replace("{model}", &model.replace(' ', "+")),
                source_level: g("source_level"),
            });
        }
    }
    let mut notes = Vec::new();
    if out.is_empty() {
        notes.push(format!("⚠️ 注册表里还没有 {make} 的官方源 —— \
            实测过的厂牌见 knowledge/官方资料源.yml，这个厂牌尚未验证。\
            通用办法：带 VIN 找 {make} 经销商零件部门要官方 workshop manual 的书号。"));
    }
    (out, notes)
}

/// 查一次书目。返回 (书, 命中数)。
fn search_books(p: &Platform, q: &str, model: &str, have: &str, year: i64) -> Result<(Vec<Book>, usize)> {
    let mut args = HashMap::new();
    args.insert("q".to_string(), q.to_string());
    let r = tools::call(p, "openlibrary.manuals", &args)?;
    let data = r.data.context("书目接口没返回数据")?;
    let n = data.get("numFound").and_then(|x| x.as_i64()).unwrap_or(0) as usize;
    let norm = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "");
    let m = norm(model);

    let books = data.get("docs").and_then(|x| x.as_array()).map(|a| a.iter().map(|d| {
        let g = |k: &str| d.get(k).and_then(|x| x.as_str()).map(String::from);
        let first = |k: &str| d.get(k).and_then(|x| x.as_array())
            .and_then(|a| a.first()).and_then(|x| x.as_str()).map(String::from);
        let title = g("title").or_else(|| first("title")).unwrap_or_default();
        let isbn_v = first("isbn");
        let years = parse_years(&title);
        let covers = match years {
            Some((a, b)) if year > 0 => if year >= a && year <= b { "covers" } else { "not_covered" },
            _ => "unknown",
        }.to_string();
        Book {
            years, covers,
            ebook_access: g("ebook_access").unwrap_or_else(|| "unknown".into()),
            buy: isbn_v.as_deref().map(buy_links).unwrap_or_default(),
            names_model: !m.is_empty() && norm(&title).contains(&m),
            // 书名（归一化后）出现在库存清单里 = 这本已经有了。
            // 例：查到「BMW R1200 twins」，INDEX.md 写着《BMW R1200 Twins '04 to '09》→ 命中
            owned: { let t = norm(&title); t.len() > 6 && have.contains(&t) },
            title,
            publisher: first("publisher").unwrap_or_else(|| "（未注明）".into()),
            year: d.get("first_publish_year").and_then(|x| x.as_i64()),
            isbn: isbn_v,
            pages: d.get("number_of_pages_median").and_then(|x| x.as_i64()),
        }
    }).collect()).unwrap_or_default();
    Ok((books, n))
}

pub fn build(p: &Platform, v: &Vehicle) -> Result<Acquire> {
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(v.yml()).with_context(|| format!("读不了 {}", v.yml().display()))?)?;
    let make  = sv(&doc, "make").unwrap_or_default();
    let model = sv(&doc, "model").unwrap_or_default();
    let year  = doc.get("year").and_then(|x| x.as_i64()).unwrap_or(0);

    // 已有的手册 —— 别让它推荐你已经买了的那本
    let mut already_have = Vec::new();
    if let Ok(rd) = std::fs::read_dir(v.manuals_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with('.') { already_have.push(n); }
        }
    }
    already_have.sort();

    // 库存干草堆：目录名 + INDEX.md 正文，归一化后用于和书目命中对账。
    // 没有它，系统会把你手上那本 Haynes 4598 当成「还没有」再推荐一次。
    let mut have_blob = already_have.join(" ");
    for n in ["INDEX.md", "索引.md"] {
        if let Ok(t) = std::fs::read_to_string(v.manuals_dir().join(n)) { have_blob.push_str(&t); }
    }
    if let Ok(rd) = std::fs::read_dir(v.manuals_dir().join("third-party")) {
        for e in rd.flatten() { have_blob.push(' '); have_blob.push_str(&e.file_name().to_string_lossy()); }
    }
    let have_norm: String = have_blob.to_lowercase().chars()
        .filter(|c| c.is_alphanumeric()).collect();

    // 多角度查书目：车型名最准，但冷门车型只有车系级的书
    let queries = vec![
        format!("{make} {model} repair manual"),
        format!("{make} {model} workshop manual"),
        format!("{make} motorcycle repair manual"),
    ];
    let (mut books, mut tried, mut notes) = (Vec::<Book>::new(), Vec::new(), Vec::new());
    let mut owned_found = false;
    let mut ok_queries = 0usize;   // 真正查成功的次数 —— 0 次就不许下「没有」的结论
    let mut seen = Vec::<String>::new();
    let mut model_specific_found = false;

    for q in &queries {
        match search_books(p, q, &model, &have_norm, year) {
            Ok((bs, n)) => {
                ok_queries += 1;
                tried.push(format!("「{q}」→ {n} 条"));
                for b in bs {
                    if seen.contains(&b.title) { continue; }
                    seen.push(b.title.clone());
                    if b.names_model { model_specific_found = true; }
                    if b.owned { owned_found = true; }
                    books.push(b);
                }
            }
            // fail-closed：查不动就说查不动，不要让空结果看起来像「没有」
            Err(e) => {
                tried.push(format!("「{q}」→ ❌ 查询失败：{e}"));
                notes.push(format!("🔴 书目检索有一次失败（{q}）—— \
                    下面的清单**可能不完整**，不要据此断定「没有手册」。"));
            }
        }
    }

    if ok_queries == 0 {
        // fail-closed：一次都没查成，就什么结论都不许下。
        // （2026-09-16 实测抓到：三次查询全失败时，旧代码仍断言「没有手册，这是个真结论」——
        //   查都没查成凭什么说没有。这正是本项目命名的假绿灯。）
        notes.push(format!("🔴 **书目检索一次都没成功 —— 关于「{make} 有没有手册」本次什么也没查出来。**\
            上面的空清单**不是**「没有手册」，是「没查到」。两者不是一回事。\
            先修好检索（看上面的失败原因）再下结论。"));
    } else if books.is_empty() {
        notes.push(format!("🔴 **书目库里没有任何 {make} 的维修手册**（{ok_queries} 次检索均成功执行，结果为空）。\
            这是个真结论，意味着 Haynes/Clymer 这类第三方出版社没做过这个厂牌 —— \
            只能走厂商官方手册。"));
    } else if owned_found {
        notes.push(format!("✅ **查到的书里有你库里已经有的那本**（下面标 ✓）—— 不用重复买。\
            其余各本在采信前仍须核对适用年份/车型是否明确包含本车。"));
        if !model_specific_found {
            notes.push(format!("ℹ️ 没有一本书名里直接点到「{model}」，这很正常：\
                第三方出版社常按**车系**出书（如《BMW R1200 Twins '04 to '09》覆盖 GS/RT/R 等）。\
                书名不点名不代表不覆盖，但**必须翻到版权页核对年份范围**才能采信。"));
        }
    } else if !model_specific_found {
        notes.push(format!("ℹ️ 查到的 {} 本里没有一本书名点到「{model}」—— 已按年份分组，\
            能买的和不能买的见上。套用邻近车系/年款的手册是本项目栽过的跟头\
            （2009 hexhead 当成 2010 camhead，16 条规格全废）。", books.len()));
    }

    // 手册本体的来源 —— 全是给人点的链接，机器一个字节也没下载。
    // 具体到某一本的买法由 buy_links(ISBN) 给，这里只放「书目没查到时」的兜底门路。
    let enc = |s: &str| s.replace(' ', "+");
    let leads = vec![
        Lead { what: "Haynes / Clymer 官网按车型查是否出过本车的书".into(),
               url: format!("https://haynes.com/en-us/search?q={}", enc(&format!("{make} {model}"))),
               source_level: "B".into() },
        Lead { what: "eBay —— 绝版维修手册的主要流通地（按车型+manual 搜）".into(),
               url: format!("https://www.ebay.com/sch/i.html?_nkw={}", enc(&format!("{make} {model} service manual"))),
               source_level: "B".into() },
    ];
    let (diagrams, src_notes) = official_sources(p, &make, &model);
    for n in src_notes { notes.push(n); }

    // ── 结论 ──「该买什么」必须是一句话，不是让人在一堆书里自己找
    let owned_ok = books.iter().any(|b| b.owned);
    let best = books.iter().find(|b| b.covers == "covers" && b.names_model)
        .or_else(|| books.iter().find(|b| b.covers == "covers"))
        .or_else(|| books.iter().find(|b| b.names_model));
    let verdict = if ok_queries == 0 {
        "❓ 本次检索没跑成，给不出结论 —— 见下面的失败原因".to_string()
    } else if owned_ok {
        format!("✅ **不用买**：库里已有覆盖本车的手册。缺的是把它数字化（tools/bookproc.sh），不是再买一本。")
    } else if let Some(b) = best {
        format!("🛒 **买这本**：《{}》{}{}\n   {} —— 下面有图书馆/二手/新书三个入口。",
            b.title,
            b.isbn.as_ref().map(|i| format!("  ISBN {i}")).unwrap_or_default(),
            b.years.map(|(x,y)| format!("  适用 {x}–{y}")).unwrap_or_default(),
            if b.names_model { "书名直接点到本车型" } else { "年份覆盖本车，但书名没点到车型 —— 到手先翻版权页确认" })
    } else {
        format!("🔴 **第三方出版社没有覆盖本车的手册** —— 查到的 {} 本要么年份对不上，要么是别的车系。\n   唯一可靠路径是**厂商官方工作手册**：带 VIN 找 {make} 经销商零件部门，要 workshop/service manual 的官方书号并订购（官方手册是 A 级，比第三方还权威）。\n   爆炸图与件号可以先免费查（见下面「图纸怎么拿」）。", books.len())
    };

    let next_steps = vec![
        "① 用上面的书目/链接确认该买或该借哪一本 —— 买之前核对它的适用年份明确包含本车".to_string(),
        "② 拿到实体书后翻拍：bash tools/snap.sh -b（翻拍台模式）".to_string(),
        "③ OCR 进库：bash tools/bookproc.sh —— 这是手册内容进库的唯一合法路径".to_string(),
        "④ repairbench index --all，然后 repairbench gaps 看还缺什么".to_string(),
    ];

    Ok(Acquire { make, model, year, already_have, queries_tried: tried, books, leads, diagrams, verdict, notes, next_steps })
}

pub fn render(a: &Acquire) -> String {
    let mut o = format!("资料获取计划  {} {} {}\n", a.year, a.make, a.model);
    o.push_str(&"─".repeat(60));
    o.push('\n');

    o.push_str(&format!("\n【结论】{}\n", a.verdict));

    if !a.already_have.is_empty() {
        o.push_str(&format!("\n库里已有：{}\n", a.already_have.join("、")));
    }

    o.push_str("\n【书目检索】机器查的是「有没有这本书」，不是书的内容\n");
    for t in &a.queries_tried { o.push_str(&format!("  {t}\n")); }

    if a.books.is_empty() {
        o.push_str("\n  （没查到任何相关出版物）\n");
    } else {
        // 按覆盖情况分三组 —— 让「该买哪本」一眼看得出，而不是一堵墙。
        // 年份对不上的**不给购买链接**：买错代际的手册比没有手册更危险。
        let grp = |k: &str| a.books.iter().filter(|b| b.covers == k).collect::<Vec<_>>();
        let (yes, unk, no) = (grp("covers"), grp("unknown"), grp("not_covered"));

        let mut show = |title: &str, list: &Vec<&Book>, with_buy: bool, o: &mut String| {
            if list.is_empty() { return; }
            o.push_str(&format!("\n{title}\n"));
            for b in list {
                let mark = if b.owned { "✓" } else if b.names_model { "★" } else { "·" };
                o.push_str(&format!("  {} {}{}\n", mark, b.title,
                    if b.owned { "   ← 库里已有，别再买" } else { "" }));
                o.push_str(&format!("      {} {}{}{}{}\n",
                    b.publisher,
                    b.year.map(|y| y.to_string()).unwrap_or_default(),
                    b.pages.map(|p| format!(" · {p}页")).unwrap_or_default(),
                    b.isbn.as_ref().map(|i| format!(" · ISBN {i}")).unwrap_or_default(),
                    b.years.map(|(x, y)| format!(" · 适用 {x}–{y}")).unwrap_or_default()));
                if b.owned || !with_buy { continue; }
                let lend = match b.ebook_access.as_str() {
                    "public"        => Some("📖 公有域，可直接在线读"),
                    "borrowable"    => Some("📖 **可合法在线借阅** —— 先借来翻，确认覆盖本车再买"),
                    "printdisabled" => Some("📖 仅限印刷障碍读者（一般人只能买纸质）"),
                    "no_ebook"      => Some("📕 没有合法电子版，只能买纸质"),
                    _ => None,
                };
                if let Some(l) = lend { o.push_str(&format!("      {l}\n")); }
                for l in &b.buy { o.push_str(&format!("      → {:<28} {}\n", l.what, l.url)); }
            }
        };

        show("【✅ 年份覆盖本车 —— 就买这个】", &yes, true, &mut o);
        let unk_named: Vec<&Book> = unk.iter().filter(|b| b.names_model || b.owned).cloned().collect();
        let unk_other: Vec<&Book> = unk.iter().filter(|b| !b.names_model && !b.owned).cloned().collect();
        show("【⚠️ 书名没写年段，需翻版权页核对】", &unk_named, true, &mut o);
        if !unk_other.is_empty() {
            // 同厂牌但没点到本车型、也没写年段 —— 列出来备查，但**不给购买链接**。
            // 给链接等于变相推荐，而它们多半是别的车系（实测 Ducati 查询里混进了
            // 748/916/996 水冷四气门超跑和单缸车，跟 GT1000 的气冷双缸毫无关系）。
            o.push_str("\n【同厂牌其他出版物 —— 未必覆盖本车，不给购买入口】\n");
            for b in &unk_other {
                o.push_str(&format!("  · {}{}\n", b.title,
                    b.isbn.as_ref().map(|i| format!("  (ISBN {i})")).unwrap_or_default()));
            }
        }

        if !no.is_empty() {
            o.push_str("\n【❌ 年份对不上本车 —— 不要买】\n");
            for b in &no {
                o.push_str(&format!("  ✗ {}\n      适用 {} ，本车是 {} —— 差了代际，买错比没有更危险\n",
                    b.title,
                    b.years.map(|(x, y)| format!("{x}–{y}")).unwrap_or_default(), a.year));
            }
        }
        o.push_str("\n  ✓库里已有  ★书名点到本车型\n");
    }

    for n in &a.notes { o.push_str(&format!("\n{n}\n")); }

    o.push_str("\n【手册怎么买】以上每本书的直达入口按 ISBN 生成；书目没覆盖到的走下面这几条\n");
    for l in &a.leads {
        o.push_str(&format!("  [{}] {}\n      {}\n", l.source_level, l.what, l.url));
    }

    o.push_str("\n【厂商官方源 + 图纸】以下是逐厂牌实测过的直达入口（2026-09-16）：\n");
    o.push_str("  · **线路图/电路图** 在维修手册里面 —— 没有单独的免费来源，买手册就有\n");
    o.push_str("  · **爆炸图 + 件号(fiche)** 在线免费，但站点普遍封机器人 —— 只能你去点\n");
    o.push_str("  · 逐厂牌实测结论：**没有任何厂牌提供可机器查询的官方书号接口**；\n");
    o.push_str("    证据与「别再试的死路」记在 knowledge/官方资料源.yml\n");
    for l in &a.diagrams {
        o.push_str(&format!("  [{}] {}\n      {}\n", l.source_level, l.what, l.url));
    }

    o.push_str("\n【下一步】\n");
    for s in &a.next_steps { o.push_str(&format!("  {s}\n")); }

    o.push_str("\n⚠️ 本命令**不下载任何文件**。手册内容进库的唯一路径是实体扫描 + OCR。\n\
                 原因：网上抓来的是 C 级，作业单只吃 A/B；且权利不明的扫描件不可下载。\n");
    o
}
