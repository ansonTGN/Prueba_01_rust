# Sistema de Agentes Inteligentes en Rust

**Un laboratorio de desarrollo para explorar sistemas distribuidos, agentes de IA y la eficiencia de Rust.**

[![Estado de la Construcción](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/usuario/repositorio)
[![Licencia](https://img.shields.io/badge/license-MIT-blue)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.79%2B-orange)](https://www.rust-lang.org/)
[![NATS](https://img.shields.io/badge/comms-NATS-blueviolet)](https://nats.io/)

Este repositorio documenta el desarrollo de un sistema distribuido y extensible construido en Rust. Utiliza una arquitectura multi-agente para procesar, analizar y resumir archivos, sirviendo como un entorno práctico para la investigación y aplicación de tecnologías avanzadas.

El sistema se fundamenta en una comunicación asíncrona a través de un bus de mensajería NATS y se integra con múltiples proveedores de Modelos de Lenguaje Grandes (LLM) a través de un **Gateway** unificado, demostrando un enfoque modular y agnóstico a la tecnología.

## 🔭 Visión del Proyecto

Este proyecto es más que una simple aplicación; es un campo de pruebas diseñado para profundizar en varias áreas clave de la ingeniería de software moderna:

*   **Dominio de Rust:** Aprovechar la seguridad, concurrencia y rendimiento de Rust para construir servicios de bajo nivel robustos y eficientes.
*   **Sistemas Distribuidos:** Diseñar y operar una arquitectura descentralizada, enfrentando desafíos como la comunicación entre servicios, la tolerancia a fallos y la escalabilidad.
*   **Agentes de Inteligencia Artificial:** Implementar un sistema multi-agente donde entidades autónomas colaboran para resolver tareas complejas, sentando las bases para aplicaciones de IA más sofisticadas.
*   **Integración de LLMs:** Abstraer la comunicación con diversos proveedores de LLM (Ollama, OpenAI, Groq), desarrollando una pasarela flexible que centraliza la lógica y facilita la extensibilidad.
*   **Protocolos de Comunicación Eficientes:** Diseñar e implementar un protocolo de comunicación personalizado (MCP) para estandarizar y optimizar el intercambio de datos entre los agentes y los sistemas de IA.

## 🚀 Características Principales

*   **Arquitectura Multi-Agente:** Cada responsabilidad (exploración de archivos, extracción de metadatos, resumen) es encapsulada en un agente independiente y autónomo.
*   **Comunicación Asíncrona y Desacoplada:** El uso de NATS como bus de mensajería garantiza la resiliencia y escalabilidad del sistema, permitiendo que los agentes operen de forma independiente.
*   **LLM Gateway Unificado:** Actúa como un proxy inteligente que abstrae la complejidad de interactuar con diferentes APIs de LLMs, permitiendo el enrutamiento y la configuración centralizada.
*   **Cliente Interactivo de Escritorio:** Una GUI construida con `egui` que sirve como centro de mando para interactuar con el ecosistema de agentes y visualizar resultados en tiempo real.
*   **Orquestador de Agentes:** Un lanzador centralizado (`agent_launcher`) que gestiona el ciclo de vida de todos los agentes del sistema, aplicando políticas de reinicio para garantizar la alta disponibilidad.
*   **Configuración Declarativa:** El comportamiento del sistema se define a través de un archivo `config.toml` y variables de entorno, facilitando la personalización y el despliegue.

## 🏗️ Arquitectura del Sistema

La arquitectura está diseñada siguiendo los principios de los sistemas distribuidos, priorizando el **desacoplamiento**, la **resiliencia** y la **escalabilidad horizontal**. Cada componente es un proceso independiente que se comunica exclusivamente a través del bus de mensajería, lo que permite que puedan ser desplegados, actualizados y reiniciados de forma aislada.

1.  **NATS Server**: Actúa como el sistema nervioso central, gestionando todos los flujos de comunicación.
2.  **Agent Launcher**: Punto de entrada del backend. Orquesta el lanzamiento y la supervisión de los agentes según la configuración definida.
3.  **Agentes de Backend**:
    *   `file_explorer`: Escanea directorios locales y expone el contenido de los archivos bajo demanda.
    *   `metadata_extractor`: Extrae y sirve metadatos estructurados de los archivos.
    *   `summarizer`: Orquesta el flujo de resumen, coordinando con otros agentes para obtener datos y con el LLM Gateway para generar el resultado.
    *   `llm_gateway`: Recibe solicitudes de procesamiento de lenguaje natural y las delega al proveedor de LLM configurado, gestionando la autenticación y adaptación de la API.
4.  **Cliente Interactivo**: La interfaz de usuario final que se conecta a NATS para enviar comandos y recibir actualizaciones de manera asíncrona.

![Diagrama de Arquitectura (Conceptual)](https://i.imgur.com/example.png)
*(Nota: Reemplaza esta imagen con un diagrama de tu arquitectura detallado).*

## 💡 Protocolo de Comunicación (MCP)

Para garantizar una comunicación eficiente y estandarizada entre el `summarizer` y el `llm_gateway`, se ha diseñado un Protocolo de Comunicación de Modelos (MCP, *Model Communication Protocol*).

*   **Propósito**: Define una estructura de mensajes agnóstica al proveedor final de LLM, permitiendo que los agentes soliciten tareas de IA sin necesidad de conocer los detalles de implementación de OpenAI, Groq u otros.
*   **Implementación**: La especificación y las estructuras de datos de este protocolo se encuentran en `src/mcp_protocol.rs`.

## 🛠️ Componentes

El proyecto se estructura en los siguientes binarios ejecutables:

| Binario | Archivo Fuente | Descripción |
|---|---|---|
| `agent_launcher` | `src/bin/6_agent_launcher.rs` | **Orquestador principal.** Lanza y supervisa a los demás agentes. |
| `file_explorer` | `src/bin/1_file_explorer.rs` | Agente que escanea el sistema de archivos y sirve el contenido. |
| `metadata_extractor` | `src/bin/2_metadata_extractor.rs` | Agente que extrae y devuelve metadatos de un archivo. |
| `summarizer` | `src/bin/3_summarizer.rs` | Agente que orquesta la lógica de resumen y se comunica con el LLM Gateway. |
| `llm_gateway` | `src/bin/5_llm_gateway.rs` | **Puerta de enlace a LLMs.** Gestiona la comunicación con proveedores externos. |
| `interactive_client` | `src/bin/4_interactive_client.rs` | **GUI de escritorio.** Permite al usuario interactuar con el sistema. |

## ⚙️ Requisitos Previos

*   **Rust**: Toolchain de Rust (`rustc` y `cargo`). Instálalo desde [rustup.rs](https://rustup.rs/).
*   **Docker**: Para ejecutar el servidor NATS de forma aislada.
*   **(Opcional) Ollama**: Si deseas utilizar modelos de lenguaje de forma local. Asegúrate de tener un modelo descargado (ej: `ollama pull llama3.1:8b`).

## 🚀 Guía de Inicio Rápido

### 1. Clonar el Repositorio

```bash
git clone https://github.com/tu_usuario/tu_repositorio.git
cd tu_repositorio
```

### 2. Configurar el Entorno

Crea un archivo `.env` a partir de un `env.example` (buena práctica) o desde cero en la raíz del proyecto.

```dotenv
# .env

# URL del servidor NATS.
NATS_URL="nats://127.0.0.1:4222"

# Directorio que el explorador de archivos escaneará.
DIRECTORY_TO_SCAN="/ruta/absoluta/a/tus/documentos"

# Configuración para el LLM Gateway
SUMMARIZER_MODEL="ollama:llama3.1:8b" # Formato: proveedor:nombre-del-modelo
LLM_PROVIDER="ollama"                 # Proveedor por defecto: auto | ollama | openai | groq

# (Opcional) Claves de API para servicios remotos
OPENAI_API_KEY="sk-..."
GROQ_API_KEY="gsk_..."
```

### 3. Iniciar NATS Server

Usa Docker para lanzar una instancia de NATS de forma rápida y limpia.

```bash
docker run --rm -p 4222:4222 -p 8222:8222 --name nats-server nats:latest
```

### 4. Iniciar los Agentes de Backend

En una terminal, ejecuta el `agent_launcher`. Este compilará y ejecutará todos los agentes habilitados en `config.toml`.

```bash
cargo run --bin agent_launcher
```

### 5. Iniciar el Cliente Interactivo

En una **segunda terminal**, lanza la interfaz gráfica.

```bash
cargo run --bin interactive_client```

¡Listo! La GUI se conectará al ecosistema de agentes a través de NATS, permitiéndote explorar archivos y solicitar resúmenes.

## 🔧 Configuración Avanzada

### `config.toml`

Este archivo es el manifiesto para el `agent_launcher`.

*   `build_profile`: Perfil de compilación (`debug` o `release`).
*   `[[agents]]`: Lista de agentes a gestionar.
    *   `name`: Nombre descriptivo para logs.
    *   `bin`: Nombre del binario ejecutable.
    *   `enabled`: `true` para iniciarlo, `false` para ignorarlo.
    *   `restart`: Política de reinicio (`never`, `on_failure`, `always`).

## 🌱 Desarrollo y Futuras Mejoras

Este proyecto está en constante evolución. Algunas de las áreas de interés para el futuro desarrollo incluyen:

*   **Añadir Nuevos Agentes:** Crear agentes para tareas como extracción de entidades (NER), análisis de sentimiento o clasificación de documentos.
*   **Soporte para más Proveedores de LLM:** Integrar nuevos proveedores en el `llm_gateway` como Anthropic (Claude) o Cohere.
*   **Persistencia de Datos:** Implementar un agente de base de datos para almacenar y consultar los resultados de los análisis.
*   **Containerización Completa:** Escribir un archivo `docker-compose.yml` para levantar todo el ecosistema (NATS y todos los agentes) con un solo comando.
*   **Seguridad y Autenticación:** Implementar mecanismos de seguridad en NATS para proteger la comunicación entre agentes.

Las contribuciones y sugerencias son siempre bienvenidas.

## 📄 Licencia

Este proyecto está distribuido bajo la Licencia MIT. Consulta el archivo `LICENSE` para más detalles.