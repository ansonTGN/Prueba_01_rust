// src/bin/5_llm_gateway.rs
use anyhow::{Context, Result};
use futures_util::StreamExt;
use multi_agent_file_processor::{
    connect_to_nats,
    mcp_protocol::{McpRequest, McpResponse},
    setup_tracing, AgentResponse,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{error, info};

#[derive(Debug, Clone, Default)]
struct LlmConfigState {
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LlmConfigSet {
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    temperature: Option<f32>,
}

// -------- Provider inspection types ----------
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProviderReport {
    providers: Vec<ProviderInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProviderInfo {
    name: String,
    endpoint: Option<String>,
    reachable: bool,
    latency_ms: Option<u128>,
    auth_mode: Option<String>,
    error: Option<String>,
    models: Vec<ModelInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ModelInfo {
    id: String,
    family: Option<String>,
    modality: Option<String>,       // "text" | "multimodal"
    context_length: Option<u32>,
    supports_json: Option<bool>,
    supports_tools: Option<bool>,
    supports_images: Option<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_tracing();

    let client = connect_to_nats().await?;
    info!("[LLM Gateway] Conectado a NATS.");

    let mut sub = client.subscribe("mcp.request.completion").await?;
    let mut ping_sub = client.subscribe("llm.ping").await?;
    let mut cfg_sub = client.subscribe("llm.config.set").await?;
    let mut models_sub = client.subscribe("llm.models.list").await?;
    let mut inspect_sub = client.subscribe("llm.providers.inspect").await?;
    info!("[LLM Gateway] Escuchando en 'mcp.request.completion'.");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut state = LlmConfigState::default();

    loop {
        tokio::select! {
            Some(msg) = sub.next() => {
                let req: McpRequest = match serde_json::from_slice(&msg.payload) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("[LLM Gateway] Solicitud MCP malformada: {}", e);
                        continue;
                    }
                };
                let rply = msg.reply.clone();
                let http = http.clone();
                let state_snapshot = state.clone();
                let client2 = client.clone();

                tokio::spawn(async move {
                    let resp = match handle_mcp(req, &http, &state_snapshot).await {
                        Ok(m) => AgentResponse::Success(m),
                        Err(e) => {
                            error!("[LLM Gateway] Error LLM: {}", e);
                            AgentResponse::Error(e.to_string())
                        }
                    };
                    if let Some(r) = rply {
                        if let Ok(payload) = serde_json::to_vec(&resp) {
                            let _ = client2.publish(r, payload.into()).await;
                        }
                    }
                });
            }
            Some(msg) = ping_sub.next() => {
                if let Some(r) = msg.reply {
                    let _ = client.publish(r, "pong".into()).await;
                }
            }
            Some(msg) = cfg_sub.next() => {
                match serde_json::from_slice::<LlmConfigSet>(&msg.payload) {
                    Ok(cfg) => {
                        state.provider = cfg.provider.or(state.provider);
                        state.model = cfg.model.or(state.model);
                        state.base_url = cfg.base_url.or(state.base_url);
                        state.api_key = cfg.api_key.or(state.api_key);
                        state.temperature = cfg.temperature.or(state.temperature);
                        info!("[LLM Gateway] Config LLM actualizada: {:?}", state);
                    }
                    Err(e) => error!("[LLM Gateway] Config inválida: {}", e),
                }
            }
            Some(msg) = models_sub.next() => {
                let rply = msg.reply.clone();
                let http = http.clone();
                let state_snapshot = state.clone();
                let client2 = client.clone();

                tokio::spawn(async move {
                    let resp: AgentResponse<Vec<String>> = match list_models(&http, &state_snapshot).await {
                        Ok(list) => AgentResponse::Success(list),
                        Err(e) => AgentResponse::Error(e.to_string()),
                    };
                    if let Some(r) = rply {
                        if let Ok(payload) = serde_json::to_vec(&resp) {
                            let _ = client2.publish(r, payload.into()).await;
                        }
                    }
                });
            }
            Some(msg) = inspect_sub.next() => {
                let rply = msg.reply.clone();
                let http = http.clone();
                let state_snapshot = state.clone();
                let client2 = client.clone();

                tokio::spawn(async move {
                    let resp: AgentResponse<ProviderReport> = match inspect_providers(&http, &state_snapshot).await {
                        Ok(rep) => AgentResponse::Success(rep),
                        Err(e) => AgentResponse::Error(e.to_string()),
                    };
                    if let Some(r) = rply {
                        if let Ok(payload) = serde_json::to_vec(&resp) {
                            let _ = client2.publish(r, payload.into()).await;
                        }
                    }
                });
            }
            else => break,
        }
    }

    Ok(())
}

// ------------------------ MCP handler (OpenAI/Groq/Ollama) ----------------
async fn handle_mcp(req: McpRequest, http: &reqwest::Client, state: &LlmConfigState) -> Result<McpResponse> {
    let provider = state.provider.clone().unwrap_or_else(|| "openai".to_string());
    let model = req.model;
    let temp = req.temperature.or(state.temperature).unwrap_or(0.7);

    match provider.as_str() {
        "openai" | "groq" => {
            let (base, key_header) = if provider == "openai" {
                ("https://api.openai.com", "OPENAI_API_KEY")
            } else {
                ("https://api.groq.com", "GROQ_API_KEY")
            };
            let api_key = state.api_key.clone().or_else(|| std::env::var(key_header).ok())
                .context(format!("{} no definido", key_header))?;

            // Arregla E0716: construye el URL en ramas separadas
            let url = if provider == "openai" {
                format!("{}/v1/chat/completions", base)
            } else {
                format!("{}/openai/v1/chat/completions", base)
            };

            let payload = serde_json::json!({
                "model": model,
                "temperature": temp,
                "messages": req.messages.iter().map(|m| {
                    serde_json::json!({"role": m.role, "content": m.content})
                }).collect::<Vec<_>>()
            });

            let resp = http.post(&url)
                .bearer_auth(api_key)
                .json(&payload)
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                anyhow::bail!("OpenAI/Groq devolvió {}: {}", status, txt);
            }
            #[derive(Deserialize)]
            struct ChoiceMsg { content: String }
            #[derive(Deserialize)]
            struct Choice { message: ChoiceMsg }
            #[derive(Deserialize)]
            struct ChatResp { choices: Vec<Choice> }
            let jr: ChatResp = resp.json().await?;
            let content = jr.choices.get(0).map(|c| c.message.content.clone()).unwrap_or_default();
            Ok(McpResponse { content, token_usage: None })
        }
        "ollama" => {
            let base = state.base_url.clone().unwrap_or_else(|| "http://localhost:11434".to_string());
            let url = format!("{}/api/chat", base);
            let messages: Vec<serde_json::Value> = req.messages.iter().map(|m| {
                serde_json::json!({"role": m.role, "content": m.content})
            }).collect();
            let payload = serde_json::json!({
                "model": model,
                "stream": false,
                "options": { "temperature": temp },
                "messages": messages
            });

            let resp = http.post(&url).json(&payload).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                anyhow::bail!("Ollama devolvió {}: {}", status, txt);
            }
            #[derive(Deserialize)]
            struct Msg { content: String }
            #[derive(Deserialize)]
            struct OllamaResp { message: Msg }
            let jr: OllamaResp = resp.json().await?;
            Ok(McpResponse { content: jr.message.content, token_usage: None })
        }
        other => anyhow::bail!("Proveedor no soportado: {}", other),
    }
}

