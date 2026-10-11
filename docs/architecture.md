# Arquitectura

> Decisión vigente: **OpenAI Realtime API es el sistema principal de conversación por voz.** Sustituye cualquier diseño anterior basado en WhisperKit + ElevenLabs, Whisper + TTS o cadenas independientes de transcripción y síntesis.

## Vista general

```
┌──────────────────────────── App de escritorio (Tauri 2) ────────────────────────────┐
│                                                                                      │
│  WebView — React + TypeScript (src/)                                                 │
│  ┌──────────────────────────────┐   ┌──────────────────┐   ┌──────────────────────┐  │
│  │ VoiceController              │   │ TextChat         │   │ ApprovalDialog       │  │
│  │ RealtimeAgent/RealtimeSession│   │                  │   │ (confirmaciones)     │  │
│  │ @openai/agents/realtime      │   │                  │   │                      │  │
│  └──────┬───────────────▲───────┘   └────────┬─────────┘   └──────────▲───────────┘  │
│         │ audio WebRTC  │ eventos            │                        │ eventos       │
│         │ (o WebSocket) │                    │ invoke()               │               │
│  ═══════╪═══════════════╪════════════════════╪════════════════════════╪═══ IPC Tauri │
│         │               │                    ▼                        │               │
│  Rust   │   jarvis-core (crates/jarvis-core) — capa de orquestación de confianza       │
│         │   ┌────────────┐ ┌──────────┐ ┌───────────┐ ┌───────────┐ ┌─────────────┐   │
│         │   │ Credentials│ │ Executor │ │ Approvals │ │ TextAgent │ │ AuditLog    │   │
│         │   │ (env/llav.)│ │ validar→ │ │ broker    │ │ Claude o  │ │ estado real │   │
│         │   └─────┬──────┘ │ política→│ └───────────┘ │ comandos  │ └─────────────┘   │
│         │         │        │ ejecutar→│               └─────┬─────┘                   │
│         │         │        │ verificar│                     │                         │
│         │         │        └────┬─────┘                     │                         │
│         │         │             ▼                           │                         │
│         │         │       Vault (Obsidian, .md)             │                         │
└─────────┼─────────┼─────────────────────────────────────────┼─────────────────────────┘
          │         │ POST /v1/realtime/client_secrets        │ POST /v1/messages
          ▼         ▼                                         ▼
     OpenAI Realtime (voz, sesión)                      Anthropic (opcional)
```

## Responsabilidades

| Componente | Hace | No hace |
|---|---|---|
| **Realtime** (OpenAI) | Escucha, detecta turnos, interpreta la petición, decide herramientas y responde con voz. Mantiene el estado de la sesión. | No es memoria persistente. No ejecuta herramientas. |
| **Capa de orquestación** (`jarvis-core`) | Credenciales, secretos efímeros, catálogo de herramientas, validación, política de permisos, confirmaciones, ejecución, verificación, auditoría, retención. | No conversa por voz. |
| **Obsidian** (Vault) | Memoria personal y conocimiento persistente. | — |
| **Claude** (opcional) | Especialista para razonamiento extenso, análisis documental o programación (`delegate_to_claude`), y modelo de la interfaz de texto. | No comparte memoria ni historial con Realtime: el contexto se envía explícitamente y acotado. |
| **Interfaz de texto** | Mismas herramientas, misma política y misma Vault. Claude si está configurado; si no, modo comandos sin credenciales. | — |

## Flujos

### Turno de voz
1. El usuario pulsa **Hablar con JARVIS** (micrófono apagado por defecto).
2. `getUserMedia` con cancelación de eco, supresión de ruido y control de ganancia.
3. Rust crea un secreto efímero (`POST /v1/realtime/client_secrets`) con modelo, voz, transcripción, VAD y reducción de ruido, y devuelve también instrucciones (personalidad) y el catálogo de herramientas.
4. El frontend crea `RealtimeAgent` + `RealtimeSession` y conecta por **WebRTC** (o **WebSocket** si el WebView no tiene WebRTC, p. ej. WebKitGTK en Linux).
5. La detección de turno (`semantic_vad` por defecto) y la interrupción (`interrupt_response`) ocurren en el servidor; la interfaz refleja los estados: conectando → escuchando → te escucho → pensando → hablando.

