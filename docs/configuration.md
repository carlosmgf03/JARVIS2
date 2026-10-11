# Configuración

## Requisitos

| Componente | Versión |
|---|---|
| Node.js | 20 o superior (probado con 22) |
| Rust | 1.80 o superior (probado con 1.97) |
| Dependencias de Tauri 2 | Ver <https://tauri.app/start/prerequisites/>. Linux (Debian/Ubuntu): `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev libdbus-1-dev` |
| Cuenta de OpenAI | Con acceso a un modelo Realtime (por defecto `gpt-realtime-2.1`) |
| Cuenta de Anthropic | Opcional |

## Puesta en marcha

```bash
npm ci
# 1. Credenciales (fuera del frontend y del repositorio)
cargo run -p jarvis-core --bin jarvis -- secrets set openai
cargo run -p jarvis-core --bin jarvis -- secrets set anthropic   # opcional
# 2. Vault de Obsidian
export JARVIS_VAULT_PATH="$HOME/Obsidian/MiVault"
# 3. Comprobación
cargo run -p jarvis-core --bin jarvis -- doctor
# 4. App
npm run app:dev
```

Pulsa **Hablar con JARVIS**, concede el micrófono y habla. Para terminar, **Finalizar sesión** (apaga el micrófono).

En lugar de `export`, puedes usar el fichero `config.toml` (ver abajo) o, solo en desarrollo, un `.env` en la raíz (ignorado por git). Si usas la variable `OPENAI_API_KEY` en lugar del almacén del sistema, defínela en el entorno del proceso; nunca en ficheros del repositorio.

## Fichero `config.toml`

Ubicación (directorio de configuración de la app, identificador `com.jarvis.desktop`):

- macOS: `~/Library/Application Support/com.jarvis.desktop/config.toml`
- Windows: `%APPDATA%\com.jarvis.desktop\config.toml`
- Linux: `~/.config/com.jarvis.desktop/config.toml`
- O la ruta indicada en `JARVIS_CONFIG_FILE` (también para la CLI).

Usa las mismas claves que las variables de entorno (ver `docs/config.example.toml`). Las variables de entorno tienen prioridad. **No admite claves API.**

## Variables

### Voz (OpenAI Realtime)

| Variable | Por defecto | Descripción |
|---|---|---|
| `OPENAI_API_KEY` | — | Clave de larga duración. Preferible guardarla con `jarvis secrets set openai`. |
| `OPENAI_REALTIME_MODEL` | `gpt-realtime-2.1` | Modelo Realtime. Es el predeterminado del SDK `@openai/agents` 0.18.0. Alternativa más barata: `gpt-realtime-2.1-mini`. Verifícalo con `jarvis doctor`. |
| `OPENAI_REALTIME_VOICE` | `cedar` | Voz de salida. `cedar` es masculina y grave (estilo JARVIS). Opciones oficiales: `cedar`, `marin` (las dos recomendadas, más naturales), `alloy`, `ash`, `ballad`, `coral`, `echo`, `sage`, `shimmer`, `verse`. Todas hablan español; se puede cambiar por sesión desde la interfaz. |
| `OPENAI_REALTIME_TRANSCRIPTION_MODEL` | `gpt-4o-mini-transcribe` | Transcripción de tu voz para mostrarla. `off` la desactiva (ahorra coste; la conversación sigue funcionando). |
| `JARVIS_LANGUAGE` | `es` | Idioma indicado al transcriptor. |
| `OPENAI_REALTIME_VAD` | `semantic_vad` | Detección de turno. `semantic_vad` respeta pausas naturales; `server_vad` usa silencio. |
| `OPENAI_REALTIME_VAD_EAGERNESS` | `auto` | Solo `semantic_vad`: `low` espera más antes de responder; `high` responde antes. |
| `OPENAI_REALTIME_VAD_THRESHOLD` | `0.6` | Solo `server_vad`: más alto = menos sensible al ruido o al eco. |
| `OPENAI_REALTIME_VAD_SILENCE_MS` | `600` | Solo `server_vad`: silencio que cierra el turno. |
| `OPENAI_REALTIME_NOISE_REDUCTION` | `far_field` | `far_field` (micrófono del portátil o de sobremesa), `near_field` (auriculares), `off`. |
| `OPENAI_REALTIME_SECRET_TTL_SECONDS` | `600` | Validez del secreto efímero (10–7200). Solo afecta al establecimiento de la conexión. |

