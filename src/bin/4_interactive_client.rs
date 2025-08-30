// src/bin/4_interactive_client.rs

use anyhow::{Context as AnyhowContext, Result};
use async_nats::Client as NatsClient;
use eframe::{egui, egui::Context as EguiContext};
use egui::{Color32, RichText, TextStyle, Ui};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant, SystemTime},
};

/// Eventos que envían las tareas async hacia la GUI.
#[derive(Debug)]
enum GuiEvent {
    Status(String),
    Error(String),
    PingMs(u128),
    Models(Vec<String>),
    ProviderReport(Value),
    Metadata(String),
    Summary(String),
}

/// Nodo del explorador de archivos (para el árbol opcional).
#[derive(Clone, Debug)]
struct DirNode {
    name: String,
    path: PathBuf,
    is_dir: bool,
    children: Option<Vec<DirNode>>,
}

impl DirNode {
    fn new(path: PathBuf) -> Self {
        let is_dir = path.is_dir();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        DirNode {
            name,
            path,
            is_dir,
            children: None,
        }
    }

    fn ensure_children_loaded(&mut self) {
        if !self.is_dir || self.children.is_some() {
            return;
        }
        let mut items = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&self.path) {
            for entry in read_dir.flatten() {
                items.push(DirNode::new(entry.path()));
            }
            items.sort_by(|a, b| {
                match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                }
            });
        }
        self.children = Some(items);
    }
}

/// Entrada mostrada en el listado de contenidos de un directorio.
#[derive(Clone, Debug)]
struct EntryView {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: Option<u64>,
    kind: String, // "Carpeta" o extensión
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortBy {
    Name,
    Kind,
    Size,
}

/// Configuración del LLM.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LlmConfig {
    provider: String,   // "openai" | "groq" | "ollama"
    base_url: String,   // https://api.openai.com / https://api.groq.com / http://localhost:11434
    api_key: String,    // para openai/groq
    model: String,      // nombre del modelo
    temperature: f32,   // 0.0..=1.5
    max_tokens: u32,    // límite
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key: String::new(),
            model: "llama3".to_string(),
            temperature: 0.2,
            max_tokens: 2048,
        }
    }
}

/// App principal
struct ClientApp {
    // Infraestructura
    rt: tokio::runtime::Runtime,
    nats_url: String,
    nats: Option<NatsClient>,
    tx: Sender<GuiEvent>,
    events_rx: Option<Receiver<GuiEvent>>,

    // Visibilidad de paneles/ventanas
    show_explorer: bool,
    show_results: bool,
    show_models_window: bool,
    show_providers_window: bool,
    show_monitor_window: bool,
    show_settings_window: bool,

    // Estado UI y datos
    logs: Vec<String>,
    accent: Color32,
    selected_path: Option<PathBuf>,
    metadata_text: String,
    summary_text: String,
    last_ping_ms: Option<u128>,
    models: Vec<String>,
    provider_report: Option<Value>,

    // Explorador
    current_dir: PathBuf,
    dir_items: Vec<EntryView>,
    needs_refresh: bool,
    show_hidden: bool,
    filter_text: String,
    sort_by: SortBy,
    sort_asc: bool,
    favorites: Vec<PathBuf>,

    // Árbol opcional
    root: DirNode,

    // Ajustes LLM
    llm: LlmConfig,

    // Vista previa
    preview_text: String,
    preview_error: Option<String>,
    preview_max_bytes: usize,
    preview_dirty: bool,
}

