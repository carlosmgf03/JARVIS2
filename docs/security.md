# Seguridad y privacidad

## Credenciales

| Credencial | Dónde vive | Quién la ve |
|---|---|---|
| `OPENAI_API_KEY` (larga duración) | Variable de entorno o almacén del sistema (Llavero de macOS, Administrador de credenciales de Windows, Secret Service en Linux), servicio `com.jarvis.desktop`, usuario `openai`. | Solo el proceso Rust (`jarvis-core`). |
| `ANTHROPIC_API_KEY` (opcional) | Igual, usuario `anthropic`. | Solo el proceso Rust. |
| Secreto efímero Realtime (`ek_…`) | Memoria del WebView durante la conexión. Caduca en `OPENAI_REALTIME_SECRET_TTL_SECONDS` (600 s por defecto). | El frontend, para abrir la sesión. No se guarda ni se registra. |

Reglas aplicadas en el código:

- `Secret` (Rust) no implementa `Display` ni `Serialize`; su `Debug` imprime `Secret(****)`. `EphemeralSession` oculta la clave efímera en `Debug`.
- Los mensajes de error de proveedores pasan por `secrets::redact` (elimina `sk-…`, `sk-ant-…`, `ek_…`).
- `config.toml` **rechaza** cualquier clave `*_API_KEY`: la configuración no secreta y los secretos están separados.
- Vite solo expone variables `VITE_*`; ninguna clave se incrusta en el bundle.
- `npm run check:secrets` busca claves en los ficheros versionados y en `dist/`, y falla si el bundle menciona `OPENAI_API_KEY`/`ANTHROPIC_API_KEY`. Se ejecuta en CI.
- `.gitignore` excluye `.env`, `.env.*` (salvo `.env.example`) y `config.toml`.
- El `.env` local solo se carga en compilaciones de desarrollo (`debug_assertions`).
- Nunca guardes secretos en Obsidian ni en ficheros Markdown.

### Guardar una clave

```bash
cargo run -p jarvis-core --bin jarvis -- secrets set openai      # entrada oculta
cargo run -p jarvis-core --bin jarvis -- secrets set anthropic   # opcional
cargo run -p jarvis-core --bin jarvis -- secrets status          # muestra el origen, nunca el valor
```

La interfaz **no** tiene un campo para pegar claves: así la clave principal nunca pasa por React ni por la consola del WebView.

## Superficie del WebView

- **Capacidades Tauri** (`src-tauri/capabilities/default.json`): `core:default` y los comandos de la app declarados en `build.rs`. Sin plugins de sistema de ficheros, shell, HTTP ni portapapeles.
- **CSP** (`tauri.conf.json`): `script-src 'self'`; `connect-src` limitado a IPC y `api.openai.com` (HTTPS y WSS); sin `object-src`; sin marcos.
- **Micrófono**:
  - macOS: `NSMicrophoneUsageDescription` (`src-tauri/Info.plist`) y la autorización `com.apple.security.device.audio-input` (`Entitlements.plist`).
  - Windows (WebView2): el sistema muestra el aviso de permiso la primera vez.
  - Linux (WebKitGTK): se habilitan `enable-media-stream`/`enable-webrtc` y se concede **solo** audio (nunca vídeo) al contenido propio de la app. El micrófono solo se abre cuando el usuario inicia la conversación.

## Herramientas y política

- Catálogo cerrado (`crates/jarvis-core/src/tools/`). No hay herramientas de shell, red arbitraria ni borrado.
- Validación estricta antes de cualquier efecto: nombre conocido, objeto JSON, sin campos adicionales, tipos, longitudes, enumeraciones, fechas `AAAA-MM-DD`, rutas relativas seguras.
- Rutas de la Vault: sin absolutas, sin `..`, sin segmentos ocultos (`.obsidian`, `.git`, `.trash`), solo `.md`, y comprobación tras `canonicalize` de que siguen dentro de la Vault (bloquea enlaces simbólicos que escapan).
- Política (`JARVIS_POLICY_WRITE`, `JARVIS_POLICY_DELEGATE`: `allow` | `confirm` | `deny`):
  - `read` (buscar, leer, tareas, proyectos, resumen): directo.
  - `write` (crear nota, añadir tarea): **confirmación por defecto**. Nunca sobrescribe (`create_new`) ni borra; añadir tarea solo agrega una línea.
  - `delegate` (enviar a Claude): permitido por defecto porque el usuario configuró la clave; cámbialo a `confirm` si quieres aprobar cada envío.
