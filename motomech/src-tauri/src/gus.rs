//! Gus —— 问答里说话的那个老修车佬（人设 + 数字白名单）
//!
//! ## 为什么人设和数字检查放在一起
//!
//! Gus 的设定是絮絮叨叨：爱讲当年、爱吐槽、爱叮嘱。**话多的角色最容易顺嘴带出一个数**——
//! 「这颗螺栓一般拧个四十来牛」。这正是本项目从第一天就在防的东西：说不出出处的扭力值。
//! 提示词里禁止编数字挡不住这一句，所以 Gus 说完之后还要过一道**程序检查**：
//! 回答里出现的每个数字都必须能在事实层里找到，找不到就整段拦下，只留事实层。
//!
//! 人设只管口吻，不管内容：事实层原样给出、不经 Gus 的嘴。
//!
//! ## 数字白名单的边界（刻意偏严）
//!
//! - 阿拉伯数字一律检查，包括小数。`0.22` 和 `0.220` 视为同一个数。
//! - **换算出来的数也拦**：事实写 0.22 L，Gus 说「220 毫升」→ 拦。换算是人的事，不是 Gus 的。
//! - 中文数字：两个字以上且含 十/百/千/万 的串（如「四十」「五十六」「三十年」），或紧跟计量单位的串
//!   （如「三毫米」「四十来牛」），换算成整数后检查。单个「一颗」「两个」这种量词不查；
//!   「几十年」「十几个」这种约数不查；「十分」「万一」「千万」「零件」等常用词不查。
//!   代价：Gus 讲「三十年前我修过一台」这种带数的往事也会被拦 —— 提示词本来就叫他不带数，宁严勿松。
//! - 英文数字词（twenty / forty-two …）紧跟计量单位时检查。
//! - **带单位的数字要和事实层里同类单位的数字对上**：牛米对牛米、升/毫升对升/毫升、页码对页码。
//!   事实里只有「少 40ml」时，Gus 说「40 牛米」→ 拦（同值≠同件，本项目反复吃过这个亏）。
//!   事实里数值后没写单位的，用规格 key 的后缀判类（`..._nm` 扭力、`..._l` 容量、`..._mm` 长度……）。
//! - 已知限度：**没带单位的裸数字**只按数值匹配；事实里无法判类的数字（family 未知）对任何单位都算匹配。
//!   所以提示词强制 Gus「说数字必须带单位」，这里只是兜底，不是万能。
//! - 豁免：行首的列表序号（`1.` `2)` `3、`）、问题里用户自己说过的数字、车辆 id 里的数字。

/// Gus 的系统提示词。`facts` 是已渲染好的事实层上下文。
pub fn system_prompt(vehicle_id: &str, facts: &str) -> String {
    format!(
"你是 Gus，一个在车库里干了一辈子的老修车师傅，正在帮车主修 {vehicle_id}。\n\
性格：絮絮叨叨、热心、爱讲当年修车的见闻、爱吐槽偷工减料的设计、对安全特别啰嗦。\n\
说话像真人师傅聊天，可以先唠两句再说正事，但正事一定要说清楚。\n\
用提问者的语言回答（中文问就中文答，英文问就英文答）。\n\n\
铁律（比你的性格重要一万倍）：\n\
1. 你的话里**不许出现任何数字**，除非这个数字原样出现在下面的「已核实事实」里。\n\
   扭力、间隙、容量、尺寸、温度、件号、年份……一律照抄事实里的，不换算、不估算、不凭经验说「一般是多少」。\n\
   想表达经验时只说定性的话（「这颗螺栓很容易拧滑丝」），不带数。\n\
   说数字时必须连同事实里的单位一起说（0.22 升、20 牛米），不许只丢一个裸数字。\n\
2. 提到具体数值时，顺口说一句出处（「书上 p22 写的」），让车主知道去哪核对。\n\
3. 某项标了「有互相矛盾的值/冲突」：明确说这个值没定、不能用，别替它选一个。\n\
4. 事实里没有的，就直说「这个我手头的资料里没有」，不要猜。\n\
5. 程序会逐个检查你说的数字，任何一个对不上，你整段话都会被删掉。\n\n\
已核实事实：\n{facts}"
    )
}

