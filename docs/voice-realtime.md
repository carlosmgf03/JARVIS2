# Módulo de voz: OpenAI Realtime

Detalle de implementación de la conversación por voz. Para la visión general, ver `architecture.md`.

## Versiones verificadas

| Pieza | Valor | Cómo se verificó |
|---|---|---|
| SDK | `@openai/agents` **0.18.0** (`@openai/agents/realtime`) | Fijado en `package.json`. API revisada en el código del paquete instalado (`RealtimeAgent`, `RealtimeSession`, `OpenAIRealtimeWebRTC`, `OpenAIRealtimeWebSocket`, `tool`). |
| Modelo por defecto | `gpt-realtime-2.1` | `DEFAULT_OPENAI_REALTIME_MODEL` del SDK 0.18.0. La disponibilidad para tu cuenta se comprueba con `jarvis doctor` (`GET /v1/models/{modelo}`). |
| Secretos efímeros | `POST /v1/realtime/client_secrets` → `{ value: "ek_…", expires_at }` | Cuerpo GA: `{ expires_after, session: { type: "realtime", model, audio: {…} } }`. |
| WebRTC | `POST /v1/realtime/calls` (SDP) | Lo hace el SDK con el secreto efímero. |

> Durante la implementación, los dominios de documentación de OpenAI no eran accesibles desde el entorno de desarrollo, así que la fuente de verdad fue el SDK publicado. Si actualizas el SDK, revisa esta tabla, `config.rs` (`DEFAULT_REALTIME_MODEL`, `REALTIME_VOICES`) y `realtime.rs` (cuerpo de `client_secrets`).

## Ficheros

| Fichero | Función |
|---|---|
| `crates/jarvis-core/src/realtime.rs` | Construye la configuración de sesión y emite el secreto efímero. |
| `crates/jarvis-core/src/persona.rs` | Instrucciones de personalidad (fuente única para voz y texto). |
| `src/voice/controller.ts` | Ciclo de vida de la sesión, eventos, controles, reconexión. |
| `src/voice/machine.ts` | Máquina de estados de la interfaz. |
| `src/voice/tools.ts` | Envuelve el catálogo de Rust como herramientas del SDK; ejecución delegada en Rust. |
| `src/voice/pcmAudio.ts`, `public/pcm-capture-worklet.js` | Audio PCM16 24 kHz para el transporte WebSocket de respaldo. |
| `src/voice/transcript.ts` | Transcripción y resumen de continuidad. |

## Configuración de sesión

Se envía igual al crear el secreto (servidor) y en el `session.update` inicial del SDK:

- `output_modalities: ["audio"]`, PCM 24 kHz.
- `audio.output.voice`: la elegida (por defecto `marin`).
- `audio.input.turn_detection`: `semantic_vad` (`eagerness: auto`) o `server_vad` (umbral 0.6, silencio 600 ms), siempre con `create_response: true` e `interrupt_response: true`.
- `audio.input.noise_reduction`: `far_field` por defecto.
- `audio.input.transcription`: `gpt-4o-mini-transcribe` con `language: "es"` (desactivable).
- `tracingDisabled: true`, `historyStoreAudio: false`.

## Personalidad

`persona.rs`: profesional, inteligente, serena y resolutiva; español natural; respuestas de 1–3 frases ampliables; cede la palabra al ser interrumpida; admite cuando no sabe; no lee código, JSON, rutas, registros ni documentos largos salvo petición; consulta la Vault antes de afirmar nada sobre el usuario; solo da por hecha una acción con `ok: true`; recuerda que el audio se procesa en OpenAI. No se clona ni imita ninguna voz real: solo voces oficiales.

## Estados de la interfaz

