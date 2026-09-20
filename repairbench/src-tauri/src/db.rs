//! SQLite 索引 —— **可重建的缓存，不是真相源**
//!
//! 删掉 .index/repairbench.db 不丢任何数据，重跑 `repairbench index` 即可。
//! 因此这里不存任何只在数据库里存在的内容：每一行都能从文件重新导出。

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 5;

pub fn open(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建不了索引目录 {}", parent.display()))?;
    }
    let conn = Connection::open(db_path)
        .with_context(|| format!("打不开索引库 {}", db_path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    // 【2026-09-14 第二轮监工发现】此前直接 CREATE TABLE IF NOT EXISTS 后
    // 无条件把 schema_version 刷成当前值：v1 旧库跑新二进制 → 表结构没变、
    // 脏数据留存、版本号却谎报 v2，之后 index/search 吐裸 SQL 错。
    // 索引本就可重建，正确做法是**检出版本不符就明确要求重建**，而不是假装升级成功。
    let existing: Option<i32> = conn
        .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok());
    if let Some(v) = existing {
        if v != SCHEMA_VERSION {
            anyhow::bail!(
                "索引库 schema 版本不符：库是 v{v}，本程序需要 v{SCHEMA_VERSION}。\n\
                 索引是可重建的缓存，删掉重建即可（文件才是真相源）：\n\
                   rm -rf <平台根>/.index && repairbench index --all"
            );
        }
    }
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- 车辆索引。source_path 指回 VEHICLE.yml，内容以文件为准
CREATE TABLE IF NOT EXISTS vehicles (
    id           TEXT PRIMARY KEY,
    make         TEXT,
    model        TEXT,
    year         INTEGER,
    engine_code  TEXT,
    source_path  TEXT NOT NULL,
    -- excludes 指纹：VEHICLE.yml 改了排除项但没重新索引时，doctor 能发现。
    -- 过滤结果是索引时固化进 manuals.applicable 的，不重索引就不生效（监工指出）。
    excludes_fp  TEXT,
    indexed_at   TEXT NOT NULL
);

-- 适用性排除项：检索时硬过滤，防止 camhead 用到 hexhead 的资料
CREATE TABLE IF NOT EXISTS applicability_excludes (
    vehicle_id TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    term       TEXT NOT NULL,
    PRIMARY KEY (vehicle_id, term)
);

-- 规格。source_level 是硬约束：A 官方 / B 权威三方 / C 社区 / D 推测
-- 查询作业单时必须过滤掉 C/D
CREATE TABLE IF NOT EXISTS specs (
    id           INTEGER PRIMARY KEY,
    vehicle_id   TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    item         TEXT NOT NULL,
    -- 语义身份：同一颗螺栓无论写在哪个组，canonical 相同就参与冲突检测。
    -- 默认 = item（完整路径），零误报；跨组关联需在 YAML 显式声明 canonical。
    canonical    TEXT NOT NULL,
    value        TEXT NOT NULL,
    source_level TEXT NOT NULL CHECK (source_level IN ('A','B','C','D')),
    source_ref   TEXT,
    -- 出处页码。A/B 级必须有，否则「可回溯原图」这个定义就是空的
    page         INTEGER,
    pending      INTEGER NOT NULL DEFAULT 0,
    source_path  TEXT NOT NULL,
    indexed_at   TEXT NOT NULL,
    -- 真正的不变式：C/D 级只能以 pending 形态存在，绝不会被当成可用数据读出。
    -- 此前只有字母表 CHECK，C+pending=0 能插进来（监工 2026-09-14 实测）。
    CHECK (source_level IN ('A','B') OR pending = 1)
);
CREATE INDEX IF NOT EXISTS idx_specs_lookup ON specs(vehicle_id, item);

-- 手册文件与页。全文进 FTS，但原图页码保留 —— 数字要回原图确认
CREATE TABLE IF NOT EXISTS manuals (
    id          INTEGER PRIMARY KEY,
    vehicle_id  TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    tier        TEXT,
    file_path   TEXT NOT NULL,
    pages       INTEGER,
    -- 适用性：0 = 不适用本车（hexhead/LC 资料），默认检索必须排除。
    -- 没有这一列时，hexhead 扭力值 PDF 被打成 tier B 挂在 GS 名下，
    -- 检索一上线就会与官方手册并排返回，而 B 级按定义「可直接用于作业」。
    applicable     INTEGER NOT NULL DEFAULT 1,
    exclude_reason TEXT,
    indexed_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS manual_pages (
    id         INTEGER PRIMARY KEY,
    manual_id  INTEGER NOT NULL REFERENCES manuals(id) ON DELETE CASCADE,
    page_no    INTEGER NOT NULL,
    text       TEXT,
    -- v5：这一页的原图路径。双层检索的下半截 ——
    -- 文本层负责定位，**数值一律回原图确认**。
    -- 没有它，「回原图」只能靠人去翻 2GB 的整本 PDF。
    image_path TEXT
);
-- ── 安全视图：所有消费端只准查这两个，不准直接碰底表 ──────────────
-- 【2026-09-14 第二轮监工】过滤此前是 cmd_search 里拼的一个 SQL 片段。
-- 阶段 2 UI / 阶段 5 MCP / 阶段 6 HTTP 各自都得记得再写一遍 AND applicable=1，
-- 漏一处就把 hexhead 扭力值当「B 级·可用于作业」返回。
-- 视图把过滤做成**不可绕过的默认**，而不是每个调用方的自觉。

-- 冲突：同一语义身份(canonical)有多个不同的已核实值。
-- 两个都带出处、都看着可信 —— 这种情况下**一个都不能用**，
-- 返回矛盾的两个扭力值比不返回更危险。
CREATE VIEW IF NOT EXISTS spec_conflicts AS
SELECT vehicle_id, canonical,
       COUNT(DISTINCT value) AS n_values,
       GROUP_CONCAT(DISTINCT value) AS values_csv,
       GROUP_CONCAT(DISTINCT item)  AS items_csv,
       GROUP_CONCAT(DISTINCT COALESCE(source_ref,'')||' p'||COALESCE(page,0)) AS sources_csv
FROM specs
WHERE source_level IN ('A','B') AND pending = 0
GROUP BY vehicle_id, canonical
HAVING COUNT(DISTINCT value) > 1;

-- 可用于作业的规格：A/B 级、非 pending、**且该 canonical 没有冲突**。
-- 有冲突的 item 在解决前从这里消失 —— 消费端根本查不到矛盾值。
CREATE VIEW IF NOT EXISTS specs_usable AS
SELECT s.id, s.vehicle_id, s.item, s.canonical, s.value, s.source_level, s.source_ref, s.page, s.source_path
FROM specs s
WHERE s.source_level IN ('A','B') AND s.pending = 0
  AND NOT EXISTS (
    SELECT 1 FROM spec_conflicts c
    WHERE c.vehicle_id = s.vehicle_id AND c.canonical = s.canonical
  );

-- 适用本车的手册页：不适用的资料在这里根本不存在
CREATE VIEW IF NOT EXISTS manual_pages_applicable AS
SELECT mp.id, mp.manual_id, mp.page_no, mp.text, mp.image_path,
       m.vehicle_id, m.title, m.tier, m.file_path
FROM manual_pages mp
JOIN manuals m ON m.id = mp.manual_id
WHERE m.applicable = 1;

CREATE VIRTUAL TABLE IF NOT EXISTS manual_fts USING fts5(
    text, content='manual_pages', content_rowid='id', tokenize='unicode61'
);

-- 维修日志与照片：同样只是索引
CREATE TABLE IF NOT EXISTS log_entries (
    id          INTEGER PRIMARY KEY,
    vehicle_id  TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    occurred_at TEXT,
    phase       TEXT,
    title       TEXT,
    source_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS photos (
    id          INTEGER PRIMARY KEY,
    vehicle_id  TEXT NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    shot_at     TEXT,
    phase       TEXT,
    kind        TEXT,
    file_path   TEXT NOT NULL UNIQUE,
    indexed_at  TEXT NOT NULL
);
"#,
    )?;
    conn.execute(
        "INSERT INTO meta(key,value) VALUES('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

/// 读库里记录的 schema 版本（不是编译期常量）。
/// doctor 此前打印 SCHEMA_VERSION，对任何库都说 v2 —— 谎报。
pub fn db_schema_version(conn: &Connection) -> Option<i32> {
    conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| {
        r.get::<_, String>(0)
    }).ok().and_then(|v| v.parse().ok())
}

pub fn table_counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let tables = ["vehicles", "applicability_excludes", "specs",
                  "manuals", "manual_pages", "log_entries", "photos"];
    let mut out = Vec::new();
    for t in tables {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {}", t), [], |r| r.get(0))?;
        out.push((t.to_string(), n));
    }
    Ok(out)
}
