//! 扫描平台目录 → 建索引
//!
//! 幂等：每次索引先清空该车辆的旧行再重建。
//! 索引里的每一行都能从文件重新导出 —— 删库不丢数据。

use crate::platform::{Platform, Vehicle};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_yaml_ng::Value;
use std::path::Path;
use std::process::Command;

pub struct Stats {
    pub vehicles: usize,
    pub excludes: usize,
    pub specs_verified: usize,
    pub specs_pending: usize,
    pub manuals: usize,
    pub manual_pages: usize,
    pub photos: usize,
    pub logs: usize,
    pub manuals_excluded: usize,
}

pub fn index_vehicle(conn: &mut Connection, _p: &Platform, v: &Vehicle) -> Result<Stats> {
    let now = Utc::now().to_rfc3339();
    let tx = conn.transaction()?;

    // 文件是真相源：先清空该车旧索引（级联清 excludes/specs/manuals/photos/logs）
    tx.execute("DELETE FROM vehicles WHERE id = ?1", params![v.id])?;

    let yml_path = v.yml();
    let raw = std::fs::read_to_string(&yml_path)
        .with_context(|| format!("读不到 {}", yml_path.display()))?;
    let doc: Value = serde_yaml_ng::from_str(&raw).context("VEHICLE.yml 不是合法 YAML")?;

    let gs = |k: &str| doc.get(k).and_then(|x| x.as_str()).map(|s| s.to_string());
    let gi = |k: &str| doc.get(k).and_then(|x| x.as_i64());

    // excludes 指纹先算出来，随车辆行一起写入
    let excl_list: Vec<String> = doc.get("applicability_excludes")
        .and_then(|x| x.as_sequence())
        .map(|l| l.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let fp = excludes_fingerprint(&excl_list);

    tx.execute(
        "INSERT INTO vehicles(id,make,model,year,engine_code,source_path,excludes_fp,indexed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![v.id, gs("make"), gs("model"), gi("year"), gs("engine_code"),
                yml_path.to_string_lossy(), fp, now],
    )?;

    // ── 适用性排除项：检索时硬过滤，防 camhead 用到 hexhead 资料 ──
    let mut n_ex = 0usize;
    let excludes: Vec<String> = excl_list.clone();
    let mut _unused: Vec<String> = Vec::new();
    if let Some(list) = doc.get("applicability_excludes").and_then(|x| x.as_sequence()) {
        for t in list.iter().filter_map(|x| x.as_str()) {
            _unused.push(t.to_string());
            tx.execute(
                "INSERT OR IGNORE INTO applicability_excludes(vehicle_id,term) VALUES(?1,?2)",
                params![v.id, t],
            )?;
            n_ex += 1;
        }
    }

    // ── 已核实规格：展平成 item.field，来源级别从 "A｜出处" 解析 ──
    let mut n_ok = 0usize;
    if let Some(map) = doc.get("specs_verified").and_then(|x| x.as_mapping()) {
        for (group, body) in map {
            let g = group.as_str().unwrap_or("?");
            let body_map = match body.as_mapping() { Some(m) => m, None => continue };
            let (level, sref) = parse_source(body_map.get(Value::from("source")));
            // 出处页码：A/B 级没有它就不满足「可回溯原图」的定义
            let page = body_map.get(Value::from("page")).and_then(|x| x.as_i64());
            // 逐字段出处覆盖。
            // 监工 2026-09-14-b 抓到：spark_plug 组共用一个 "A｜p160"，
            // 但 p160 只给了型号与间隙，**数量 4 是推算的**（twin-spark 每缸 2 支 × 2 缸），
            // 却跟着继承了 A 级页码徽章 —— 一条假 A 级。
            // 组级出处只对「确实出自那一页」的字段成立；推算值必须自报是推算。
            let fs = body_map.get(Value::from("field_sources")).and_then(|x| x.as_mapping());
            for (k, val) in body_map {
                let key = k.as_str().unwrap_or("?");
                if key == "source" || key == "page" || key == "field_sources" { continue; }
                let item = format!("{g}.{key}");
                let fo = fs.and_then(|m| m.get(Value::from(key))).and_then(|x| x.as_mapping());
                let (mut lvl, mut sr, mut pg) = (level.clone(), sref.clone(), page);
                if let Some(o) = fo {
                    let (l2, s2) = parse_source(o.get(Value::from("source")));
                    lvl = l2; sr = s2;
                    pg = o.get(Value::from("page")).and_then(|x| x.as_i64());
                }
                // canonical：语义身份。默认 = item 完整路径（零误报）；
                // field_sources 里可显式声明，让「同一颗螺栓的两处记录」自动参与冲突检测。
                let canon = fo.and_then(|o| o.get(Value::from("canonical")))
                    .and_then(|x| x.as_str()).map(|s| s.to_string())
                    .unwrap_or_else(|| item.clone());
                // 已核实规格必须是 A/B 级。若某字段的 field_source 把级别降成了 C/D,
                // 这是**录入错误**(已核实的东西不该是 C/D)——拒这一条并告警,
                // **不要让它用 `?` 拖垮整车索引**(那会让一处笔误使整台车凭空消失)。
                if lvl != "A" && lvl != "B" {
                    eprintln!("⚠️ {}:{item} 的 field_source 级别是 {lvl}(非 A/B),                               已核实规格不能是 C/D —— 跳过此条。要么修 source,要么移到 specs_pending",
                              v.id);
                    continue;
                }
                tx.execute(
                    "INSERT INTO specs(vehicle_id,item,canonical,value,source_level,source_ref,page,pending,source_path,indexed_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9)",
                    params![v.id, item, canon, scalar(val), lvl, sr, pg,
                            yml_path.to_string_lossy(), now],
                )?;
                n_ok += 1;
            }
        }
    }

    // ── 待核实规格：pending=1，绝不进作业单 ──
    //
    // 级别取 YAML 里写的 `level`（早前这里硬编码成 D，把文件里的 C 改写成了 D ——
    // 真相源说的话被索引改写，正是本项目反复栽跟头的那一类）。
    // C 与 D 的区别是有用的：C 是多个独立来源指向同一个值，可作交叉参考；
    // D 是没有依据的推测，连参考都不算。
    // 待核实项声称自己是 A/B 是自相矛盾（真有官方出处就该进 specs_verified），
    // 一律降为 D 并告警 —— 往严处落。
    let mut n_pend = 0usize;
    if let Some(list) = doc.get("specs_pending").and_then(|x| x.as_sequence()) {
        for e in list {
            let item = e.get("item").and_then(|x| x.as_str()).unwrap_or("?");
            let note = e.get("note").and_then(|x| x.as_str()).unwrap_or("");
            let raw = e.get("level").and_then(|x| x.as_str()).unwrap_or("D").trim();
            let level = match raw {
                "C" | "c" => "C",
                "D" | "d" | "无" | "" => "D",
                other => {
                    eprintln!("⚠️ {}: 待核实项「{item}」声称级别 {other} —— \
                               待核实却有权威出处是自相矛盾，已降为 D", v.id);
                    "D"
                }
            };
            // 候选值展示出来，但它永远进不了 specs_usable（视图只收 A/B 且 pending=0）
            let shown = e.get("candidate").and_then(|x| x.as_str())
                .map(|c| format!("（待核实·{level}级候选）{c}"))
                .unwrap_or_else(|| "（待核实）".to_string());
            tx.execute(
                "INSERT INTO specs(vehicle_id,item,canonical,value,source_level,source_ref,page,pending,source_path,indexed_at)
                 VALUES(?1,?2,?2,?3,?4,?5,NULL,1,?6,?7)",
                params![v.id, item, shown, level, note, yml_path.to_string_lossy(), now],
            )?;
            n_pend += 1;
        }
    }

    // ── 手册 ──
    let (n_man, n_pages, n_excl) = index_manuals(&tx, v, &now, &excludes)?;

    // ── 照片 ──
    let mut n_photo = 0usize;
    for f in walk(&v.photos_dir(), &["jpg", "jpeg", "png", "heic"]) {
        let (phase, kind) = parse_photo_name(&f);
        tx.execute(
            "INSERT OR IGNORE INTO photos(vehicle_id,shot_at,phase,kind,file_path,indexed_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![v.id, Option::<String>::None, phase, kind, f.to_string_lossy(), now],
        )?;
        n_photo += 1;
    }

    // ── 维修日志（每个 .md 一条）──
    let mut n_log = 0usize;
    for f in walk(&v.log_dir(), &["md"]) {
        let title = f.file_stem().map(|s| s.to_string_lossy().to_string());
        tx.execute(
            "INSERT INTO log_entries(vehicle_id,occurred_at,phase,title,source_path)
             VALUES(?1,NULL,NULL,?2,?3)",
            params![v.id, title, f.to_string_lossy()],
        )?;
        n_log += 1;
    }

    tx.commit()?;
    Ok(Stats { vehicles: 1, excludes: n_ex, specs_verified: n_ok, specs_pending: n_pend,
               manuals: n_man, manual_pages: n_pages, photos: n_photo, logs: n_log,
               manuals_excluded: n_excl })
}