// ------------------------ List models (del proveedor activo) --------------
async fn list_models(http: &reqwest::Client, state: &LlmConfigState) -> Result<Vec<String>> {
    let provider = state.provider.clone().unwrap_or_else(|| "openai".to_string());
    match provider.as_str() {
        "openai" | "groq" => {
            let (base, key_header) = if provider == "openai" {
                ("https://api.openai.com", "OPENAI_API_KEY")
            } else {
                ("https://api.groq.com/openai", "GROQ_API_KEY")
            };
            let api_key = state.api_key.clone().or_else(|| std::env::var(key_header).ok())
                .context(format!("{} no definido", key_header))?;
            let url = format!("{}/v1/models", base);
            let resp = http.get(&url).bearer_auth(api_key).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                anyhow::bail!("{} /models devolvió {}: {}", provider, status, txt);
            }
            #[derive(Deserialize)]
            struct Model { id: String }
            #[derive(Deserialize)]
            struct List { data: Vec<Model> }
            let list: List = resp.json().await?;
            Ok(list.data.into_iter().map(|m| m.id).collect())
        }
        "ollama" => {
            let base = state.base_url.clone().unwrap_or_else(|| "http://localhost:11434".to_string());
            let url = format!("{}/api/tags", base);
            let resp = http.get(&url).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                anyhow::bail!("ollama /api/tags devolvió {}: {}", status, txt);
            }
            #[derive(Deserialize)]
            struct Tag { name: String }
            #[derive(Deserialize)]
            struct Tags { models: Vec<Tag> }
            let tags: Tags = resp.json().await?;
            Ok(tags.models.into_iter().map(|t| t.name).collect())
        }
        other => anyhow::bail!("Proveedor no soportado: {}", other),
    }
}