impl ClientApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel::<GuiEvent>();
        let rt = tokio::runtime::Runtime::new().expect("Tokio runtime");

        let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut favorites = Vec::new();
        favorites.push(home.clone());
        for name in ["Downloads", "Descargas", "Documents", "Documentos", "Desktop", "Escritorio"] {
            let cand = home.join(name);
            if cand.exists() && cand.is_dir() {
                favorites.push(cand);
            }
        }

        let root = DirNode::new(home.clone());

        let mut app = Self {
            rt,
            nats_url,
            nats: None,
            tx,
            events_rx: Some(rx),

            show_explorer: true,
            show_results: true,
            show_models_window: true,
            show_providers_window: true,
            show_monitor_window: true,
            show_settings_window: true,

            logs: Vec::new(),
            accent: Color32::from_rgb(52, 120, 246),
            selected_path: None,
            metadata_text: String::new(),
            summary_text: String::new(),
            last_ping_ms: None,
            models: Vec::new(),
            provider_report: None,

            current_dir: home.clone(),
            dir_items: Vec::new(),
            needs_refresh: true,
            show_hidden: false,
            filter_text: String::new(),
            sort_by: SortBy::Name,
            sort_asc: true,
            favorites,

            root,
            llm: LlmConfig::default(),

            preview_text: String::new(),
            preview_error: None,
            preview_max_bytes: 64 * 1024, // 64KB
            preview_dirty: false,
        };

        app.spawn_connect_and_ping();
        app
    }

    // ===== Infra / NATS =====

    fn spawn_connect_and_ping(&mut self) {
        let url = self.nats_url.clone();
        let tx = self.tx.clone();

        self.rt.spawn(async move {
            match async_nats::connect(&url).await {
                Ok(client) => {
                    let _ = tx.send(GuiEvent::Status("✅ Conectado a NATS".to_string()));

                    let start = Instant::now();
                    match client.request("mcp.ping", Vec::<u8>::new().into()).await {
                        Ok(_msg) => {
                            let _ = tx.send(GuiEvent::PingMs(start.elapsed().as_millis()));
                        }
                        Err(e) => {
                            let _ = tx.send(GuiEvent::Error(format!("Ping LLM Gateway falló: {e}")));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(GuiEvent::Error(format!("❌ Error conectando a NATS ({url}): {e}")));
                }
            }
        });
    }

    fn ensure_nats(&mut self) -> Result<()> {
        if self.nats.is_some() {
            return Ok(());
        }
        let url = self.nats_url.clone();
        let client = self
            .rt
            .block_on(async_nats::connect(&url))
            .with_context(|| format!("No se pudo conectar a NATS en {url}"))?;
        self.nats = Some(client);
        self.push_log("✅ Conectado a NATS");
        Ok(())
    }

    fn client_clone(&self) -> Option<NatsClient> {
        self.nats.as_ref().cloned()
    }

    fn push_log(&mut self, s: &str) {
        self.logs.push(s.to_string());
    }

    // ===== Acciones LLM/NATS =====

    fn ping_gateway(&mut self) {
        if let Err(e) = self.ensure_nats() {
            self.push_log(&format!("❌ NATS no disponible: {e}"));
            return;
        }
        let tx = self.tx.clone();
        if let Some(c) = self.client_clone() {
            self.rt.spawn(async move {
                let start = Instant::now();
                match c.request("mcp.ping", Vec::<u8>::new().into()).await {
                    Ok(_m) => {
                        let _ = tx.send(GuiEvent::PingMs(start.elapsed().as_millis()));
                        let _ = tx.send(GuiEvent::Status("📡 Ping OK".to_string()));
                    }
                    Err(e) => {
                        let _ = tx.send(GuiEvent::Error(format!("Ping falló: {e}")));
                    }
                }
            });
        }
    }

    /// Obtiene la lista de modelos para el proveedor actual.
    fn list_models(&mut self) {
        if let Err(e) = self.ensure_nats() {
            self.push_log(&format!("❌ NATS no disponible: {e}"));
            return;
        }
        let tx = self.tx.clone();
        let cfg = self.llm.clone();
        if let Some(c) = self.client_clone() {
            self.rt.spawn(async move {
                let payload = serde_json::json!({
                    "provider": cfg.provider,
                    "base_url": cfg.base_url,
                    "api_key": cfg.api_key,
                });
                let data = serde_json::to_vec(&payload).unwrap_or_default();
                match c.request("mcp.provider.list", data.into()).await {
                    Ok(msg) => {
                        let Ok(body) = String::from_utf8(msg.payload.to_vec()) else {
                            let _ = tx.send(GuiEvent::Error("Respuesta binaria inválida al listar modelos".into()));
                            return;
                        };
                        match serde_json::from_str::<Value>(&body) {
                            Ok(v) => {
                                let models = if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
                                    arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
                                } else if let Some(arr) = v.as_array() {
                                    arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
                                } else {
                                    Vec::new()
                                };
                                let _ = tx.send(GuiEvent::Models(models));
                            }
                            Err(_) => {
                                let _ = tx.send(GuiEvent::Error(format!("No se pudo parsear modelos: {body}")));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(GuiEvent::Error(format!("Solicitud de modelos falló: {e}")));
                    }
                }
            });
        }
    }

    fn inspect_providers(&mut self) {
        if let Err(e) = self.ensure_nats() {
            self.push_log(&format!("❌ NATS no disponible: {e}"));
            return;
        }
        let tx = self.tx.clone();
        if let Some(c) = self.client_clone() {
            self.rt.spawn(async move {
                match c.request("mcp.provider.inspect", Vec::<u8>::new().into()).await {
                    Ok(msg) => {
                        let Ok(body) = String::from_utf8(msg.payload.to_vec()) else {
                            let _ = tx.send(GuiEvent::Error("Respuesta binaria inválida al inspeccionar proveedores".into()));
                            return;
                        };
                        match serde_json::from_str::<Value>(&body) {
                            Ok(v) => { let _ = tx.send(GuiEvent::ProviderReport(v)); }
                            Err(e) => { let _ = tx.send(GuiEvent::Error(format!("Inspección inválida: {e} / {body}"))); }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(GuiEvent::Error(format!("Solicitud de inspección falló: {e}")));
                    }
                }
            });
        }
    }

    fn request_metadata(&mut self) {
        let Some(path) = self.selected_path.clone() else {
            self.push_log("Seleccione un archivo para extraer metadatos");
            return;
        };
        if let Err(e) = self.ensure_nats() {
            self.push_log(&format!("❌ NATS no disponible: {e}"));
            return;
        }
        let tx = self.tx.clone();
        if let Some(c) = self.client_clone() {
            self.rt.spawn(async move {
                let payload = serde_json::json!({ "path": path });
                let data = serde_json::to_vec(&payload).unwrap_or_default();
                match c.request("metadata.request", data.into()).await {
                    Ok(msg) => {
                        let body = String::from_utf8_lossy(&msg.payload).to_string();
                        let _ = tx.send(GuiEvent::Metadata(body));
                    }
                    Err(e) => {
                        let _ = tx.send(GuiEvent::Error(format!("metadata.request falló: {e}")));
                    }
                }
            });
        }
    }

    fn request_summary(&mut self) {
        let Some(path) = self.selected_path.clone() else {
            self.push_log("Seleccione un archivo para resumir");
            return;
        };
        if let Err(e) = self.ensure_nats() {
            self.push_log(&format!("❌ NATS no disponible: {e}"));
            return;
        }
        let tx = self.tx.clone();
        if let Some(c) = self.client_clone() {
            self.rt.spawn(async move {
                let payload = serde_json::json!({ "path": path });
                let data = serde_json::to_vec(&payload).unwrap_or_default();
                match c.request("summary.request", data.into()).await {
                    Ok(msg) => {
                        let body = String::from_utf8_lossy(&msg.payload).to_string();
                        let _ = tx.send(GuiEvent::Summary(body));
                    }
                    Err(e) => {
                        let _ = tx.send(GuiEvent::Error(format!("summary.request falló: {e}")));
                    }
                }
            });
        }
    }

    // ===== Vista previa =====

    fn load_preview_now(&mut self) {
        self.preview_error = None;
        self.preview_text.clear();
        let Some(path) = self.selected_path.clone() else {
            return;
        };
        if path.is_dir() {
            self.preview_text = "(La vista previa solo está disponible para archivos)".to_string();
            return;
        }
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                self.preview_error = Some(format!("No se pudo abrir el archivo: {e}"));
                return;
            }
        };
        let mut buf = vec![0u8; self.preview_max_bytes];
        let mut read_total = 0usize;
        match file.read(&mut buf) {
            Ok(n) => read_total = n,
            Err(e) => {
                self.preview_error = Some(format!("Error leyendo: {e}"));
                return;
            }
        }
        buf.truncate(read_total);
        let mut text = String::from_utf8_lossy(&buf).to_string();

        // Si no termina en \n y hay más datos, indica truncado:
        if read_total == self.preview_max_bytes {
            text.push_str("\n… (vista previa truncada)");
        }
        self.preview_text = text;
    }

    // ===== Explorador =====

    fn refresh_dir(&mut self) {
        self.dir_items.clear();
        let dir = self.current_dir.clone();
        let show_hidden = self.show_hidden;
        let filter = self.filter_text.to_lowercase();

        let mut entries: Vec<EntryView> = Vec::new();
        if let Ok(read) = fs::read_dir(&dir) {
            for ent in read.flatten() {
                let p = ent.path();
                let file_name = ent.file_name().to_string_lossy().to_string();

                // ocultos
                if !show_hidden && file_name.starts_with('.') {
                    continue;
                }

                // filtro
                if !filter.is_empty() && !file_name.to_lowercase().contains(&filter) {
                    continue;
                }

                let is_dir = p.is_dir();
                let (size, kind) = if is_dir {
                    (None, "Carpeta".to_string())
                } else {
                    let meta = fs::metadata(&p).ok();
                    let sz = meta.as_ref().map(|m| m.len());
                    let kind = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("archivo")
                        .to_string();
                    (sz, kind)
                };

                entries.push(EntryView {
                    name: file_name,
                    path: p,
                    is_dir,
                    size,
                    kind,
                });
            }
        }

        // ordenar
        entries.sort_by(|a, b| {
            // carpetas primero
            let dir_order = match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if dir_order != std::cmp::Ordering::Equal {
                return dir_order;
            }

            let ord = match self.sort_by {
                SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortBy::Kind => a.kind.to_lowercase().cmp(&b.kind.to_lowercase()),
                SortBy::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            };
            if self.sort_asc {
                ord
            } else {
                ord.reverse()
            }
        });

        self.dir_items = entries;
        self.needs_refresh = false;
    }

    fn human_size(size: u64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;
        const TB: f64 = GB * 1024.0;

        let s = size as f64;
        if s < KB {
            format!("{} B", size)
        } else if s < MB {
            format!("{:.1} KB", s / KB)
        } else if s < GB {
            format!("{:.1} MB", s / MB)
        } else if s < TB {
            format!("{:.1} GB", s / GB)
        } else {
            format!("{:.1} TB", s / TB)
        }
    }

    fn age_str(path: &PathBuf) -> Option<String> {
        let meta = fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        let now = SystemTime::now();
        let dur = now.duration_since(modified).ok().unwrap_or(Duration::ZERO);

        let secs = dur.as_secs();
        let (val, unit) = if secs < 60 {
            (secs, "s")
        } else if secs < 3600 {
            (secs / 60, "min")
        } else if secs < 86400 {
            (secs / 3600, "h")
        } else {
            (secs / 86400, "d")
        };
        Some(format!("{val} {unit}"))
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.needs_refresh = true;
        }
    }

    // ===== Acciones rápidas (OS) =====

    fn open_in_os(path: &Path) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            Command::new("xdg-open").arg(path).spawn()?;
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("open").arg(path).spawn()?;
        }
        #[cfg(target_os = "windows")]
        {
            Command::new("cmd").arg("/C").arg("start").arg(path).spawn()?;
        }
        Ok(())
    }

    // ===== UI helpers =====

    fn poll_events(&mut self) {
        let mut rx_opt = self.events_rx.take();
        if let Some(rx) = rx_opt.as_mut() {
            while let Ok(evt) = rx.try_recv() {
                match evt {
                    GuiEvent::Status(s) => self.push_log(&s),
                    GuiEvent::Error(e) => self.push_log(&format!("❌ {e}")),
                    GuiEvent::PingMs(ms) => {
                        self.last_ping_ms = Some(ms);
                        self.push_log(&format!("📡 Ping Gateway: {ms} ms"));
                    }
                    GuiEvent::Models(list) => {
                        self.models = list;
                        if !self.models.is_empty() && !self.models.contains(&self.llm.model) {
                            self.llm.model = self.models[0].clone();
                            self.push_log(&format!("ℹ️ Modelo ajustado a '{}'", self.llm.model));
                        }
                        self.push_log(&format!("📚 Modelos disponibles: {}", self.models.len()));
                    }
                    GuiEvent::ProviderReport(rep) => {
                        self.provider_report = Some(rep);
                        self.push_log("🔍 Inspección de proveedores actualizada");
                    }
                    GuiEvent::Metadata(m) => {
                        self.metadata_text = m;
                        self.push_log("📊 Metadatos recibidos");
                    }
                    GuiEvent::Summary(s) => {
                        self.summary_text = s;
                        self.push_log("📝 Resumen recibido");
                    }
                }
            }
        }
        self.events_rx = rx_opt;
    }

    fn apply_theme(&mut self, ctx: &EguiContext, dark: bool) {
        if dark {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }
    }

    fn ui_menubar(&mut self, ctx: &EguiContext, ui: &mut Ui) {
        egui::menu::bar(ui, |ui| {
            ui.heading(RichText::new("🧩 Multi-Agent Client").strong());
            ui.separator();

            // Menú de paneles/ventanas
            ui.menu_button("📋 Paneles", |ui| {
                ui.checkbox(&mut self.show_explorer, "Explorador");
                ui.checkbox(&mut self.show_results, "Resultados");
                ui.separator();
                ui.checkbox(&mut self.show_models_window, "Modelos");
                ui.checkbox(&mut self.show_providers_window, "Proveedores");
                ui.checkbox(&mut self.show_monitor_window, "Monitor");
                ui.checkbox(&mut self.show_settings_window, "Ajustes LLM");
            });

            ui.separator();

            ui.menu_button("🎨 Tema", |ui| {
                if ui.button("Oscuro").clicked() {
                    self.apply_theme(ctx, true);
                    ui.close_menu();
                }
                if ui.button("Claro").clicked() {
                    self.apply_theme(ctx, false);
                    ui.close_menu();
                }
            });

            ui.separator();

            if ui.button("📡 Ping").clicked() {
                self.ping_gateway();
            }
            let ping_text = match self.last_ping_ms {
                Some(ms) => format!("{ms} ms"),
                None => "— ms".into(),
            };
            ui.label(format!("Ping: {ping_text}"));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Angel A. Urbina — Copyright 2025").italics());
            });
        });
    }

    fn ui_left_explorer(&mut self, ui: &mut Ui) {
        if self.needs_refresh {
            self.refresh_dir();
        }

        ui.heading("📁 Explorador de archivos");
        ui.add_space(6.0);

        // Barra de acciones de navegación
        ui.horizontal(|ui| {
            if ui.button("⬆ Arriba").clicked() {
                self.go_up();
            }
            if ui.button("⟳ Recargar").clicked() {
                self.needs_refresh = true;
            }
            if ui.button("⭐ Favorito").clicked() {
                if !self.favorites.contains(&self.current_dir) {
                    self.favorites.push(self.current_dir.clone());
                }
            }
        });

        // Breadcrumbs seguros (snapshot para evitar préstamos activos)
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let path_snapshot = self.current_dir.clone();
            let mut acc = PathBuf::new();
            let mut first = true;

            #[cfg(target_family = "unix")]
            {
                acc.push("/");
                if ui.button("/").clicked() {
                    self.current_dir = PathBuf::from("/");
                    self.needs_refresh = true;
                }
                ui.label(" / ");
            }

            for comp in path_snapshot.components() {
                let c = comp.as_os_str().to_string_lossy().to_string();
                if c == "/" {
                    continue;
                }
                if !first {
                    ui.label(" / ");
                }
                first = false;
                acc = acc.join(&c);
                let acc_clone = acc.clone();
                if ui.button(&c).clicked() {
                    self.current_dir = acc_clone;
                    self.needs_refresh = true;
                }
            }
        });

        ui.add_space(6.0);

        // Filtro/Orden
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("Filtro:");
                ui.text_edit_singleline(&mut self.filter_text);
                if ui.button("Limpiar").clicked() {
                    self.filter_text.clear();
                    self.needs_refresh = true;
                }
                ui.checkbox(&mut self.show_hidden, "Ocultos");
            });

            ui.horizontal(|ui| {
                ui.label("Ordenar por:");
                egui::ComboBox::from_id_source("sort_by")
                    .selected_text(match self.sort_by { SortBy::Name => "Nombre", SortBy::Kind => "Tipo", SortBy::Size => "Tamaño" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_by, SortBy::Name, "Nombre");
                        ui.selectable_value(&mut self.sort_by, SortBy::Kind, "Tipo");
                        ui.selectable_value(&mut self.sort_by, SortBy::Size, "Tamaño");
                    });
                if ui.button(if self.sort_asc { "⬆︎ Asc" } else { "⬇︎ Desc" }).clicked() {
                    self.sort_asc = !self.sort_asc;
                    self.needs_refresh = true;
                }
            });
        });

        ui.add_space(6.0);

        // Favoritos + Árbol (colapsables)
        egui::CollapsingHeader::new("⭐ Favoritos")
            .default_open(true)
            .show(ui, |ui| {
                for fav in self.favorites.clone() {
                    ui.horizontal(|ui| {
                        if ui.button("➡").clicked() {
                            self.current_dir = fav.clone();
                            self.needs_refresh = true;
                        }
                        ui.label(fav.to_string_lossy());
                    });
                }
            });

        egui::CollapsingHeader::new("🌲 Árbol (opcional)")
            .default_open(false)
            .show(ui, |ui| {
                draw_tree_select(ui, &mut self.root, &mut self.selected_path);
                if let Some(sel) = &self.selected_path {
                    if sel.is_dir() && ui.button("Abrir carpeta seleccionada").clicked() {
                        self.current_dir = sel.clone();
                        self.needs_refresh = true;
                    }
                }
            });

        ui.separator();

        // Contenidos del directorio actual (lista con SCROLL)
        ui.heading("📂 Contenido");
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .id_source("dir_list")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // Cabecera
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Nombre").strong());
                    ui.add_space(12.0);
                    ui.label(RichText::new("Tipo").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Tamaño").strong());
                    });
                });
                ui.separator();

                for item in self.dir_items.clone() {
                    let row = ui.horizontal(|ui| {
                        let icon = if item.is_dir { "📁" } else { "📄" };
                        let label = format!("{icon} {}", item.name);
                        let resp = ui.selectable_label(
                            self.selected_path.as_ref().map(|p| p == &item.path).unwrap_or(false),
                            label,
                        );
                        if resp.clicked() {
                            self.selected_path = Some(item.path.clone());
                            self.preview_dirty = true; // cargar vista previa
                        }
                        if resp.double_clicked() && item.is_dir {
                            self.current_dir = item.path.clone();
                            self.needs_refresh = true;
                        }

                        ui.add_space(12.0);
                        ui.label(&item.kind);

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let size_str = item.size.map(Self::human_size).unwrap_or_else(|| "—".into());
                            ui.label(size_str);
                        });
                    });

                    if row.response.hovered() {
                        if let Some(age) = Self::age_str(&item.path) {
                            row.response.on_hover_text(format!("Modificado hace {age}"));
                        }
                    }
                }
            });

        ui.add_space(6.0);
        // Acciones sobre el archivo seleccionado:
        ui.horizontal(|ui| {
            let enabled = self.selected_path.is_some();
            ui.add_enabled_ui(enabled, |ui| {
                if ui.button("📊 Metadatos").clicked() {
                    self.request_metadata();
                }
                if ui.button("📝 Resumen").clicked() {
                    self.request_summary();
                }
            });
            if let Some(sel) = &self.selected_path {
                ui.label(format!("Seleccionado: {}", sel.file_name().and_then(|s| s.to_str()).unwrap_or("")));
            }
        });
    }

    fn ui_center_results(&mut self, ui: &mut Ui) {
        ui.heading("🧾 Resultados");
        ui.add_space(8.0);

        // Info + acciones rápidas
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Archivo seleccionado:");
                if let Some(p) = &self.selected_path {
                    ui.code(p.to_string_lossy());
                } else {
                    ui.weak("— (nada seleccionado)");
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enabled = self.selected_path.is_some();
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.button("🔗 Copiar ruta").clicked() {
                        if let Some(p) = &self.selected_path {
                            let s = p.to_string_lossy().to_string();
                            ui.output_mut(|o| o.copied_text = s.clone());
                            self.push_log(&format!("📋 Copiado: {s}"));
                        }
                    }
                    if ui.button("📋 Copiar nombre").clicked() {
                        if let Some(p) = &self.selected_path {
                            let s = p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
                            ui.output_mut(|o| o.copied_text = s.clone());
                            self.push_log(&format!("📋 Copiado: {s}"));
                        }
                    }
                    if ui.button("🖼️ Abrir archivo").clicked() {
                        if let Some(p) = &self.selected_path {
                            if let Err(e) = Self::open_in_os(p.as_path()) {
                                self.push_log(&format!("❌ No se pudo abrir: {e}"));
                            }
                        }
                    }
                    if ui.button("📂 Abrir carpeta").clicked() {
                        if let Some(p) = &self.selected_path {
                            if let Some(parent) = p.parent() {
                                if let Err(e) = Self::open_in_os(parent) {
                                    self.push_log(&format!("❌ No se pudo abrir carpeta: {e}"));
                                }
                            }
                        }
                    }
                });
            });
        });

        ui.add_space(8.0);

        // Resumen / Metadatos lado a lado
        ui.columns(2, |cols| {
            cols[0].group(|ui| {
                ui.heading("📝 Resumen");
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                    ui.label(&self.summary_text);
                    ui.style_mut().override_text_style = None;
                });
            });
            cols[1].group(|ui| {
                ui.heading("📊 Metadatos");
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                    ui.label(&self.metadata_text);
                    ui.style_mut().override_text_style = None;
                });
            });
        });

        ui.add_space(8.0);

        // Vista previa (monoespaciada) con scroll
        ui.group(|ui| {
            ui.heading("👀 Vista previa del archivo");
            ui.add_space(6.0);

            if let Some(err) = &self.preview_error {
                ui.colored_label(Color32::from_rgb(200, 80, 80), err);
            }

            let hint = format!(
                "Mostrando primeras ~{} KB{}",
                self.preview_max_bytes / 1024,
                if self.preview_text.ends_with("… (vista previa truncada)") { " (truncado)" } else { "" }
            );
            ui.weak(hint);

            egui::ScrollArea::vertical()
                .id_source("preview_scroll")
                .auto_shrink([false; 2])
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                    if self.preview_text.is_empty() && self.preview_error.is_none() {
                        ui.weak("— No hay vista previa. Seleccione un archivo en el explorador.");
                    } else {
                        ui.label(&self.preview_text);
                    }
                    ui.style_mut().override_text_style = None;
                });
        });

        ui.add_space(8.0);
        ui.separator();

        ui.heading("🧯 Log de eventos / errores");
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            for line in &self.logs {
                ui.label(line);
            }
        });
    }

    fn ui_models_window(&mut self, ctx: &EguiContext) {
        let mut open = self.show_models_window;
        let mut trigger_list = false;

        egui::Window::new("📚 Modelos disponibles")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .default_height(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("🔄 Actualizar modelos").clicked() {
                        trigger_list = true;
                    }
                    ui.label(format!("Total: {}", self.models.len()));
                });
                ui.separator();

                let models = self.models.clone();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for m in models {
                        ui.horizontal(|ui| {
                            ui.label("•");
                            if ui.selectable_label(self.llm.model == m, &m).clicked() {
                                self.llm.model = m.clone();
                                self.push_log(&format!("✅ Modelo seleccionado: {}", m));
                            }
                        });
                    }
                });
            });
        self.show_models_window = open;

        if trigger_list {
            self.list_models();
        }
    }

    fn ui_providers_window(&mut self, ctx: &EguiContext) {
        let mut open = self.show_providers_window;
        let mut trigger_inspect = false;

        egui::Window::new("🔍 Proveedores detectados")
            .open(&mut open)
            .resizable(true)
            .default_width(600.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("🔎 Inspeccionar").clicked() {
                        trigger_inspect = true;
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let text = match &self.provider_report {
                        Some(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| "<json inválido>".into()),
                        None => "— (sin datos aún)".into(),
                    };
                    ui.style_mut().override_text_style = Some(TextStyle::Monospace);
                    ui.label(text);
                    ui.style_mut().override_text_style = None;
                });
            });
        self.show_providers_window = open;

        if trigger_inspect {
            self.inspect_providers();
        }
    }

    fn ui_monitor_window(&mut self, ctx: &EguiContext) {
        let mut open = self.show_monitor_window;
        let mut trigger_ping = false;
        let mut trigger_reconnect = false;

        egui::Window::new("📡 Monitor")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .default_height(260.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("📡 Ping LLM Gateway").clicked() {
                        trigger_ping = true;
                    }
                    if let Some(ms) = self.last_ping_ms {
                        ui.label(format!("Último ping: {} ms", ms));
                    } else {
                        ui.label("Último ping: —");
                    }
                });

                ui.separator();
                ui.label(format!("NATS_URL: {}", self.nats_url));
                if ui.button("🔌 Re-conectar NATS").clicked() {
                    trigger_reconnect = true;
                }
            });

        self.show_monitor_window = open;
        if trigger_ping {
            self.ping_gateway();
        }
        if trigger_reconnect {
            self.nats = None;
            if let Err(e) = self.ensure_nats() {
                self.push_log(&format!("❌ Reconexión NATS falló: {e}"));
            } else {
                self.push_log("✅ Reconectado a NATS");
            }
        }
    }

    fn ui_settings_window(&mut self, ctx: &EguiContext) {
        let mut open = self.show_settings_window;

        // Disparadores diferidos para evitar préstamos simultáneos
        let mut trigger_list_models = false;

        egui::Window::new("⚙️ Ajustes LLM / Gateway")
            .open(&mut open)
            .resizable(true)
            .default_width(660.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                // Sección: Proveedor
                ui.group(|ui| {
                    ui.heading("Proveedor");
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Proveedor:");
                        let mut provider_changed = false;

                        egui::ComboBox::from_id_source("provider_settings")
                            .selected_text(&self.llm.provider)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_value(&mut self.llm.provider, "openai".to_string(), "OpenAI")
                                    .clicked()
                                {
                                    if self.llm.base_url.is_empty() || self.llm.base_url.contains("localhost") {
                                        self.llm.base_url = "https://api.openai.com".to_string();
                                    }
                                    provider_changed = true;
                                }
                                if ui
                                    .selectable_value(&mut self.llm.provider, "groq".to_string(), "Groq")
                                    .clicked()
                                {
                                    if self.llm.base_url.is_empty() || self.llm.base_url.contains("openai") {
                                        self.llm.base_url = "https://api.groq.com".to_string();
                                    }
                                    provider_changed = true;
                                }
                                if ui
                                    .selectable_value(&mut self.llm.provider, "ollama".to_string(), "Ollama")
                                    .clicked()
                                {
                                    if self.llm.base_url.is_empty() || self.llm.base_url.contains("api.") {
                                        self.llm.base_url = "http://localhost:11434".to_string();
                                    }
                                    provider_changed = true;
                                }
                            });

                        if provider_changed {
                            self.models.clear();
                            trigger_list_models = true; // auto carga lista del proveedor actual
                        }

                        if ui.button("📚 Obtener modelos").clicked() {
                            trigger_list_models = true;
                        }
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Base URL:");
                        ui.text_edit_singleline(&mut self.llm.base_url);
                    });

                    if self.llm.provider != "ollama" {
                        ui.horizontal(|ui| {
                            ui.label("API Key:");
                            let mut masked = self.llm.api_key.clone();
                            if ui.add(egui::TextEdit::singleline(&mut masked).password(true)).changed() {
                                self.llm.api_key = masked;
                            }
                        });
                    } else {
                        ui.weak("Ollama no requiere API Key (usa servidor local).");
                    }
                });

                ui.add_space(8.0);

                // Sección: Modelos
                ui.group(|ui| {
                    ui.heading("Modelos");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Modelo actual:");
                        // Clonamos la lista para evitar préstamo inmutable mientras mutamos self
                        let models = self.models.clone();
                        egui::ComboBox::from_id_source("model_settings")
                            .selected_text(if self.llm.model.is_empty() { "—" } else { &self.llm.model })
                            .show_ui(ui, |ui| {
                                if models.is_empty() {
                                    ui.weak("— (sin lista; pulsa 'Obtener modelos')");
                                } else {
                                    for m in models {
                                        if ui.selectable_value(&mut self.llm.model, m.clone(), m.clone()).clicked() {
                                            self.push_log(&format!("✅ Modelo seleccionado: {}", m));
                                        }
                                    }
                                }
                            });
                        if ui.button("🔄").on_hover_text("Refrescar modelos").clicked() {
                            trigger_list_models = true;
                        }
                    });

                    ui.add_space(4.0);
                    ui.weak("La lista de modelos se consulta al gateway según el proveedor y credenciales configurados.");
                });

                ui.add_space(8.0);

                // Sección: Parámetros
                ui.group(|ui| {
                    ui.heading("Parámetros de inferencia");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Temperatura:");
                        ui.add(egui::Slider::new(&mut self.llm.temperature, 0.0..=1.5).suffix(" ℃"));
                    });

                    ui.horizontal(|ui| {
                        ui.label("Máx. tokens:");
                        let mut val = self.llm.max_tokens as i64;
                        if ui.add(egui::DragValue::new(&mut val)).changed() {
                            if val < 0 { val = 0; }
                            if val > 32768 { val = 32768; }
                            self.llm.max_tokens = val as u32;
                        }
                    });
                });

                ui.add_space(12.0);
                ui.label("Estos ajustes se usan para listar modelos y diagnosticar el gateway.\nEl agente 'summarizer' tomará su configuración del LLM Gateway según lo que esté configurado allí.");
            });

        self.show_settings_window = open;

        // Ejecutar acciones diferidas fuera del cierre para evitar conflictos de préstamos
        if trigger_list_models {
            self.list_models();
        }
    }
}