/// 判定一份资料是否适用于本车。
///
/// 三道判据，任一命中即不适用：
/// 1. 归档目录 `参考-非本车型/` —— 人工明确标记
/// 2. **相对路径**（相对车辆目录）命中 excludes 词
/// 3. 文件内容前两页命中 excludes 词 —— 命名中性但内容是别代的，靠这道兜住
///
/// 【2026-09-14 第二轮监工发现的两个坑，已修】
/// (a) 原来用 `is_alphanumeric()` 做词边界 —— **汉字也是 alphanumeric**，
///     于是 `lc维修手册` 整串是一个 token，短词永远匹配不上。
///     本项目文件名以中文为主，这不是边角情况。改用 `is_ascii_alphanumeric()`，
///     让汉字充当分隔符。另对短词补一条「紧邻汉字」的子串判定，
///     覆盖 `R1200GSLC` 这类连写（GSLC 是水冷车常见写法）。
/// (b) 原来匹配的是**完整绝对路径**。用户的素材目录名就含 `BMW`
///     （`…/维修_2010 BMW R1200GS/`），平台哪天落到那种路径下会整库被判不适用。
///     改为只匹配相对于车辆目录的部分。
fn judge_applicability(path: &Path, vehicle_dir: &Path, excludes: &[String],
                       head_text: &str) -> (bool, Option<String>) {
    let rel = path.strip_prefix(vehicle_dir).unwrap_or(path)
        .to_string_lossy().to_lowercase();
    if rel.contains("参考-非本车型") {
        return (false, Some("归档于「参考-非本车型」".to_string()));
    }

    // ASCII 分词：汉字视作分隔符，于是 "lc维修手册" → ["lc"]
    let tokens: Vec<&str> = rel.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty()).collect();

    for t in excludes {
        let tl = t.to_lowercase();
        if tl.is_empty() { continue; }

        // 按**字符集**分流，不是按长度：
        //  · 纯 ASCII 短词（LC）→ 精确 token 相等。汉字已在分词时充当分隔符，
        //    所以 "lc维修手册" → ["lc"] 能命中，而 "lcd"/"calc" 不会误伤。
        //    连写变体（R1200GSLC）靠在 excludes 里显式加 "GSLC" 覆盖，
        //    不用 starts_with/ends_with —— 那会让 calc、lcd 全部中枪（实测过）。
        //  · 含中文或较长的词 → 直接子串匹配。中文没有词边界问题，
        //    "水冷" 必须能在 "水冷维修手册" 里命中。
        let ascii_short = tl.is_ascii() && tl.len() < 4;
        let hit = if ascii_short {
            tokens.iter().any(|w| *w == tl)
        } else {
            rel.contains(&tl)
        };
        if hit {
            return (false, Some(format!("路径命中排除项：{t}")));
        }
        // 内容判据：只看开头（标题页/版权页），避免正文偶然提及导致误伤。
        // 短词不做内容匹配 —— 正文里出现一个 "LC" 说明不了什么。
        if !ascii_short && head_text.to_lowercase().contains(&tl) {
            return (false, Some(format!("内容命中排除项：{t}")));
        }
    }
    (true, None)
}