- La política es idéntica para voz, texto y CLI. El origen nunca relaja permisos.
- Confirmaciones: los argumentos se quedan en Rust; la interfaz solo aprueba o deniega un id. Expiran (120 s por defecto) como denegación.
- Éxito solo tras verificación: las escrituras se releen; `ok: true` solo con `verified: true`. Las escrituras no se cortan por tiempo para no informar de un fallo falso.
- Auditoría (`audit.log` en el directorio de datos de la app): herramienta, origen, estado real, código, duración y ruta afectada. **Sin argumentos ni contenido.**

## Qué datos salen del dispositivo

| Cuándo | Qué | A quién |
|---|---|---|
| Sesión de voz activa | Audio del micrófono; texto escrito durante la sesión; resultados de herramientas (fragmentos de notas, tareas) | OpenAI |
| Transcripción activada (por defecto) | El audio de entrada se transcribe en OpenAI con `OPENAI_REALTIME_TRANSCRIPTION_MODEL` | OpenAI |
| `delegate_to_claude` | Tarea, contexto mínimo y fragmentos de hasta 3 notas | Anthropic |
| Interfaz de texto con Claude | Mensajes, historial compacto y resultados de herramientas | Anthropic |
| Modo comandos / pruebas locales de micrófono | Nada | — |

**El procesamiento de voz no es local.** La interfaz lo indica y las instrucciones de JARVIS le impiden afirmar lo contrario. Tracing de OpenAI desactivado (`tracingDisabled: true`).

## Retención

- Audio: nunca se guarda (`historyStoreAudio: false`).
- Transcripciones: solo en memoria por defecto. `JARVIS_SAVE_TRANSCRIPTS=true` las guarda como JSON en el directorio de datos de la app (no en la Vault) y se purgan tras `JARVIS_TRANSCRIPT_RETENTION_DAYS` (7).
- Métricas de uso (`usage.jsonl`): solo recuentos de tokens, peticiones y duración.

## Distribución a otros usuarios

Esta versión está pensada para uso personal: la clave vive en el equipo del usuario. Para distribuirla:

1. Mover la emisión de secretos efímeros a un **backend remoto autenticado** (reutilizando `jarvis-core::realtime`), con login por usuario.
2. Aplicar límites de uso y cuotas por usuario, y registro de consumo.
3. La app de escritorio pediría el secreto efímero a ese backend, nunca tendría la clave de OpenAI.
4. Firmar y notarizar los binarios (macOS) y revisar la CSP para el dominio del backend.

## Modelo de amenazas (resumen)

| Amenaza | Mitigación |
|---|---|
| El modelo pide una operación no prevista | Catálogo cerrado + validación estricta + política. |
| El modelo inventa argumentos peligrosos (rutas) | Validación de rutas y `canonicalize`. |
| El modelo dice que algo se hizo sin hacerlo | Resultado verificado y auditoría con el estado real; instrucciones explícitas en los errores. |
| Doble ejecución tras reconexión | Deduplicación por `call_id` + resumen de operaciones hechas. |
| Fuga de claves | Claves solo en Rust; efímeras en el frontend; redacción; escaneo en CI. |
| Eco del altavoz interpretado como usuario | AEC del navegador, `far_field`, VAD semántico, modo altavoz opcional. |
| WebView comprometido | Fuera del alcance de esta fase; mitigado por CSP y capacidades mínimas. |