| Estado | Evento que lo provoca |
|---|---|
| Desconectado | Inicial / `stop()` |
| Solicitando micrófono | Pulsar «Hablar con JARVIS» |
| Conectando | Micrófono concedido; emisión del secreto y conexión |
| Escuchando | Conexión establecida; fin de la respuesta |
| Te escucho | `input_audio_buffer.speech_started` |
| Pensando | `speech_stopped`, `response.created`, herramienta en curso |
| Hablando | `output_audio_buffer.started` (WebRTC) o primer audio (WebSocket) |
| Reconectando | `connection_change: disconnected` o fallo transitorio |
| Error | Error no recuperable o reintentos agotados |

Indicadores adicionales: micrófono activo/silenciado, herramienta en curso y «esperando tu confirmación».

## Interrupción y eco

- **Por voz**: el VAD del servidor detecta la voz del usuario e interrumpe la respuesta (`interrupt_response`). En WebRTC el servidor corta el audio; en WebSocket el SDK emite `audio_interrupted` y JARVIS detiene la reproducción local al instante.
- **Botón «Detener respuesta»**: `session.interrupt()` (cancela la respuesta y vacía el búfer de salida).
- **Eco del altavoz**: `getUserMedia` con `echoCancellation`, `noiseSuppression` y `autoGainControl`; reducción de ruido `far_field`; VAD semántico. Si aun así el altavoz dispara turnos (altavoces potentes sin auriculares), activa **Modo altavoz**: silencia el micrófono mientras JARVIS habla (la interrupción por voz deja de funcionar; el botón sí). Con `server_vad` puedes subir `OPENAI_REALTIME_VAD_THRESHOLD`.

## Controles

Iniciar · Detener respuesta · Silenciar/activar micrófono · Volumen y silenciar a JARVIS · Selección de voz (antes de iniciar) · Modo altavoz · Mostrar/ocultar transcripción · Finalizar sesión (cierra la sesión, detiene las pistas de audio y libera listeners). Si el sistema bloquea la reproducción automática, aparece «Activar audio».

## Reconexión

- Reintentos: 4, con espera exponencial y algo de aleatoriedad (~1, 2, 4, 8 s). Solo para errores transitorios (`network_error`, `provider_error`, `rate_limited`, `timeout`, `connection_lost`).
- No se reintenta: falta de clave, clave inválida, modelo no disponible, falta de micrófono o permiso.
- Cada intento emite un secreto nuevo, crea un agente nuevo (herramientas registradas una sola vez) y cierra el anterior. Los eventos de sesiones antiguas se descartan por número de generación.
- La nueva sesión recibe en sus instrucciones los últimos turnos y la lista de operaciones ya ejecutadas. El backend deduplica por `call_id`.
- La transcripción visible se conserva entre reconexiones.

## Transporte WebSocket de respaldo

Se usa si `RTCPeerConnection` no existe en el WebView. En la verificación con WebKitGTK 2.52 (Ubuntu 24.04) **no había WebRTC** y JARVIS pasó automáticamente a WebSocket. El SDK conecta a `wss://api.openai.com/v1/realtime` con el secreto efímero; el audio se captura con un `AudioWorklet` (contexto a 24 kHz, bloques de 100 ms en PCM16) y se reproduce programando `AudioBufferSourceNode`s con un `GainNode` para el volumen. La cancelación de eco depende de la del WebView.

## Coste y uso

Al finalizar cada sesión se registran localmente (`usage.jsonl`) los recuentos de `session.usage` (peticiones y tokens) y la duración; la interfaz muestra el total de 30 días. Para ahorrar: modelo `gpt-realtime-2.1-mini`, transcripción `off`, respuestas breves (por defecto en la personalidad) y herramientas que devuelven fragmentos acotados.

## Errores visibles

`missing_api_key`, `invalid_api_key`, `rate_limited`, `quota_exceeded`, `model_unavailable`, `provider_error`, `network_error`, `microphone_denied`, `microphone_unavailable`, `playback_failed`, `connection_lost`, y los de herramientas (`timeout`, `vault_io`, `vault_not_configured`, `approval_denied`, …). Cada uno con mensaje en español y, cuando procede, botón de reintento.