/// "A｜BMW 车主手册 技术数据" → ("A", "BMW 车主手册 技术数据")
/// 取不到级别时退回 D —— 宁可当成不可用，也不假装可靠。
fn parse_source(v: Option<&Value>) -> (String, Option<String>) {
    let s = match v.and_then(|x| x.as_str()) { Some(s) => s, None => return ("D".into(), None) };
    for sep in ['｜', '|'] {
        if let Some((lvl, rest)) = s.split_once(sep) {
            let lvl = lvl.trim().to_uppercase();
            if matches!(lvl.as_str(), "A" | "B" | "C" | "D") {
                return (lvl, Some(rest.trim().to_string()));
            }
        }
    }
    ("D".into(), Some(s.to_string()))
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => serde_yaml_ng::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

fn index_manuals(tx: &rusqlite::Transaction, v: &Vehicle, now: &str,
                 excludes: &[String]) -> Result<(usize, usize, usize)> {
    let mut n_man = 0usize;
    let mut n_pages = 0usize;
    let mut n_excl = 0usize;

    // ── 先收「页目录」形态的手册 ────────────────────────────────
    // 形态：<手册目录>/pages/pNNN.md（+ 同名 pNNN.jpg）
    // 由 tools/pdf2pages.sh 产出。页号与原 PDF 完全一致，
    // 所以规格里所有 `page` 出处**原样继续解析**，不需要改一个字。
    let mut page_dirs: Vec<std::path::PathBuf> = Vec::new();
    collect_page_dirs(&v.manuals_dir(), &mut page_dirs);
    let mut covered: Vec<std::path::PathBuf> = Vec::new();   // 已被页目录覆盖的手册目录

    for pd in &page_dirs {
        let book = match pd.parent() { Some(b) => b.to_path_buf(), None => continue };
        covered.push(book.clone());
        let title = book.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();

        // 读出 pNNN.md，按页号排序
        let mut pages: Vec<(i64, String, Option<String>)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(pd) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("md") { continue; }
                let raw = match std::fs::read_to_string(&p) { Ok(t) => t, Err(_) => continue };
                let (no, img, body) = parse_page_md(&raw, &p);
                let img_abs = img.map(|i| pd.join(i).to_string_lossy().to_string())
                                 .filter(|i| std::path::Path::new(i).exists());
                pages.push((no, body, img_abs));
            }
        }
        if pages.is_empty() { continue; }
        pages.sort_by_key(|x| x.0);

        let head: String = pages.iter().take(2).map(|x| x.1.clone()).collect::<Vec<_>>().join(" ");
        let (applicable, reason) = judge_applicability(&book, &v.dir, excludes, &head);
        if !applicable { n_excl += 1; }
        let tier = read_tier(&book);

        tx.execute(
            "INSERT INTO manuals(vehicle_id,title,tier,file_path,pages,applicable,exclude_reason,indexed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![v.id, title, tier, pd.to_string_lossy(), pages.len() as i64,
                    applicable as i64, reason, now],
        )?;
        let mid = tx.last_insert_rowid();
        for (no, text, img) in &pages {
            tx.execute(
                "INSERT INTO manual_pages(manual_id,page_no,text,image_path) VALUES(?1,?2,?3,?4)",
                params![mid, no, text, img],
            )?;
            let rid = tx.last_insert_rowid();
            tx.execute("INSERT INTO manual_fts(rowid,text) VALUES(?1,?2)", params![rid, text])?;
            n_pages += 1;
        }
        n_man += 1;
    }

    // ── 再收 PDF 形态 ──────────────────────────────────────────
    for pdf in walk(&v.manuals_dir(), &["pdf"]) {
        // raw/ 是流水线的输入区，处理后会生成带 OCR 文字层的成品。
        // 两边都索引 = 同一本书进两次：检索出重复命中，页数统计翻倍。
        // （2026-09-14 装入 Haynes 后当场暴露：手册 4 份里有 2 份是同一本。）
        let rel = pdf.strip_prefix(&v.manuals_dir()).unwrap_or(&pdf).to_string_lossy().to_string();
        if rel.split('/').any(|seg| seg == "raw" || seg == ".ocr" || seg == "pages") {
            continue;
        }
        // 同一本书已经有页目录了就跳过 PDF —— 否则同一本书进两次，
        // 正是上面那条注释记的老毛病，换个形态又会犯一遍。
        if pdf.parent().map(|d| covered.iter().any(|c| c == d)).unwrap_or(false) {
            continue;
        }
        let title = pdf.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let pages_text = pdf_pages(&pdf)?;
        // 内容判据只取前两页：标题页/版权页会写明适用车型年代
        let head: String = pages_text.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
        let (applicable, reason) = judge_applicability(&pdf, &v.dir, excludes, &head);
        if !applicable { n_excl += 1; }
        let tier = read_tier(pdf.parent().unwrap_or(&pdf));
        tx.execute(
            "INSERT INTO manuals(vehicle_id,title,tier,file_path,pages,applicable,exclude_reason,indexed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![v.id, title, tier, pdf.to_string_lossy(), pages_text.len() as i64,
                    applicable as i64, reason, now],
        )?;
        let mid = tx.last_insert_rowid();
        for (i, text) in pages_text.iter().enumerate() {
            tx.execute(
                "INSERT INTO manual_pages(manual_id,page_no,text,image_path) VALUES(?1,?2,?3,NULL)",
                params![mid, (i + 1) as i64, text],
            )?;
            let rid = tx.last_insert_rowid();
            tx.execute("INSERT INTO manual_fts(rowid,text) VALUES(?1,?2)", params![rid, text])?;
            n_pages += 1;
        }
        n_man += 1;
    }
    Ok((n_man, n_pages, n_excl))
}

