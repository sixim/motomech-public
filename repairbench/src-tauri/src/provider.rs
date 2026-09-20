//! 模型接入 —— 双通道
//!
//! A) `openai`     OpenAI 兼容 HTTP。主力，指向 LM Studio / OpenRouter / 任何兼容端点。
//! B) `hermes_cli` Hermes CLI 子进程。**用上 Claude Max / ChatGPT 订阅的唯一路径**
//!                 （已核实：`hermes proxy` 只支持 nous + xai，不代理 Claude/ChatGPT）。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Answer {
    pub provider: String,
    pub model: String,
    pub text: String,
    /// 推理模型的思考过程。content 为空时这里往往有内容 —— 不丢弃它。
    pub reasoning: Option<String>,
    pub elapsed_ms: u128,
    pub warnings: Vec<String>,
}

pub fn load(platform_root: &Path) -> Result<(Vec<Provider>, String)> {
    let p = platform_root.join("providers.yml");
    let doc: Value = serde_yaml_ng::from_str(&std::fs::read_to_string(&p)
        .with_context(|| format!("读不到 {}", p.display()))?)?;
    let list: Vec<Provider> = serde_yaml_ng::from_value(
        doc.get("providers").cloned().unwrap_or(Value::Sequence(vec![])))?;
    let default = doc.get("default").and_then(|x| x.as_str())
        .unwrap_or("local").to_string();
    Ok((list, default))
}

pub fn pick(list: &[Provider], id: Option<&str>, default: &str) -> Result<Provider> {
    let want = id.unwrap_or(default);
    list.iter().find(|p| p.id == want).cloned()
        .ok_or_else(|| anyhow!("没有这个 provider: {want}\n可用: {:?}",
                               list.iter().map(|p| &p.id).collect::<Vec<_>>()))
}

pub fn ask(p: &Provider, system: &str, user: &str) -> Result<Answer> {
    let t0 = std::time::Instant::now();
    match p.kind.as_str() {
        "openai"     => ask_openai(p, system, user, t0),
        "hermes_cli" => ask_hermes(p, system, user, t0),
        other => Err(anyhow!("未知 provider kind: {other}")),
    }
}

fn ask_openai(p: &Provider, system: &str, user: &str,
              t0: std::time::Instant) -> Result<Answer> {
    let base = p.base_url.clone()
        .ok_or_else(|| anyhow!("provider {} 缺 base_url", p.id))?;
    let model = p.model.clone()
        .ok_or_else(|| anyhow!("provider {} 缺 model", p.id))?;
    let mut warnings = Vec::new();

    let mut req = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?
        .post(format!("{}/chat/completions", base.trim_end_matches('/')));

    if let Some(env_name) = &p.api_key_env {
        match std::env::var(env_name) {
            Ok(k) => req = req.bearer_auth(k),
            Err(_) => warnings.push(format!("环境变量 {env_name} 未设置，未带认证")),
        }
    }

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user",   "content": user}
        ],
        "max_tokens": p.max_tokens.unwrap_or(2000),
        "temperature": 0.2,
    });

    let resp = req.json(&body).send()
        .with_context(|| format!("连不上 {base} —— LM Studio 开着吗？"))?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().context("响应不是 JSON")?;
    if !status.is_success() {
        return Err(anyhow!("HTTP {status}: {}", v.to_string().chars().take(300).collect::<String>()));
    }

    let msg = &v["choices"][0]["message"];
    let text = msg["content"].as_str().unwrap_or("").trim().to_string();
    // 推理模型把 token 先花在思考上。给少了 max_tokens，content 会是空的 ——
    // 实测 Qwen3.8 在 400 tokens 时思考占 239，正文一个字都没输出。
    let reasoning = msg["reasoning_content"].as_str()
        .map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if text.is_empty() && reasoning.is_some() {
        warnings.push("正文为空、只有思考内容 —— max_tokens 可能不够，思考把配额吃光了".into());
    }
    if let Some(rt) = v["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64() {
        if rt > 0 { warnings.push(format!("推理模型：{rt} tokens 花在思考上")); }
    }

    Ok(Answer { provider: p.id.clone(), model, text, reasoning,
                elapsed_ms: t0.elapsed().as_millis(), warnings })
}