/// 清掉模型漏进正文的思考过程。
///
/// 有些推理模型不把思考放进 `reasoning_content`，而是直接写在正文里：`<think>…</think>真正的回答`，
/// 甚至思考里夹着自己对答案的「检查」。这些不是 Gus 说的话，不该给用户看，更不该参与数字检查
/// （思考里的数字不等于最终回答里的数字）。
/// - 有 `</think>`：只留最后一个 `</think>` 之后的部分。
/// - 有 `<think>` 却没收尾：思考没写完、没有最终回答 → 返回 None，宁可没有也不展示半截思考。
pub fn clean_answer(raw: &str) -> Option<String> {
    let text = match raw.rfind("</think>") {
        Some(i) => &raw[i + "</think>".len()..],
        None => {
            if raw.contains("<think>") { return None; }
            raw
        }
    };
    let t = text.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// 白名单检查结果：`Err` 里是事实层找不到的数字（原样写法）。
pub fn check_numbers(answer: &str, facts: &str, question: &str, vehicle_id: &str)
    -> Result<(), Vec<String>>
{
    // allowed:(归一化数值, 单位类) 。类为 None = 事实里没法判类(对任何单位都算匹配)
    let mut allowed: std::collections::HashSet<(String, Option<Fam>)> = std::collections::HashSet::new();
    for line in facts.lines() {
        // 「key = 值」:值后面没写单位时,按 key 的后缀判类(_nm → 扭力 …)
        let (head, tail) = match line.split_once(" = ") {
            Some((h, t)) => (h, Some(t)), None => (line, None),
        };
        for (_, v, f) in extract_numbers(head) { allowed.insert((v, f)); }
        if let Some(t) = tail {
            let suffix = key_family(head);
            for (idx, (_, v, f)) in extract_numbers(t).into_iter().enumerate() {
                let f = if idx == 0 && f.is_none() { suffix } else { f };
                allowed.insert((v, f));
            }
        }
    }
    for src in [question, vehicle_id] {
        for (_, v, f) in extract_numbers(src) { allowed.insert((v, f)); }
    }
    let any_family = |v: &str| allowed.iter().any(|(n, _)| n == v);
    let mut bad: Vec<String> = Vec::new();
    for (raw, v, fam) in extract_numbers(&strip_idioms(&strip_list_markers(answer))) {
        let ok = match fam {
            // 带单位:同类 或 事实里该数判不出类
            Some(f) => allowed.contains(&(v.clone(), Some(f))) || allowed.contains(&(v.clone(), None)),
            // 裸数字:只能按数值匹配
            None => any_family(&v),
        };
        if !ok && !bad.contains(&raw) { bad.push(raw); }
    }
    if bad.is_empty() { Ok(()) } else { Err(bad) }
}

/// 含数字字的成语和常用词，不是数据。替换成等长的占位，保持其余位置不变。
fn strip_idioms(s: &str) -> String {
    const IDIOMS: [&str; 14] = ["百分之百", "十有八九", "一五一十", "千千万万", "乱七八糟", "七上八下",
        "百分百", "万一", "十分", "千万", "十足", "万万", "零件", "零部件"];
    let mut out = s.to_string();
    for w in IDIOMS { out = out.replace(w, &"·".repeat(w.chars().count())); }
    out
}

/// 去掉每行行首的列表序号（`1.` `2)` `3、` `(4)`），它们不是数据。
fn strip_list_markers(s: &str) -> String {
    s.lines().map(|line| {
        let t = line.trim_start();
        let t2 = t.trim_start_matches(|c| c == '(' || c == '（');
        let digits: String = t2.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.len() <= 2 {
            let rest = &t2[digits.len()..];
            for m in [". ", ".", ")", "）", "、"] {
                if let Some(r) = rest.strip_prefix(m) {
                    // `1.5` 是小数不是序号
                    if m == "." && r.chars().next().map_or(false, |c| c.is_ascii_digit()) { break; }
                    return r.to_string();
                }
            }
        }
        line.to_string()
    }).collect::<Vec<_>>().join("\n")
}

/// 单位类:同类才算同一个量。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fam { Torque, Volume, Length, Pressure, Mass, Angle, Voltage, Page }

/// 抽出文本里的数字：(原文写法, 归一化值, 单位类)。
fn extract_numbers(s: &str) -> Vec<(String, String, Option<Fam>)> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ',' && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_digit() && thousands_sep(&chars, i)) { i += 1; }
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() { i += 1; }
            }
            let raw: String = chars[start..i].iter().collect();
            let fam = if is_page_prefix(&chars, start) { Some(Fam::Page) } else { unit_family(&chars, i) };
            out.push((raw.clone(), normalize(&raw.replace(',', "")), fam));
            continue;
        }
        if is_cn_digit(c) {
            let start = i;
            while i < chars.len() && is_cn_digit(chars[i]) { i += 1; }
            let raw: String = chars[start..i].iter().collect();
            // 「几十年」「十几个」是约数，不是数据
            let near_ji = (start > 0 && chars[start - 1] == '几') || chars.get(i) == Some(&'几');
            let has_mult = raw.chars().count() >= 2 && raw.chars().any(|c| "十百千万".contains(c))
                && !near_ji;
            if has_mult || followed_by_unit(&chars, i) {
                if let Some(v) = cn_to_int(&raw) { out.push((raw, v.to_string(), unit_family(&chars, i))); }
            }
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '-') { i += 1; }
            let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
            if let Some(v) = en_to_int(&word) {
                if followed_by_unit(&chars, i) { out.push((word, v.to_string(), unit_family(&chars, i))); }
            }
            continue;
        }
        i += 1;
    }
    out
}