/// 找出所有 `pages/` 目录（里面有 pNNN.md 才算）。
fn collect_page_dirs(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    let rd = match std::fs::read_dir(root) { Ok(r) => r, Err(_) => return };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() { continue; }
        let name = e.file_name().to_string_lossy().to_string();
        if name == "raw" || name == ".ocr" { continue; }
        if name == "pages" {
            let has_md = std::fs::read_dir(&p).map(|r| r.flatten().any(|x|
                x.path().extension().and_then(|e| e.to_str()) == Some("md"))).unwrap_or(false);
            if has_md { out.push(p); continue; }
        }
        collect_page_dirs(&p, out);
    }
}

/// 解析 pNNN.md 的前置块，返回 (页号, 图片文件名, 正文)。
///
/// 页号优先取前置块里的 `pdf_page`；没有就退回文件名里的数字。
/// **绝不猜**：两者都拿不到时返回 0，调用方排序时它会排在最前面，一眼看得出不对。
fn parse_page_md(raw: &str, path: &Path) -> (i64, Option<String>, String) {
    let mut no: i64 = 0;
    let mut img: Option<String> = None;
    let mut body = raw;
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(idx) = rest.find("\n---") {
            let (fm, after) = rest.split_at(idx);
            for line in fm.lines() {
                if let Some(x) = line.strip_prefix("pdf_page:") { no = x.trim().parse().unwrap_or(0); }
                if let Some(x) = line.strip_prefix("image:") { img = Some(x.trim().to_string()); }
            }
            body = after.trim_start_matches("\n---").trim_start_matches('\n');
        }
    }
    if no == 0 {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let digits: String = stem.chars().filter(|c| c.is_ascii_digit()).collect();
        no = digits.parse().unwrap_or(0);
    }
    (no, img, body.to_string())
}