impl eframe::App for ClientApp {
    fn update(&mut self, ctx: &EguiContext, _frame: &mut eframe::Frame) {
        self.poll_events();

        // Si hay que refrescar vista previa, hazlo fuera de cierres UI:
        if self.preview_dirty {
            self.load_preview_now();
            self.preview_dirty = false;
        }

        egui::TopBottomPanel::top("top_menu").show(ctx, |ui| {
            self.ui_menubar(ctx, ui);
        });

        if self.show_explorer {
            egui::SidePanel::left("left_explorer")
                .default_width(360.0)
                .resizable(true)
                .show(ctx, |ui| {
                    self.ui_left_explorer(ui);
                });
        }

        if self.show_results {
            egui::CentralPanel::default().show(ctx, |ui| {
                self.ui_center_results(ui);
            });
        }

        // Subventanas
        self.ui_models_window(ctx);
        self.ui_providers_window(ctx);
        self.ui_monitor_window(ctx);
        self.ui_settings_window(ctx);
    }
}

/// Árbol de selección (opcional). No navega por sí mismo; sirve para elegir y luego "Abrir carpeta".
fn draw_tree_select(ui: &mut Ui, node: &mut DirNode, selected_path: &mut Option<PathBuf>) {
    if node.is_dir {
        let label = format!("📂 {}", node.name);
        egui::CollapsingHeader::new(label)
            .id_source(node.path.clone())
            .default_open(false)
            .show(ui, |ui| {
                node.ensure_children_loaded();
                if let Some(children) = node.children.as_mut() {
                    for child in children {
                        draw_tree_select(ui, child, selected_path);
                    }
                }
            });
    } else {
        let selected = selected_path.as_ref().map(|p| p == &node.path).unwrap_or(false);
        if ui
            .selectable_label(selected, format!("📄 {}", node.name))
            .clicked()
        {
            *selected_path = Some(node.path.clone());
        }
    }
}

fn main() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    let _ = tracing_subscriber::fmt::try_init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(1200.0, 820.0)),
        ..Default::default()
    };

    eframe::run_native(
        "🧩 Multi-Agent Client",
        native_options,
        Box::new(|cc| Box::new(ClientApp::new(cc))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
