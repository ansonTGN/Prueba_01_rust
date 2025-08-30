// src/bin/3_summarizer.rs
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use multi_agent_file_processor::{
    connect_to_nats,
    mcp_protocol::{McpMessageTurn, McpRequest, McpResponse},
    setup_tracing, AgentResponse, ProcessFileRequest,
};
use std::time::Duration;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_tracing();

    let client = connect_to_nats().await?;
    info!("[Summarizer] Agente conectado a NATS.");
    let mut sub = client.subscribe("summary.request").await?;
    info!("[Summarizer] Escuchando en 'summary.request'.");

    // Prefijo del modelo permite forzar proveedor desde aquí:
    // openai:gpt-4o-mini | ollama:llama3.1:8b | groq:llama-3.1-70b-versatile
    let summarizer_model =
        std::env::var("SUMMARIZER_MODEL").unwrap_or_else(|_| "openai:gpt-4o-mini".to_string());
    let default_provider = std::env::var("LLM_PROVIDER").ok(); // "openai" | "ollama" | "groq" | "auto"

    while let Some(msg) = sub.next().await {
        let request: ProcessFileRequest = serde_json::from_slice(&msg.payload)?;
        if let Some(reply_to) = msg.reply {
            let client = client.clone();
            let model = summarizer_model.clone();
            let provider = default_provider.clone();

            tokio::spawn(async move {
                info!("[Summarizer] Procesando solicitud para '{}'", request.path);
                let response = match process_file(&client, request, model, provider).await {
                    Ok(summary) => AgentResponse::Success(summary),
                    Err(e) => {
                        error!("[Summarizer] Fallo en el procesamiento: {:?}", e);
                        AgentResponse::Error(e.to_string())
                    }
                };

                if let Ok(payload) = serde_json::to_vec(&response) {
                    client.publish(reply_to, payload.into()).await.ok();
                }
            });
        }
    }
    Ok(())
}

async fn process_file(
    client: &async_nats::Client,
    request: ProcessFileRequest,
    model: String,
    provider_env: Option<String>,
) -> Result<String> {
    let content = std::fs::read_to_string(&request.path)
        .context(format!("No se pudo leer el archivo: {}", request.path))?;

    let mcp_request = McpRequest {
        model,                    // puede llevar prefijo: openai:/ollama:/groq:
        provider: provider_env,   // None => decide Gateway
        messages: vec![
            McpMessageTurn {
                role: "system".to_string(),
                content: "Eres un experto en resumir textos de forma concisa.".to_string(),
            },
            McpMessageTurn { role: "user".to_string(), content },
        ],
        temperature: Some(0.7),
    };

    // Request/Reply manual con inbox propio + timeout largo (120 s)
    let inbox = client.new_inbox();
    let mut replies = client.subscribe(inbox.clone()).await?;
    client
        .publish_with_reply(
            "mcp.request.completion",
            inbox,
            serde_json::to_vec(&mcp_request)?.into(),
        )
        .await?;

    // timeout :: Result<Option<Message>, Elapsed>
    let maybe_msg = tokio::time::timeout(Duration::from_secs(120), replies.next())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout esperando respuesta del LLM Gateway (120s)."))?;
    let msg = maybe_msg
        .ok_or_else(|| anyhow::anyhow!("El LLM Gateway cerró la respuesta sin emitir mensaje"))?;

    let mcp_response: AgentResponse<McpResponse> =
        serde_json::from_slice(&msg.payload).context("Respuesta del Gateway malformada")?;

    match mcp_response {
        AgentResponse::Success(resp) => Ok(resp.content),
        AgentResponse::Error(e) => bail!("El LLM Gateway devolvió un error: {}", e),
    }
}