### Vault (Obsidian)

| Variable | Por defecto | Descripción |
|---|---|---|
| `JARVIS_VAULT_PATH` | — | Carpeta raíz de la Vault. Sin ella, la voz funciona pero las herramientas devuelven `vault_not_configured`. |
| `JARVIS_NOTES_FOLDER` | `JARVIS` | Carpeta donde `create_note` crea notas. |
| `JARVIS_TASKS_FILE` | `JARVIS/Tareas.md` | Fichero donde `add_task` añade tareas. |
| `JARVIS_DAILY_FOLDER` | `Diario` | Carpeta de notas diarias. |
| `JARVIS_DAILY_FORMAT` | `%Y-%m-%d` | Formato del nombre de la nota diaria (chrono/strftime). |
| `JARVIS_PROJECTS_FOLDER` | `Proyectos` | Carpeta de proyectos (también cuentan notas con `type: project`). |

Las tareas se leen con la sintaxis de casillas de Obsidian (`- [ ]`, `- [x]`) y el vencimiento con la del plugin Tasks (`📅 2026-10-15`) o `due:2026-10-15`.

### Claude (opcional)

| Variable | Por defecto | Descripción |
|---|---|---|
| `ANTHROPIC_API_KEY` | — | Activa `delegate_to_claude` y la interfaz de texto con lenguaje natural. Requiere reiniciar la app. |
| `ANTHROPIC_MODEL` | `claude-opus-5-5` | Modelo para delegación y texto. |
| `JARVIS_TEXT_PROVIDER` | `auto` | `auto` (Claude si hay clave; si no, comandos), `claude`, `commands`. |

### Política, tiempos y retención

| Variable | Por defecto | Descripción |
|---|---|---|
| `JARVIS_POLICY_WRITE` | `confirm` | `allow`, `confirm` o `deny` para crear notas y añadir tareas. |
| `JARVIS_POLICY_DELEGATE` | `allow` | Igual para enviar contexto a Claude. |
| `JARVIS_TOOL_TIMEOUT_MS` | `15000` | Límite de las lecturas en la Vault. |
| `JARVIS_DELEGATE_TIMEOUT_MS` | `120000` | Límite de la delegación en Claude. |
| `JARVIS_APPROVAL_TIMEOUT_MS` | `120000` | Tiempo para confirmar; al expirar se deniega. |
| `JARVIS_SAVE_TRANSCRIPTS` | `false` | Guarda transcripciones (JSON, directorio de datos de la app). |
| `JARVIS_TRANSCRIPT_RETENTION_DAYS` | `7` | Días antes de purgarlas. |

### Solo pruebas

`JARVIS_OPENAI_BASE_URL` y `JARVIS_ANTHROPIC_BASE_URL` redirigen las llamadas a servidores simulados. No las uses en producción.

## Permisos de micrófono

- **macOS**: la primera vez el sistema pregunta. Si lo denegaste: Ajustes del Sistema → Privacidad y seguridad → Micrófono → JARVIS. En desarrollo (`tauri dev`) el permiso se pide para la terminal o el binario de desarrollo.
- **Windows**: Configuración → Privacidad y seguridad → Micrófono → permitir a las aplicaciones de escritorio.
- **Linux**: la app concede el permiso dentro del WebView; el sistema (PipeWire/PulseAudio) debe tener una fuente de entrada. Si tu WebKitGTK no incluye WebRTC, JARVIS usa automáticamente el transporte WebSocket (se indica en el panel de voz).

Si falta el permiso, el micrófono o la conexión, la interfaz muestra el motivo y el código de error. Sin clave de OpenAI se muestra el **modo de prueba**: prueba local del micrófono e interfaz de texto, sin simular la voz real.

## Producción

```bash
npm run app:build    # instaladores en target/release/bundle/
```

macOS: firma y notarización con las autorizaciones de `src-tauri/Entitlements.plist`.
