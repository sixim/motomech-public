//! 平台路径与车辆解析
//!
//! 真相源是文件系统。这里只负责定位，不缓存任何内容。
//! 车辆解析逻辑与 tools/_common.sh 保持一致：参数 > 环境变量 > .current-vehicle，
//! 一个都没有就报错并列出可选项 —— **绝不猜**。猜错会把 GS 的数据写进 XL1200。

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub const DEFAULT_ROOT: &str = "Ai Developer/AI+摩托维修助理";

pub struct Platform {
    pub root: PathBuf,
}

impl Platform {
    pub fn new(root: Option<String>) -> Result<Self> {
        let root = match root {
            Some(r) => PathBuf::from(shellexpand(&r)),
            None => {
                let home = std::env::var("HOME").context("读不到 $HOME")?;
                PathBuf::from(home).join(DEFAULT_ROOT)
            }
        };
        if !root.is_dir() {
            return Err(anyhow!("平台目录不存在: {}\n用 --root 指定，或设 REPAIRBENCH_ROOT", root.display()));
        }
        Ok(Self { root })
    }

    pub fn vehicles_dir(&self) -> PathBuf { self.root.join("vehicles") }
    pub fn index_db(&self) -> PathBuf { self.root.join(".index").join("repairbench.db") }
    fn current_file(&self) -> PathBuf { self.root.join(".current-vehicle") }

    pub fn list_vehicles(&self) -> Result<Vec<String>> {
        let dir = self.vehicles_dir();
        if !dir.is_dir() { return Ok(vec![]); }
        let mut out: Vec<String> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        out.sort();
        Ok(out)
    }

    /// 解析车辆目录。顺序：参数 > $VEHICLE > .current-vehicle。绝不猜。
    pub fn resolve_vehicle(&self, arg: Option<&str>) -> Result<Vehicle> {
        let id = arg
            .map(|s| s.to_string())
            .or_else(|| std::env::var("VEHICLE").ok().filter(|s| !s.trim().is_empty()))
            .or_else(|| {
                std::fs::read_to_string(self.current_file())
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });

        let avail = self.list_vehicles()?;
        let id = id.ok_or_else(|| {
            anyhow!("未指定车辆。用 -v <id>，或写入 .current-vehicle\n可用: {:?}", avail)
        })?;

        // 车辆 id 必须是**单个目录名**，不能是路径。
        // 监工 2026-09-14-c 实测：Path::join 遇到绝对路径会丢弃前缀，
        // 于是 `-v /tmp/任意/EVIL` 配一个自制 VEHICLE.yml 就成了「合法车辆」——
        // 召回为空、硬红线为空，BLOCK 门禁随之全部解除。
        // `a/../b` 同样能穿越。
        if id.is_empty()
            || id.contains('/') || id.contains('\\')
            || id.starts_with('.')
            || id.contains("..")
        {
            return Err(anyhow!("车辆 id 必须是单个目录名，不能是路径：「{}」\n可用: {:?}",
                               id, avail));
        }
        // 只认枚举出来的那几台 —— 不靠 is_dir() 判断，那会把任何存在的目录放进来
        if !avail.iter().any(|x| x == &id) {
            return Err(anyhow!("没有这台车: {}\n可用: {:?}", id, avail));
        }
        let dir = self.vehicles_dir().join(&id);
        Ok(Vehicle { id, dir })
    }
}

pub struct Vehicle {
    pub id: String,
    pub dir: PathBuf,
}

impl Vehicle {
    pub fn yml(&self) -> PathBuf { self.dir.join("VEHICLE.yml") }
    pub fn manuals_dir(&self) -> PathBuf { self.dir.join("10_手册资料") }
    pub fn photos_dir(&self) -> PathBuf { self.dir.join("60_影像") }
    pub fn specs_dir(&self) -> PathBuf { self.dir.join("70_规格数据") }
    pub fn log_dir(&self) -> PathBuf { self.dir.join("40_维修日志") }
}

fn shellexpand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    s.to_string()
}

pub fn exists(p: &Path) -> bool { p.exists() }