/// 来源级别**必须显式声明**，不能从路径猜。
///
/// 监工 2026-09-14-c：此前是 `path.contains("official") ? A : B`，
/// 于是一份**社区网站转录的** 2005 REP-ROM 扭力表被打成 B 级送进作业 ——
/// 而那个目录自己的 README 写着「交叉验证前仍是 C 级，不进作业单」。
///
/// 规则：同目录下放一个 TIER 文件，内容是 A / B / C。
/// 没有声明就按 **C** 处理（不是 B）—— 来路不明的资料不该默认权威。
fn read_tier(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("TIER")).ok()
        .map(|t| t.trim().to_uppercase())
        .filter(|t| t == "A" || t == "B" || t == "C")
        .unwrap_or_else(|| {
            eprintln!("⚠️ {} 未声明来源级别（缺 TIER 文件），按 C 处理", dir.display());
            "C".to_string()
        })
}

/// 用 pdftotext -layout 提取（已验证：技术数字 100% 准，版面靠 -layout 保持）
/// 换页符 \f 分页，页码与原 PDF 一致 —— 数字要回原图确认时用得上。
fn pdf_pages(pdf: &Path) -> Result<Vec<String>> {
    let out = Command::new("pdftotext")
        .args(["-layout", &pdf.to_string_lossy(), "-"])
        .output()
        .context("跑不了 pdftotext（brew install poppler）")?;
    if !out.status.success() {
        anyhow::bail!("pdftotext 失败: {}", String::from_utf8_lossy(&out.stderr));
    }
    let mut pages: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\u{c}')
        .map(|s| s.to_string())
        .collect();
    // pdftotext 在最后一页之后也输出换页符，split 因此多出一个空串。
    // 只弹掉**末尾**的空页：中间的真空白页要保留，否则页码会和原 PDF 错位，
    // 而"数字回原图确认"全靠页码对得上。
    if pages.last().is_some_and(|s| s.trim().is_empty()) {
        pages.pop();
    }
    Ok(pages)
}

