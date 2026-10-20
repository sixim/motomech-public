//! CLI 定义 —— 一个二进制四种模式（形态对齐 somadisk）
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "motomech",
    version,
    about = "维修助理 —— 手册知识库 · 诊断 · 作业单 · MCP/HTTP 接口",
    long_about = "不带子命令时启动桌面 App。\n\n\
                  数据真相源是文件（~/Ai Developer/AI+摩托维修助理/），\
                  SQLite 只作可重建索引。"
)]
pub struct Cli {
    /// 平台根目录（数据真相源）
    #[arg(long, env = "MOTOMECH_ROOT", global = true)]
    pub root: Option<String>,

    /// 车辆 id；不给则读平台根的 .current-vehicle
    #[arg(short = 'v', long, env = "VEHICLE", global = true)]
    pub vehicle: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动 HTTP API（headless，给 soma 及其他 agent 调用）
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },

    /// 作为 stdio MCP server 运行（给 Claude Code / Hermes 调用）
    Mcp,

    /// 扫描平台目录、重建索引（索引可删，删了重跑即可）
    Index {
        /// 重建**所有**车辆的索引。不加此参数只处理当前一台，
        /// 「删库重建」对多车场景必须用它
        #[arg(long)]
        all: bool,
    },

    /// 手册全文检索（默认排除不适用本车的资料）
    Search {
        /// 检索词
        query: String,
        /// 连不适用本车的资料一并检索（危险，仅用于排查）
        #[arg(long)]
        include_inapplicable: bool,
        /// 连 C 级资料（社区转录/未交叉验证）一并返回。看到了也只能当线索
        #[arg(long)]
        include_low_tier: bool,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },

    /// 安全门禁判定：对一个打算做的动作下判决（BLOCK / GATE / WARN / OK）
    Safety {
        /// 你打算做什么，用自然语言写，例如「打火试试能不能着」
        #[arg(default_value = "")]
        action: String,
        /// 跑回归用例，验证门禁既不漏拦也不误报
        #[arg(long)]
        selftest: bool,
    },

    /// 取某阶段的「开工前必读」（不给阶段则取当前阶段）
    Briefing {
        #[arg(default_value = "")]
        phase: String,
    },

    /// 采购清单，按到货周期倒排 —— 回答「今天必须下单的是哪几样」
    Parts {
        /// 每阶段预计天数（假设值，影响倒排）
        #[arg(long, default_value_t = 7)]
        days_per_phase: i64,
    },

    /// 外部工具：列出或调用（NHTSA 查询、RealOEM 链接等）
    Tool {
        /// 工具 id；省略则列出全部
        #[arg(default_value = "")]
        id: String,
        /// 参数，形如 vin=WB10... make=bmw
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// 列出本地脚本（照片/OCR/视觉/声学流水线）—— 只列出，不代为执行
    Scripts,

    /// 查规格（只返回 A/B 级且已核实，每条带来源级别与页码）
    Spec {
        /// 规格名关键词，如 valve、spark、torque、oil
        query: String,
    },

    /// 资料获取计划：这台车该找哪本手册、去哪找。**只定位，不下载**
    Acquire,

    /// Vlog 脚本体检：把「说不出出处的技术数字」摆出来（E6）
    Script,

    /// 电路图：本车哪个系统看哪一页。**默认只出适用本车年款的**
    Circuit {
        /// 按系统/车系/页号过滤，如 ABS、仪表、灯光、267
        #[arg(default_value = "")]
        query: String,
        /// 连不适用本车的也列出来（别的车系或别的年款）
        #[arg(long)]
        all: bool,
    },

    /// 交叉验证：安全关键项是否有两个**独立**来源（G4）。单来源只披露不扣值
    Crossval,

    /// 差距清单：系统自己知道「还不知道什么」，按后果排序
    Gaps,

    /// 诊断排查树：症状 → 测量点 → 判据 → 原因。不带 id 则列出所有树
    Diag {
        #[arg(default_value = "")]
        tree: String,
    },

    /// 症状 → 原因:按本车真实故障频率排出候选,再导向排查树
    Symptom {
        /// 观察到的现象,如「后轮异响」「燃油表乱跳」
        text: String,
    },


    /// 网络检索纪律层：出提纲、收线索（一律 C 级，不可用于作业）
    Research {
        #[command(subcommand)]
        cmd: ResearchCmd,
    },

    /// 新车冷启动：给一个 VIN，自动问出身份/召回/投诉并生成档案骨架
    Coldstart {
        /// 17 位 VIN。省略则用 -v 指定车辆档案里的 make/model/year 做研究刷新
        #[arg(long, default_value = "")]
        vin: String,
        /// 直接建车辆目录（不加则只写提案到 .proposals/）
        #[arg(long)]
        apply: bool,
    },

    /// 列出车辆及其关键标识
    Vehicles,

    /// 环境自检：路径、数据库、外部依赖
    Doctor,

    /// 列出可用的模型 provider
    Providers {
        /// 把默认通道换成这个 id
        #[arg(long)]
        set: Option<String>,
        /// 只列出，不逐条探活（探活会真的打请求）
        #[arg(long)]
        no_probe: bool,
    },

    /// 向模型提问（带本车上下文与准确性铁律）
    Ask {
        /// 问题
        question: String,
        /// provider id，默认读 providers.yml 的 default
        #[arg(long, short = 'P')]
        provider: Option<String>,
        /// 同时显示注入的系统提示
        #[arg(long)]
        show_prompt: bool,
    },

    /// 打印工作台数据（GUI 的同一份数据，便于无界面验证）
    Workbench {
        /// 指定阶段，默认读 PROGRESS.txt 或 P1
        #[arg(long)]
        phase: Option<String>,
        /// 输出 JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum ResearchCmd {
    /// 把一条差距变成检索提纲：要什么、什么算数、什么不算
    Brief {
        /// 差距 id，见 `motomech gaps`
        gap_id: String,
    },
    /// 收一条线索 —— **必须带 URL 与原文引用**
    Add {
        #[arg(long)] gap_id: String,
        #[arg(long)] url: String,
        #[arg(long)] quote: String,
        #[arg(long, default_value = "")] note: String,
        /// 幂等键，重复检索不会写出重复线索
        #[arg(long)] key: String,
    },
    /// 列出已收线索
    List,
}