// ------------------------ Inspect providers (nuevo) -----------------------
async fn inspect_providers(http: &reqwest::Client, state: &LlmConfigState) -> Result<ProviderReport> {
    let mut providers = Vec::new();

    // OPENAI
    {
        let mut info = ProviderInfo {
            name: "openai".into(),
            endpoint: Some("https://api.openai.com".into()),
            reachable: false,
            latency_ms: None,
            auth_mode: Some("bearer".into()),
            error: None,
            models: vec![],
        };
        let key = state.api_key.clone().or_else(|| std::env::var("OPENAI_API_KEY").ok());
        if key.is_none() {
            info.error = Some("OPENAI_API_KEY no definido".into());
        } else {
            let start = Instant::now();
            let res = http
                .get("https://api.openai.com/v1/models")
                .bearer_auth(key.unwrap())
                .send()
                .await;
            match res {
                Ok(resp) if resp.status().is_success() => {
                    info.reachable = true;
                    info.latency_ms = Some(start.elapsed().as_millis());
                    #[derive(Deserialize)]
                    struct Model { id: String }
                    #[derive(Deserialize)]
                    struct List { data: Vec<Model> }
                    let list: List = resp.json().await.unwrap_or(List{data:vec![]});
                    info.models = list.data.into_iter().map(|m| ModelInfo{ id: m.id, ..Default::default() }).collect();
                }
                Ok(resp) => {
                    let status = resp.status();
                    let txt = resp.text().await.unwrap_or_default();
                    info.error = Some(format!("{} {}", status, txt));
                }
                Err(e) => info.error = Some(e.to_string()),
            }
        }
        providers.push(info);
    }

    // GROQ
    {
        let mut info = ProviderInfo {
            name: "groq".into(),
            endpoint: Some("https://api.groq.com/openai".into()),
            reachable: false,
            latency_ms: None,
            auth_mode: Some("bearer".into()),
            error: None,
            models: vec![],
        };
        let key = state.api_key.clone().or_else(|| std::env::var("GROQ_API_KEY").ok());
        if key.is_none() {
            info.error = Some("GROQ_API_KEY no definido".into());
        } else {
            let start = Instant::now();
            let res = http
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(key.unwrap())
                .send()
                .await;
            match res {
                Ok(resp) if resp.status().is_success() => {
                    info.reachable = true;
                    info.latency_ms = Some(start.elapsed().as_millis());
                    #[derive(Deserialize)]
                    struct Model { id: String }
                    #[derive(Deserialize)]
                    struct List { data: Vec<Model> }
                    let list: List = resp.json().await.unwrap_or(List{data:vec![]});
                    info.models = list.data.into_iter().map(|m| ModelInfo{ id: m.id, ..Default::default() }).collect();
                }
                Ok(resp) => {
                    let status = resp.status();
                    let txt = resp.text().await.unwrap_or_default();
                    info.error = Some(format!("{} {}", status, txt));
                }
                Err(e) => info.error = Some(e.to_string()),
            }
        }
        providers.push(info);
    }

    // OLLAMA
    {
        let base = state.base_url.clone().or_else(|| std::env::var("OLLAMA_BASE_URL").ok())
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        let mut info = ProviderInfo {
            name: "ollama".into(),
            endpoint: Some(base.clone()),
            reachable: false,
            latency_ms: None,
            auth_mode: Some("none".into()),
            error: None,
            models: vec![],
        };
        let url = format!("{}/api/tags", base);
        let start = Instant::now();
        let res = http.get(&url).send().await;
        match res {
            Ok(resp) if resp.status().is_success() => {
                info.reachable = true;
                info.latency_ms = Some(start.elapsed().as_millis());
                #[derive(Deserialize)]
                struct Tag { name: String }
                #[derive(Deserialize)]
                struct Tags { models: Vec<Tag> }
                let tags: Tags = resp.json().await.unwrap_or(Tags{models:vec![]});
                info.models = tags.models.into_iter().map(|t| ModelInfo{ id: t.name, ..Default::default() }).collect();
            }
            Ok(resp) => {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                info.error = Some(format!("{} {}", status, txt));
            }
            Err(e) => info.error = Some(e.to_string()),
        }
        providers.push(info);
    }

    Ok(ProviderReport { providers })
}