/// Hermes CLI 一次性调用。`hermes -z` 专为脚本设计：单 prompt 进、纯文本出。
fn ask_hermes(p: &Provider, system: &str, user: &str,
              t0: std::time::Instant) -> Result<Answer> {
    // -z 只接受一个 prompt，没有独立的 system 位，拼进去
    let prompt = format!("{system}\n\n---\n\n{user}");
    let mut cmd = Command::new("hermes");
    cmd.arg("-z").arg(&prompt);
    if let Some(m) = &p.model { cmd.arg("--model").arg(m); }

    let out = cmd.output().context(
        "跑不了 hermes —— 装了吗？(~/.local/bin/hermes)")?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();

    let mut warnings = vec![
        "hermes -z：每次起进程、无流式、参数控制有限".to_string()
    ];
    if !out.status.success() {
        return Err(anyhow!("hermes 退出码 {:?}\n{}", out.status.code(),
                           err.chars().take(400).collect::<String>()));
    }
    if !err.is_empty() { warnings.push(format!("stderr: {}", err.chars().take(200).collect::<String>())); }

    Ok(Answer {
        provider: p.id.clone(),
        model: p.model.clone().unwrap_or_else(|| "(hermes 自身配置)".into()),
        text, reasoning: None,
        elapsed_ms: t0.elapsed().as_millis(), warnings,
    })
}


// ── 接入体检 ─────────────────────────────────────────────────
//
// 为什么要真探活而不是列个清单：2026-09-16 实测撞到一个**自相矛盾的绿灯** ——
// `hermes auth status anthropic` 报「logged in」，而真正调用推理时报
// 「No Anthropic credentials found」。两个子系统对同一件事给出相反答案。
//
// 根因不是配置错，是订阅制的本质：OAuth 会话绑在交互式 agent 上，
// **它不是一把可编程调用的 API key**。所以「登录了」与「能用来跑推理」是两回事。
// 清单式的设置页会把前者显示成可用 —— 那就是假绿灯。这里一律实际打一次。

#[derive(Debug, serde::Serialize)]
pub struct Health {
    pub id: String,
    pub kind: String,
    pub model: String,
    /// ok / down / unauthorized / misconfigured
    pub state: String,
    pub detail: String,
    pub ms: u128,
    /// 这条通道要不要花钱：free-local / subscription / pay-per-token
    pub cost: String,
}

fn cost_of(p: &Provider) -> &'static str {
    if p.kind == "hermes_cli" { return "订阅制"; }
    match p.base_url.as_deref() {
        Some(u) if u.contains("localhost") || u.contains("127.0.0.1") => "本地免费",
        Some(_) => "按量付费",
        None => "未知",
    }
}

/// 逐个 provider 实际探活。**不猜、不读配置了事。**
pub fn health(list: &[Provider]) -> Vec<Health> {
    list.iter().map(|p| {
        let t0 = std::time::Instant::now();
        let (state, detail) = probe(p);
        Health {
            id: p.id.clone(), kind: p.kind.clone(),
            model: p.model.clone().unwrap_or_else(|| "(未指定)".into()), state, detail,
            ms: t0.elapsed().as_millis(), cost: cost_of(p).to_string(),
        }
    }).collect()
}

