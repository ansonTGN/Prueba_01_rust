# Procesador de Archivos Multi-Agente

[![Estado de la Construcción](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/usuario/repositorio)
[![Licencia](https://img.shields.io/badge/license-MIT-blue)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.79%2B-orange)](https://www.rust-lang.org/)

Un sistema distribuido y extensible construido en Rust que utiliza una arquitectura multi-agente para procesar, analizar y resumir archivos locales. Los agentes se comunican a través de un bus de mensajería NATS, permitiendo una separación clara de responsabilidades y una alta escalabilidad.

El proyecto incluye un **LLM Gateway** unificado que actúa como intermediario para múltiples proveedores de modelos de lenguaje (como Ollama, OpenAI y Groq), y un **Cliente Interactivo** con una interfaz gráfica para una fácil gestión y visualización de los resultados.

## 🚀 Características Principales

-   **Arquitectura Multi-Agente**: Cada tarea (exploración de archivos, extracción de metadatos, resumen) es manejada por un agente independiente.
-   **Comunicación Asíncrona**: Utiliza NATS para una comunicación robusta y desacoplada entre los agentes.
-   **LLM Gateway Unificado**: Abstrae la complejidad de interactuar con diferentes proveedores de LLM (Ollama, OpenAI, Groq). Permite la configuración y el enrutamiento de solicitudes de manera centralizada.
-   **Cliente de Escritorio Interactivo**: Una interfaz gráfica de usuario (GUI) construida con `egui` para explorar archivos, solicitar acciones y visualizar resultados en tiempo real.
-   **Lanzador de Agentes Centralizado**: Un binario (`agent_launcher`) gestiona el ciclo de vida de todos los agentes del sistema, aplicando políticas de reinicio configurables.
-   **Configuración Flexible**: El comportamiento del sistema se define a través de un archivo `config.toml` y variables de entorno (`.env`).

## 🏗️ Arquitectura del Sistema

El sistema opera con varios componentes independientes que colaboran entre sí:

1.  **NATS Server**: Actúa como el sistema nervioso central, manejando todos los mensajes entre los agentes.
2.  **Agent Launcher**: Es el punto de entrada. Lee `config.toml`, compila y lanza todos los agentes de backend habilitados.
3.  **Agentes de Backend**:
    -   `file_explorer`: Escanea directorios y sirve el contenido de los archivos bajo demanda.
    -   `metadata_extractor`: Proporciona metadatos detallados de los archivos (tamaño, fechas, tipo).
    -   `summarizer`: Orquesta el proceso de resumen. Obtiene el contenido del archivo y solicita un resumen al LLM Gateway.
    -   `llm_gateway`: Recibe solicitudes de los agentes y las reenvía al proveedor de LLM configurado, gestionando la autenticación y el formato de la API.
4.  **Cliente Interactivo**: Es la interfaz de usuario final. Se conecta a NATS para enviar solicitudes (por ejemplo, "resume este archivo") y mostrar los resultados devueltos por los agentes.

![Diagrama de Arquitectura (Conceptual)](https://i.imgur.com/example.png)  
*(Nota: Reemplaza esta imagen con un diagrama de tu arquitectura si lo tienes).*

## 🛠️ Componentes

El proyecto se divide en los siguientes binarios:

| Binario                | Archivo Fuente                     | Descripción                                                                                              |
| ---------------------- | ---------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `agent_launcher`       | `src/bin/6_agent_launcher.rs`      | **Orquestador principal.** Lanza y supervisa a los demás agentes según la configuración de `config.toml`.  |
| `file_explorer`        | `src/bin/1_file_explorer.rs`       | Agente que escanea el sistema de archivos y sirve el contenido de los ficheros.                          |
| `metadata_extractor`   | `src/bin/2_metadata_extractor.rs`  | Agente que extrae y devuelve metadatos de un archivo específico.                                         |
| `summarizer`           | `src/bin/3_summarizer.rs`          | Agente que solicita contenido y lo envía al LLM Gateway para obtener un resumen.                         |
| `llm_gateway`          | `src/bin/5_llm_gateway.rs`         | **Puerta de enlace a LLMs.** Gestiona la comunicación con proveedores como Ollama, OpenAI y Groq.          |
| `interactive_client`   | `src/bin/4_interactive_client.rs`  | **GUI de escritorio.** Permite al usuario interactuar con el sistema de agentes.                           |

## ⚙️ Requisitos Previos

Antes de comenzar, asegúrate de tener instalado lo siguiente:

-   **Rust**: La cadena de herramientas de Rust (compilador `rustc` y gestor de paquetes `cargo`). Puedes instalarla desde [rustup.rs](https://rustup.rs/).
-   **Docker**: Para ejecutar fácilmente el servidor NATS.
-   **(Opcional) Ollama**: Si deseas utilizar modelos de lenguaje de forma local. Asegúrate de descargar un modelo, por ejemplo: `ollama pull llama3.1:8b`.

## 🚀 Guía de Inicio Rápido

Sigue estos pasos para poner en marcha todo el sistema.

### 1. Clonar el Repositorio

```bash
git clone https://github.com/tu_usuario/tu_repositorio.git
cd tu_repositorio
```

### 2. Configurar el Entorno

Crea un archivo `.env` en la raíz del proyecto. Este archivo contendrá las variables de entorno necesarias para los agentes.

```dotenv
# .env

# URL del servidor NATS. Si usas Docker como se indica a continuación, esta IP es la del contenedor.
NATS_URL="nats://127.0.0.1:4222"

# Directorio que el explorador de archivos escaneará.
DIRECTORY_TO_SCAN="/ruta/a/tus/documentos"

# Configuración para el agente Summarizer y el LLM Gateway
# Formato: proveedor:nombre-del-modelo
SUMMARIZER_MODEL="ollama:llama3.1:8b"
LLM_PROVIDER="ollama" # auto | ollama | openai | groq

# (Opcional) Claves de API si usas servicios remotos
OPENAI_API_KEY="sk-..."
GROQ_API_KEY="gsk_..."
```

### 3. Iniciar NATS Server

Puedes usar Docker para lanzar una instancia de NATS de forma sencilla.

```bash
docker run --rm -p 4222:4222 -p 8222:8222 --name nats-server nats:latest
```

### 4. Iniciar los Agentes de Backend

En una terminal, ejecuta el `agent_launcher`. Este se encargará de compilar (si es necesario) y ejecutar todos los agentes habilitados en `config.toml`.

```bash
cargo run --bin agent_launcher
```

### 5. Iniciar el Cliente Interactivo

En una **segunda terminal**, lanza la interfaz gráfica.

```bash
cargo run --bin interactive_client
```

¡Listo! Ahora deberías ver la ventana del cliente interactivo, desde donde podrás explorar archivos y solicitar resúmenes.

## 🔧 Configuración

El comportamiento de los agentes se puede ajustar a través de dos archivos principales:

### `config.toml`

Este archivo es utilizado por `agent_launcher` para determinar qué agentes iniciar y cómo gestionarlos.

-   `build_profile`: Define si se compila en modo `debug` o `release`.
-   `[[agents]]`: Es una lista donde cada entrada define un agente.
    -   `name`: Nombre descriptivo para los logs.
    -   `bin`: Nombre del binario ejecutable.
    -   `enabled`: `true` para que el lanzador lo inicie, `false` para ignorarlo.
    -   `restart`: Política de reinicio (`never`, `on_failure`, `always`).

### `.env`

Define las variables de entorno utilizadas por los agentes en tiempo de ejecución, como la URL de NATS, las claves de API y la configuración de los modelos de lenguaje.

## 📂 Estructura del Directorio

```
└── ./
    ├── src
    │   ├── bin                 # Contiene los ejecutables (agentes)
    │   │   ├── 1_file_explorer.rs
    │   │   ├── 2_metadata_extractor.rs
    │   │   ├── 3_summarizer.rs
    │   │   ├── 4_interactive_client.rs
    │   │   ├── 5_llm_gateway.rs
    │   │   └── 6_agent_launcher.rs
    │   ├── lib.rs              # Librería compartida con tipos y funciones comunes
    │   └── mcp_protocol.rs     # Define el protocolo de comunicación con el LLM Gateway
    ├── arranque.txt            # Instrucciones de arranque rápido para el backend
    ├── arranque_visualizador.txt # Instrucciones para el cliente
    ├── Cargo.toml              # Manifiesto del proyecto y dependencias
    ├── config.toml             # Configuración del lanzador de agentes
    └── README.md               # Este archivo
```

## 📄 Licencia

Este proyecto está distribuido bajo la Licencia MIT. Consulta el archivo `LICENSE` para más detalles.