/// excludes 的顺序无关指纹。VEHICLE.yml 改了排除项但忘记重新索引时，
/// doctor 靠比对这个值发现「过滤规则已过期」。
pub fn excludes_fingerprint(list: &[String]) -> String {
    let mut v: Vec<String> = list.iter().map(|s| s.trim().to_lowercase()).collect();
    v.sort();
    let joined = v.join("\u{1}");
    let mut h: u64 = 1469598103934665603;      // FNV-1a
    for b in joined.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(1099511628211); }
    format!("{h:016x}")
}

fn walk(dir: &Path, exts: &[&str]) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    fn rec(d: &Path, exts: &[&str], out: &mut Vec<std::path::PathBuf>) {
        let rd = match std::fs::read_dir(d) { Ok(r) => r, Err(_) => return };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            if p.is_dir() { rec(&p, exts, out); }
            else if let Some(ext) = p.extension().map(|x| x.to_string_lossy().to_lowercase()) {
                if exts.contains(&ext.as_str()) { out.push(p); }
            }
        }
    }
    rec(dir, exts, &mut out);
    out.sort();
    out
}

/// 文件名形如 20260913-211530_P1_… → (阶段, 类型)
fn parse_photo_name(p: &Path) -> (Option<String>, Option<String>) {
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let parts: Vec<&str> = stem.split('_').collect();
    let phase = parts.get(1).filter(|s| s.starts_with('P')).map(|s| s.to_string());
    let kind = p.parent()
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().to_string());
    (phase, kind)
}

/// 索引库不存在时，从文件全量建一次；返回是否新建了。
/// 文件是真相源、库只是可重建的索引 —— 库缺了就该自己补上，
/// 而不是让新用户打开 app 看到一片 0。建到一半失败则删掉半成品，下次启动重试。
pub fn ensure(p: &Platform) -> Result<bool> {
    let db_path = p.index_db();
    if db_path.exists() { return Ok(false); }
    let built = (|| -> Result<()> {
        let mut conn = crate::db::open(&db_path)?;
        for id in p.list_vehicles()? {
            let v = p.resolve_vehicle(Some(&id))?;
            index_vehicle(&mut conn, p, &v)?;
        }
        Ok(())
    })();
    if let Err(e) = built {
        let _ = std::fs::remove_file(&db_path);
        return Err(e.context("首次自动建索引失败"));
    }
    Ok(true)
}
