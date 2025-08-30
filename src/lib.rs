// src/lib.rs

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::SystemTime;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// Módulo para el protocolo de agentes externos
pub mod mcp_protocol;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FileType { File, Directory }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileDiscovered { pub name: String, pub path: String }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProcessFileRequest { pub path: String }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    pub file_type: FileType,
    pub len_bytes: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileListRequest;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileListResponse { pub files: Vec<FileDiscovered> }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum AgentResponse<T> { Success(T), Error(String) }

pub fn setup_tracing() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
}

pub async fn connect_to_nats() -> Result<async_nats::Client> {
    let nats_url = env::var("NATS_URL").context("La variable de entorno NATS_URL no está definida")?;
    let client = async_nats::connect(&nats_url)
        .await
        .context(format!("No se pudo conectar a NATS en {}", nats_url))?;
    Ok(client)
}