### Llamada a herramienta
1. Realtime emite una llamada a función → el SDK ejecuta el `execute` del wrapper (`src/voice/tools.ts`).
2. El wrapper invoca `tool_invoke(call_id, name, args)` en Rust. **No hay lógica de herramientas en el frontend.**
3. `Executor::invoke`: idempotencia por `call_id` → validación estricta (esquema, rangos, rutas) → política (`read` directo; `write` confirmar por defecto; `delegate` configurable) → confirmación en pantalla si procede → ejecución con límite de tiempo → verificación (relectura) → auditoría.
4. El resultado estructurado vuelve al modelo con `ok`, `verified` y, si falla, una instrucción explícita para no afirmar éxito.

### Confirmación
`ApprovalBroker` guarda los argumentos en Rust, emite `jarvis://approval` y espera la decisión (o expira, 120 s por defecto). La interfaz solo envía `aprobar/denegar` para un id: no puede alterar la operación aprobada. Igual para voz, texto y CLI.

### Reconexión
Ante `connection_change: disconnected` o un fallo transitorio: se cierra la sesión anterior (sus eventos se ignoran por número de generación), se reintenta con espera exponencial (1, 2, 4, 8 s; máx. 4 intentos), se emite un **nuevo** secreto y se crea un agente nuevo con el catálogo una sola vez. La nueva sesión recibe un resumen de la conversación y **la lista de operaciones ya ejecutadas** para que no las repita. Además, el backend deduplica por `call_id`.

### Delegación en Claude
`delegate_to_claude` solo existe si hay clave de Anthropic. Envía la tarea, el contexto mínimo y, como mucho, fragmentos relevantes de 3 notas (no notas completas). Claude responde con una conclusión breve apta para voz y el detalle; Realtime la comunica de forma natural.

### Texto ↔ voz
- Con sesión de voz activa, lo escrito se envía a Realtime (`sendMessage`).
- Sin sesión, va a `TextAgent` (Claude con herramientas o modo comandos), con un resumen de los últimos turnos de voz.
- Al iniciar la voz se pasa un resumen de los últimos turnos de texto.

## Decisiones

| Decisión | Motivo |
|---|---|
| Tauri 2 + React + TS | App de escritorio ligera; el proceso Rust es el lugar de confianza para claves y Vault. |
| `@openai/agents` 0.18.0 (`RealtimeAgent`, `RealtimeSession`) | SDK oficial; gestiona WebRTC, herramientas, interrupciones e historial. Versión fijada y verificada. |
| WebRTC principal, WebSocket de respaldo | WebRTC da la menor latencia y AEC del navegador. WebKitGTK de muchas distribuciones Linux no incluye WebRTC (comprobado con 2.52 en Ubuntu 24.04): se usa el transporte WebSocket oficial del SDK con captura/reproducción PCM16 propia (`src/voice/pcmAudio.ts`). |
| Catálogo de herramientas en Rust | Fuente única para voz y texto; validación y política en la capa de confianza. |
| Esquemas `strict: false` en el SDK | La validación autoritativa es la de Rust (rechaza campos desconocidos, rangos y rutas inseguras). |
| `jarvis-core` sin dependencia de Tauri | Testeable con `cargo test`, reutilizable por la CLI y por un futuro backend remoto. |
| Modelo y voz por configuración | `OPENAI_REALTIME_MODEL`, `OPENAI_REALTIME_VOICE`; `jarvis doctor` comprueba que el modelo está habilitado para la cuenta. |

## Preparado para palabra de activación (no implementado)

El micrófono solo se abre con `VoiceController.start()`. Una futura palabra de activación local se añadiría como un detector independiente que, al dispararse, llama a `start()`. No hay escucha permanente en esta fase. Ver `docs/roadmap.md`.
