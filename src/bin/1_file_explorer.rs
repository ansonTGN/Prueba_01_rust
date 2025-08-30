// src/bin/1_file_explorer.rs
use anyhow::{Context, Result};
use futures_util::StreamExt;
use multi_agent_file_processor::{
    connect_to_nats, setup_tracing, AgentResponse, FileDiscovered, FileListRequest,
    FileListResponse, ProcessFileRequest,
};
use std::env;
use std::fs;
use std::path::Path;
use tracing::{error, info, instrument};

#[instrument(skip(dir_path))]
fn scan_directory(dir_path: &str) -> Result<Vec<FileDiscovered>> {
    info!("[Explorer] Escaneando directorio '{}'...", dir_path);
    let discovered_files = fs::read_dir(dir_path)?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .map(|entry| FileDiscovered {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().to_string_lossy().to_string(),
        })
        .collect::<Vec<_>>();
    info!("[Explorer] Se encontraron {} archivos.", discovered_files.len());
    Ok(discovered_files)
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    setup_tracing();

    let client = connect_to_nats().await?;
    info!("[Explorer] Agente conectado a NATS.");
    let dir_to_scan = env::var("DIRECTORY_TO_SCAN").context("DIRECTORY_TO_SCAN no está definida")?;

    let mut list_sub = client.subscribe("files.list.request").await?;
    let mut content_sub = client.subscribe("file.request.content").await?;

    info!("[Explorer] Escuchando en 'files.list.request' y 'file.request.content'");

    loop {
        tokio::select! {
            Some(msg) = list_sub.next() => {
                let _req: FileListRequest = serde_json::from_slice(&msg.payload)?;
                let response = match scan_directory(&dir_to_scan) {
                    Ok(files) => AgentResponse::Success(FileListResponse { files }),
                    Err(e) => {
                        error!("[Explorer] Error al escanear directorio: {}", e);
                        AgentResponse::Error(format!("Error del explorador al escanear: {}", e))
                    }
                };
                if let Some(reply) = msg.reply { client.publish(reply, serde_json::to_vec(&response)?.into()).await?; }
            }
            Some(msg) = content_sub.next() => {
                let request: ProcessFileRequest = serde_json::from_slice(&msg.payload)?;
                let response = match fs::read_to_string(Path::new(&request.path)) {
                    Ok(content) => AgentResponse::Success(content),
                    Err(e) => {
                        error!("[Explorer] Error al leer archivo '{}': {}", &request.path, e);
                        AgentResponse::Error(format!("No se pudo leer '{}': {}", &request.path, e))
                    }
                };
                if let Some(reply) = msg.reply { client.publish(reply, serde_json::to_vec(&response)?.into()).await?; }
            }
        }
    }
}