fn probe(p: &Provider) -> (String, String) {
    match p.kind.as_str() {
        "openai" => {
            let base = match p.base_url.as_deref() {
                Some(b) => b, None => return ("misconfigured".into(), "没有 base_url".into()),
            };
            // 需要密钥的端点先看密钥在不在 —— 缺了就别浪费一次请求
            if let Some(env) = p.api_key_env.as_deref() {
                if std::env::var(env).map(|v| v.trim().is_empty()).unwrap_or(true) {
                    return ("unauthorized".into(),
                        format!("环境变量 {env} 没设 —— 按项目规矩走钥匙串注入，不要写进文件"));
                }
            }
            let url = format!("{}/models", base.trim_end_matches('/'));
            let c = match reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(6)).build() {
                Ok(c) => c, Err(e) => return ("down".into(), format!("{e}")),
            };
            let mut req = c.get(&url);
            if let Some(env) = p.api_key_env.as_deref() {
                if let Ok(k) = std::env::var(env) { req = req.bearer_auth(k); }
            }
            match req.send() {
                Ok(r) if r.status().is_success() => {
                    let n = r.json::<serde_json::Value>().ok()
                        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|a| a.len()))
                        .unwrap_or(0);
                    // 端点活着不等于**这个 model 名**在上面
                    ("ok".into(), format!("端点在线，{n} 个模型可选"))
                }
                Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 =>
                    ("unauthorized".into(), format!("HTTP {} —— 密钥无效或没权限", r.status())),
                Ok(r) => ("down".into(), format!("HTTP {}", r.status())),
                Err(e) => ("down".into(), format!("连不上：{}",
                    e.to_string().chars().take(70).collect::<String>())),
            }
        }
        "hermes_cli" => {
            let out = std::process::Command::new("hermes")
                .arg("-z").arg("ping")
                .output();
            match out {
                Err(_) => ("down".into(), "PATH 里没有 hermes".into()),
                Ok(o) => {
                    let txt = format!("{}{}",
                        String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                    let low = txt.to_lowercase();
                    if low.contains("no anthropic credentials") || low.contains("credentials") {
                        ("unauthorized".into(), concat!(
                         "hermes 说没有可用凭据。注意 `hermes auth status` 可能同时报「logged in」——",
                         "那是**交互式 agent 的 OAuth 会话**，不是可编程调用的 API key。",
                         "订阅制模型走不了这条路，这不是配置错。").into())
                    } else if low.contains("context window") || low.contains("below the minimum") {
                        ("misconfigured".into(), concat!(
                         "当前默认模型上下文不足 64K（hermes 的硬下限）。",
                         "改 ~/.hermes/config.yaml 的 model.context_length，或指定一个长上下文模型。").into())
                    } else if o.status.success() {
                        ("ok".into(), "hermes -z 可调用".into())
                    } else {
                        ("down".into(), txt.lines().last().unwrap_or("调用失败")
                            .chars().take(90).collect::<String>())
                    }
                }
            }
        }
        other => ("misconfigured".into(), format!("未知 kind: {other}")),
    }
}

pub fn render_health(list: &[Health], default_id: &str) -> String {
    let mut o = String::from("模型接入体检（逐条实际探活，不是读配置）\n");
    o.push_str(&"─".repeat(64));
    o.push('\n');
    for h in list {
        let mark = match h.state.as_str() {
            "ok" => "✅", "unauthorized" => "🔑", "misconfigured" => "⚙️ ", _ => "❌",
        };
        let d = if h.id == default_id { "  ← 默认" } else { "" };
        o.push_str(&format!("{mark} {:<12} [{}] {:<34} {}ms{}\n",
            h.id, h.cost, h.model, h.ms, d));
        o.push_str(&format!("     {}\n", h.detail));
    }
    o.push_str("\n用 `repairbench providers --set <id>` 换默认通道。\n");
    o.push_str(concat!(
        "\nℹ️ 这套东西**不需要尖端模型**：权威数值一律逐字来自规格库，\n",
        "   模型只负责组织语言，且被明确禁止引入 facts 之外的数字。\n",
        "   所以本地小模型足够，付费大模型不会让答案更准 —— 只会更贵。\n"));
    o
}

/// 改默认通道 —— 只动 providers.yml 的 `default:` 一行。
pub fn set_default(platform_root: &Path, id: &str) -> Result<()> {
    let (list, _) = load(platform_root)?;
    if !list.iter().any(|p| p.id == id) {
        return Err(anyhow!("没有这个 provider：{id}。已注册的：{}",
            list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(" ")));
    }
    let f = platform_root.join("providers.yml");
    let txt = std::fs::read_to_string(&f)?;
    let mut out = String::new();
    let mut done = false;
    for line in txt.lines() {
        if !done && line.trim_start().starts_with("default:") {
            out.push_str(&format!("default: {id}\n")); done = true;
        } else { out.push_str(line); out.push('\n'); }
    }
    if !done { out.insert_str(0, &format!("default: {id}\n")); }
    std::fs::write(&f, out)?;
    Ok(())
}
