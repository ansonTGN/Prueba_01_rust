// src/bin/2_metadata_extractor.rs
use anyhow::Result;
use futures_util::StreamExt;
use multi_agent_file_processor::{
    connect_to_nats, setup_tracing, AgentResponse, FileMetadata, FileType, ProcessFileRequest,
};
use std::fs;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_tracing();

    let client = connect_to_nats().await?;
    info!("[Metadata] Agente conectado a NATS.");
    let mut sub = client.subscribe("metadata.request").await?;
    info!("[Metadata] Escuchando en 'metadata.request'.");

    while let Some(msg) = sub.next().await {
        let request: ProcessFileRequest = serde_json::from_slice(&msg.payload)?;
        if let Some(reply) = msg.reply {
            let response = match fs::metadata(&request.path) {
                Ok(meta) => AgentResponse::Success(FileMetadata {
                    file_type: if meta.is_file() { FileType::File } else { FileType::Directory },
                    len_bytes: meta.len(),
                    created: meta.created().ok(),
                    modified: meta.modified().ok(),
                }),
                Err(e) => {
                    error!("[Metadata] Fallo al obtener metadatos para '{}': {}", request.path, e);
                    AgentResponse::Error(format!("Error al obtener metadatos: {}", e))
                }
            };
            client.publish(reply, serde_json::to_vec(&response)?.into()).await?;
        }
    }
    Ok(())
}