//! Minimal OpenAI-compatible streaming client (DeepSeek by default).
//!
//! Blocking on purpose: it runs on a dedicated worker thread, which keeps the
//! binary free of an async runtime.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::ai::{ChatDelta, ChatRequest};
use crate::settings::AppSettings;

fn dbg(msg: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("xuanci-debug.log"))
    {
        let _ = writeln!(f, "[ai] {msg}");
    }
}

pub fn stream_chat(
    settings: &AppSettings,
    api_key: &str,
    request: &ChatRequest,
    cancel: Arc<AtomicBool>,
    sink: &mut dyn FnMut(ChatDelta),
) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("尚未配置 API Key".into());
    }
    if !settings.endpoint_is_allowed() {
        return Err("Base URL 必须使用 HTTPS（本地测试地址除外）".into());
    }

    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "temperature": request.temperature,
    });
    if request.disable_thinking {
        body["thinking"] = json!({ "type": "disabled" });
    }

    let payload = serde_json::to_string(&body).map_err(|e| format!("请求序列化失败：{e}"))?;

    dbg(&format!(
        "request endpoint={} model={} messages={}",
        settings.endpoint(),
        request.model,
        request.messages.len()
    ));

    // native-tls is never selected implicitly by ureq; it must be configured.
    let config = ureq::config::Config::builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(Duration::from_secs(120)))
        .build();
    let agent = config.new_agent();

    let response = match agent
        .post(&settings.endpoint())
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .send(payload.as_str())
    {
        Ok(response) => {
            dbg(&format!("http status={}", response.status()));
            response
        }
        Err(error) => {
            dbg(&format!("http error: {error}"));
            return Err(map_ureq_error(error));
        }
    };

    let reader = response.into_body().into_reader();
    let mut lines = BufReader::new(reader).lines();
    let mut chunk_count = 0usize;

    while let Some(line) = lines.next() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let line = line.map_err(|e| format!("读取流失败：{e}"))?;
        let line = line.trim_end();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            dbg(&format!("stream done chunks={chunk_count}"));
            sink(ChatDelta::Done);
            return Ok(());
        }

        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(err) = value.get("error") {
            sink(ChatDelta::Error(err.to_string()));
            return Err(err.to_string());
        }
        let Some(delta) = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
        else {
            continue;
        };
        if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                sink(ChatDelta::Thinking(reasoning.to_string()));
            }
        }
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                chunk_count += 1;
                sink(ChatDelta::Content(content.to_string()));
            }
        }
    }

    dbg(&format!("stream eof chunks={chunk_count}"));
    sink(ChatDelta::Done);
    Ok(())
}

fn map_ureq_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(code) => match code {
            400 => "请求格式错误（400）".into(),
            401 => "API Key 无效或已过期（401）".into(),
            402 => "账户余额不足（402）".into(),
            403 => "无权访问该模型（403）".into(),
            404 => "接口地址或模型不存在（404）".into(),
            422 => "请求参数不合法（422），请检查模型名".into(),
            429 => "请求过于频繁，请稍后再试（429）".into(),
            500..=599 => format!("服务端错误（{code}），请稍后重试"),
            _ => format!("请求失败（HTTP {code}）"),
        },
        other => format!("网络请求失败：{other}"),
    }
}
