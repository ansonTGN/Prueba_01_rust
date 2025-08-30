// src/mcp_protocol.rs

use serde::{Deserialize, Serialize};

/// Un único turno en la conversación con el LLM.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpMessageTurn {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
}

/// La solicitud completa que un agente envía al LLM Gateway.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpRequest {
    /// El modelo a utilizar (puede llevar prefijo: "openai:...", "ollama:...", "groq:...")
    pub model: String,
    /// (Opcional) Forzar proveedor. Si None, el Gateway decide (o por prefijo del modelo).
    #[serde(default)]
    pub provider: Option<String>,
    /// Historial de mensajes que proporciona el contexto.
    pub messages: Vec<McpMessageTurn>,
    /// (Opcional) Parámetros de inferencia.
    #[serde(default)]
    pub temperature: Option<f32>,
}

/// La respuesta que el LLM Gateway devuelve al agente solicitante.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpResponse {
    /// El contenido generado por el modelo.
    pub content: String,
    /// (Opcional) Información sobre el uso de tokens.
    #[serde(default)]
    pub token_usage: Option<(u32, u32)>, // (prompt_tokens, completion_tokens)
}