fn thousands_sep(chars: &[char], comma: usize) -> bool {
    // 只把「,」后正好三位数字当千分位（1,200）；「4.5,4.0」这类列表分隔不吞
    let after: String = chars[comma + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
    after.len() == 3
}

fn normalize(n: &str) -> String {
    if let Some((int, frac)) = n.split_once('.') {
        let frac = frac.trim_end_matches('0');
        let int = int.trim_start_matches('0');
        let int = if int.is_empty() { "0" } else { int };
        if frac.is_empty() { int.to_string() } else { format!("{int}.{frac}") }
    } else {
        let t = n.trim_start_matches('0');
        if t.is_empty() { "0".into() } else { t.into() }
    }
}

fn is_cn_digit(c: char) -> bool { "零一二两三四五六七八九十百千万".contains(c) }

fn cn_to_int(s: &str) -> Option<u64> {
    let d = |c: char| "零一二三四五六七八九".find(c).map(|i| (i / 3) as u64)
        .or(if c == '两' { Some(2) } else { None });
    let (mut total, mut section, mut num) = (0u64, 0u64, 0u64);
    for c in s.chars() {
        if let Some(v) = d(c) { num = v; continue; }
        let unit = match c { '十' => 10, '百' => 100, '千' => 1000, '万' => 10000, _ => return None };
        if unit == 10000 { total += (section + num) * unit; section = 0; }
        else { section += (if num == 0 && unit == 10 { 1 } else { num }) * unit; }
        num = 0;
    }
    Some(total + section + num)
}

fn en_to_int(w: &str) -> Option<u64> {
    const ONES: [&str; 20] = ["zero","one","two","three","four","five","six","seven","eight","nine","ten",
        "eleven","twelve","thirteen","fourteen","fifteen","sixteen","seventeen","eighteen","nineteen"];
    const TENS: [&str; 8] = ["twenty","thirty","forty","fifty","sixty","seventy","eighty","ninety"];
    let mut total = 0u64;
    for part in w.split('-') {
        if let Some(i) = ONES.iter().position(|x| *x == part) { total += i as u64; }
        else if let Some(i) = TENS.iter().position(|x| *x == part) { total += 20 + 10 * i as u64; }
        else if part == "hundred" { total = total.max(1) * 100; }
        else { return None; }
    }
    Some(total)
}

/// 紧跟（可隔空格/「来」「多」「几」）一个计量单位 → 它属于哪一类。
fn unit_family(chars: &[char], mut i: usize) -> Option<Fam> {
    while i < chars.len() && (chars[i] == ' ' || "来多几余左右".contains(chars[i])) { i += 1; }
    let rest: String = chars[i..chars.len().min(i + 10)].iter().collect::<String>().to_lowercase();
    // 先长后短(「牛米」先于「牛」)。ASCII 单位后面不能紧跟字母(避免 "l" 命中 "later")
    const UNITS: [(&str, Fam); 36] = [
        ("牛米", Fam::Torque), ("n·m", Fam::Torque), ("n m", Fam::Torque), ("newton", Fam::Torque),
        ("nm", Fam::Torque), ("牛", Fam::Torque),
        ("毫升", Fam::Volume), ("liter", Fam::Volume), ("litre", Fam::Volume), ("ml", Fam::Volume),
        ("升", Fam::Volume), ("l", Fam::Volume),
        ("毫米", Fam::Length), ("厘米", Fam::Length), ("mm", Fam::Length), ("cm", Fam::Length),
        ("英寸", Fam::Length), ("inch", Fam::Length),
        ("bar", Fam::Pressure), ("psi", Fam::Pressure), ("kpa", Fam::Pressure), ("兆帕", Fam::Pressure),
        ("公斤", Fam::Mass), ("kg", Fam::Mass),
        ("degree", Fam::Angle), ("度", Fam::Angle), ("°", Fam::Angle),
        ("volt", Fam::Voltage), ("伏", Fam::Voltage),
        ("欧", Fam::Voltage), ("ohm", Fam::Voltage), ("amp", Fam::Voltage), ("安", Fam::Voltage),
        ("页", Fam::Page), ("page", Fam::Page), ("p.", Fam::Page),
    ];
    for (u, fam) in UNITS {
        if rest.starts_with(u) {
            let ascii = u.chars().all(|c| c.is_ascii_alphabetic());
            let next = rest[u.len()..].chars().next();
            if ascii && next.map_or(false, |c| c.is_ascii_alphabetic()) { continue; }
            return Some(fam);
        }
    }
    None
}

fn followed_by_unit(chars: &[char], i: usize) -> bool { unit_family(chars, i).is_some() }

/// 数字前紧挨着 p / P(且 p 前不是字母)→ 页码,如 p22、p1•2。
fn is_page_prefix(chars: &[char], start: usize) -> bool {
    start >= 1 && (chars[start - 1] == 'p' || chars[start - 1] == 'P')
        && (start == 1 || !chars[start - 2].is_ascii_alphabetic())
}

/// 规格 key 的后缀 → 单位类(`..._nm` 扭力、`..._l` 容量、`..._mm` 长度……)。
fn key_family(key: &str) -> Option<Fam> {
    let k = key.trim().rsplit(']').next().unwrap_or("").trim().to_lowercase();
    let suffix = k.rsplit(|c| c == '_' || c == '.').next().unwrap_or("");
    match suffix {
        "nm" => Some(Fam::Torque),
        "l" | "ml" => Some(Fam::Volume),
        "mm" | "cm" | "in" => Some(Fam::Length),
        "bar" | "psi" | "kpa" => Some(Fam::Pressure),
        "kg" => Some(Fam::Mass),
        "deg" => Some(Fam::Angle),
        "v" => Some(Fam::Voltage),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FACTS: &str = "- [spec|B级 · Haynes p1•2 p22] final_drive_oil.capacity_l = 0.22\n\
                         - [spec|B级 · Haynes 4598 p22] torque.final_drive_drain_nm = 23";

    #[test] fn verbatim_numbers_pass() {
        assert!(check_numbers("书上 p22 写的，加 0.22 升，螺塞 23 牛米。", FACTS, "终传油加多少", "2009-BMW-R1200GS").is_ok());
    }
    #[test] fn trailing_zero_is_same_number() {
        assert!(check_numbers("加 0.220 升", FACTS, "", "x").is_ok());
    }
    #[test] fn invented_number_blocked() {
        assert_eq!(check_numbers("拧到 40 牛米就行", FACTS, "", "x"), Err(vec!["40".into()]));
    }
    #[test] fn converted_number_blocked() {
        // 0.22 L → 220 ml 是换算，Gus 不许做
        assert!(check_numbers("大概 220 毫升", FACTS, "", "x").is_err());
    }
    #[test] fn chinese_numeral_with_multiplier_blocked() {
        assert!(check_numbers("一般拧个四十来牛", FACTS, "", "x").is_err());
    }
    #[test] fn chinese_numeral_matching_fact_passes() {
        assert!(check_numbers("拧二十三牛米", FACTS, "", "x").is_ok());
    }
    #[test] fn chinese_numeral_with_unit_blocked() {
        assert!(check_numbers("间隙留三毫米", FACTS, "", "x").is_err());
    }
    #[test] fn plain_measure_words_not_checked() {
        assert!(check_numbers("这一颗螺栓、两个垫片都要换", FACTS, "", "x").is_ok());
    }
    #[test] fn english_number_word_with_unit_blocked() {
        assert!(check_numbers("torque it to forty-two Nm", FACTS, "", "x").is_err());
        assert!(check_numbers("one bolt, two washers", FACTS, "", "x").is_ok());
    }
    #[test] fn list_markers_exempt_but_decimals_not() {
        assert!(check_numbers("1. 先放油\n2) 再拧回螺塞", FACTS, "", "x").is_ok());
        assert!(check_numbers("1.5 升就够", FACTS, "", "x").is_err());
    }
    #[test] fn question_and_vehicle_numbers_allowed() {
        assert!(check_numbers("你这台 2009 的车，跑了 60000 公里", FACTS, "我的车跑了 60000 公里", "2009-BMW-R1200GS").is_ok());
    }
    #[test] fn chatty_idioms_not_numbers() {
        assert!(check_numbers("这个零件十分重要，千万别省，万一断了就麻烦了，我修了几十年车", FACTS, "", "x").is_ok());
    }
    #[test] fn anecdote_with_number_is_blocked_by_design() {
        assert!(check_numbers("三十年前我修过一台", FACTS, "", "x").is_err());
    }
    #[test] fn alnum_codes_are_checked() {
        // 车型代号里的数字也算数据：不在事实里就拦
        assert!(check_numbers("你这是 K25 发动机", FACTS, "", "2009-BMW-R1200GS").is_err());
        assert!(check_numbers("R1200GS 的老毛病", FACTS, "", "2009-BMW-R1200GS").is_ok());
    }
    #[test] fn think_block_stripped() {
        assert_eq!(clean_answer("<think>先想想 99</think>\n加 0.22 升。").as_deref(), Some("加 0.22 升。"));
    }
    #[test] fn bare_closing_think_tag_stripped() {
        // 实测:有的模型只吐 </think>,前面是漏出来的思考
        assert_eq!(clean_answer("(检查一遍数字)\n</think>\n加 0.22 升。").as_deref(), Some("加 0.22 升。"));
    }
    #[test] fn unfinished_think_yields_nothing() {
        assert_eq!(clean_answer("<think>还在想,没写完 77"), None);
    }
    #[test] fn plain_answer_untouched() {
        assert_eq!(clean_answer("  加 0.22 升  ").as_deref(), Some("加 0.22 升"));
        assert_eq!(clean_answer("   "), None);
    }
    #[test] fn number_in_discarded_thinking_is_not_judged() {
        // 思考里的数字不算 Gus 的话:检查的是清理后的正文
        let cleaned = clean_answer("<think>也许 40 牛米?</think>加 0.22 升。").unwrap();
        assert!(check_numbers(&cleaned, FACTS, "", "x").is_ok());
    }
    // 真实事实文本(来自终传油问答):其中的「40ml」曾让模型编的「40 牛米」混过白名单
    const REAL: &str = "- [spec|B级 · Haynes p1•2 p22] final_drive_oil.capacity_l = 0.22\n\
        - [spec|B级 · Haynes p1•2 = 0.22 L。旧档案候选里有个「2010 款 180ml」—— 按那个加会**少 40ml** p22] final_drive_oil.note = x\n\
        - [spec|B级 · Haynes 1•2 p22] torque.final_drive_oil_plugs_nm = 20\n\
        - [spec|B级 · Haynes 4•2 p153] torque_chassis_4_2.final_drive_unit_pivot_bolt_nm = 100";
    #[test] fn same_number_different_unit_is_blocked() {
        // 回归:40 只作为「40ml」存在于事实里,不能放行「40 牛米」
        assert!(check_numbers("螺塞拧到 40 牛米就行", REAL, "", "x").is_err());
        assert!(check_numbers("tighten to 40 Nm", REAL, "", "x").is_err());
    }
    #[test] fn page_number_is_not_a_torque() {
        // 22 只作为页码 p22 存在,不能当扭力
        assert!(check_numbers("螺塞拧到 22 牛米", REAL, "", "x").is_err());
        assert!(check_numbers("书上 p22 写的", REAL, "", "x").is_ok());
    }
    #[test] fn units_matching_key_suffix_pass() {
        assert!(check_numbers("加 0.22 升，螺塞 20 牛米，枢轴螺栓 100 牛米", REAL, "", "x").is_ok());
        assert!(check_numbers("capacity is 0.22 L, plug 20 Nm", REAL, "", "x").is_ok());
    }
    #[test] fn volume_number_as_torque_blocked_both_ways() {
        assert!(check_numbers("加 20 升油", REAL, "", "x").is_err());   // 20 是扭力
        assert!(check_numbers("拧 0.22 牛米", REAL, "", "x").is_err()); // 0.22 是容量
    }
    #[test] fn quoting_the_note_with_its_own_unit_passes() {
        assert!(check_numbers("旧档案那个 180 毫升，会少 40 毫升", REAL, "", "x").is_ok());
    }
    #[test] fn cn_conversion() {
        assert_eq!(cn_to_int("五十六"), Some(56));
        assert_eq!(cn_to_int("十二"), Some(12));
        assert_eq!(cn_to_int("一百零五"), Some(105));
        assert_eq!(cn_to_int("两千"), Some(2000));
    }
}
