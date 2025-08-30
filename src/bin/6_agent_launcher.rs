// src/bin/6_agent_launcher.rs
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::signal;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Deserialize, Debug, Clone, PartialEq)]
enum RestartPolicy {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "on_failure")]
    OnFailure,
    #[serde(rename = "always")]
    Always,
}

#[derive(Deserialize, Debug, Clone)]
struct AgentConfig {
    name: String,
    bin: String,
    enabled: bool,
    restart: RestartPolicy,
}

#[derive(Deserialize, Debug)]
struct LauncherConfig {
    build_profile: String,
    agents: Vec<AgentConfig>,
}

struct ManagedAgent {
    config: AgentConfig,
    child: Arc<Mutex<Child>>,
    id: u32,
}

impl ManagedAgent {
    fn name(&self) -> &str {
        &self.config.name
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder().with_max_level(Level::INFO).finish();
    tracing::subscriber::set_global_default(subscriber)?;
    dotenvy::dotenv().ok();

    info!("Iniciando Agent Launcher...");
    let config_str = std::fs::read_to_string("config.toml")
        .context("No se pudo encontrar o leer 'config.toml'")?;
    let config: LauncherConfig = toml::from_str(&config_str)
        .context("Error al parsear 'config.toml'")?;

    info!("Compilando agentes en perfil '{}'...", config.build_profile);
    let build_status = Command::new("cargo")
        .arg("build")
        .args(if config.build_profile == "release" { vec!["--release"] } else { vec![] })
        .status()
        .await?;

    if !build_status.success() {
        anyhow::bail!("La compilación de los agentes ha fallado. Abortando.");
    }

    let bin_path = Path::new("target").join(&config.build_profile);
    let (tx, mut rx) = mpsc::channel::<(u32, AgentConfig)>(100);

    let mut agents = Vec::new();
    for agent_config in config.agents.into_iter().filter(|a| a.enabled) {
        let agent = spawn_agent(agent_config, &bin_path, tx.clone()).await?;
        agents.push(agent);
    }
    
    if agents.is_empty() {
        warn!("No hay agentes habilitados para ejecutar. Saliendo.");
        return Ok(());
    }

    info!("Todos los agentes habilitados han sido iniciados. Presione Ctrl+C para detenerlos.");

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Señal de apagado (Ctrl+C) recibida. Terminando todos los agentes...");
                break;
            },
            Some((id, config)) = rx.recv() => {
                agents.retain(|a| a.id != id);
                warn!("[Launcher] El agente '{}' (ID: {}) ha terminado.", config.name, id);
                
                if config.restart != RestartPolicy::Never {
                    info!("[Launcher] Aplicando política de reinicio '{:?}' para '{}'", config.restart, config.name);
                    let new_agent = spawn_agent(config, &bin_path, tx.clone()).await?;
                    agents.push(new_agent);
                }

                if agents.is_empty() {
                    info!("Todos los agentes gestionados han terminado. Saliendo.");
                    break;
                }
            }
        }
    }

    // Apagado: matar procesos aún vivos
    for agent in &mut agents {
        info!("[Launcher] Deteniendo al agente '{}'...", agent.name());
        let mut ch = agent.child.lock().await;
        if let Err(e) = ch.kill().await {
            error!("[Launcher] No se pudo detener al agente '{}': {}", agent.name(), e);
        }
    }
    info!("Agent Launcher finalizado.");
    Ok(())
}

async fn spawn_agent(
    config: AgentConfig,
    bin_path: &PathBuf,
    tx: mpsc::Sender<(u32, AgentConfig)>,
) -> Result<ManagedAgent> {
    let agent_path = bin_path.join(&config.bin);
    let mut command = Command::new(&agent_path);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    // Spawn del proceso hijo
    let mut child = command.spawn().context(format!(
        "Fallo al iniciar el binario '{}' en la ruta {:?}",
        config.bin, agent_path
    ))?;

    // Tomamos stdout/stderr ANTES de envolver el Child
    let stdout = child.stdout.take().expect("stdout no fue capturado");
    let stderr = child.stderr.take().expect("stderr no fue capturado");

    let id = child.id().expect("El proceso hijo debe tener un ID");
    info!(
        "[Launcher] Agente '{}' iniciado con éxito. PID: {}.",
        config.name, id
    );

    // Log de stdout
    let stdout_name = config.name.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            info!("[{}] {}", stdout_name, line);
        }
    });

    // Log de stderr
    let stderr_name = config.name.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            error!("[{}] {}", stderr_name, line);
        }
    });

    // Envolver Child para compartirlo: monitor + manejador
    let child_arc = Arc::new(Mutex::new(child));

    // Monitor de salida del proceso: hará wait() y notificará
    let monitor_config = config.clone();
    let child_for_monitor = Arc::clone(&child_arc);
    tokio::spawn(async move {
        // Espera a que el proceso termine
        {
            let mut ch = child_for_monitor.lock().await;
            let _ = ch.wait().await;
        }
        if tx.send((id, monitor_config)).await.is_err() {
            error!("[Launcher] El canal de comunicación del lanzador está cerrado.");
        }
    });
    
    Ok(ManagedAgent {
        config,
        child: child_arc,
        id,
    })
}
