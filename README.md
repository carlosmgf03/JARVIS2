# JARVIS

Asistente personal de voz en español. Hablas con JARVIS y te responde con voz, con baja latencia; puedes interrumpirle y seguir la conversación sin escribir. Su memoria es tu Vault de Obsidian.

- **Voz**: OpenAI Realtime API (`@openai/agents/realtime`, WebRTC; WebSocket de respaldo).
- **App**: Tauri 2 + React + TypeScript.
- **Capa de orquestación**: Rust (`crates/jarvis-core`): credenciales, secretos efímeros, herramientas validadas, permisos y confirmaciones.
- **Memoria**: Obsidian (buscar, leer fragmentos, tareas, proyectos, resumen del día, crear nota, añadir tarea).
- **Claude** (opcional): especialista para tareas largas y modelo de la interfaz de texto.

## Arranque rápido

```bash
npm ci
cargo run -p jarvis-core --bin jarvis -- secrets set openai   # la clave se guarda en el almacén del sistema
export JARVIS_VAULT_PATH="$HOME/Obsidian/MiVault"
cargo run -p jarvis-core --bin jarvis -- doctor              # comprueba clave, modelo y Vault
npm run app:dev                                              # abre JARVIS
```

Pulsa **Inicializar JARVIS**, concede el micrófono y habla. **Finalizar** apaga el micrófono.

Para usarlo como una app normal (sin terminal): `npm run app:install` la instala en **Aplicaciones** (ver `docs/configuration.md`).

Sin clave de OpenAI, la app arranca en **modo de prueba** (claramente indicado): prueba local del micrófono e interfaz de texto con comandos (`/ayuda`, `/tareas hoy`, `/buscar …`, `/resumen`, `/nota …`, `/tarea …`).

## Privacidad

Mientras la sesión de voz está activa, el audio se envía a OpenAI (no se procesa en local). Solo se envían los fragmentos de la Vault que pide cada herramienta. El audio nunca se guarda; las transcripciones no se guardan salvo que lo actives. Detalle en [`docs/security.md`](docs/security.md).

## Documentación

| Documento | Contenido |
|---|---|
| [`docs/configuration.md`](docs/configuration.md) | Requisitos, variables, voces, permisos de micrófono por sistema |
| [`docs/architecture.md`](docs/architecture.md) | Componentes, flujos (voz, herramientas, confirmación, reconexión, delegación) y decisiones |
| [`docs/voice-realtime.md`](docs/voice-realtime.md) | Detalle del módulo de voz: SDK, sesión, estados, interrupción, eco, reconexión |
| [`docs/security.md`](docs/security.md) | Credenciales, permisos, datos que salen del dispositivo, retención, distribución |
| [`docs/testing.md`](docs/testing.md) | Pruebas automáticas y lista de aceptación |
| [`docs/roadmap.md`](docs/roadmap.md) | Próximas fases (palabra de activación, backend remoto…) |

## Desarrollo

```bash
npm test && npm run typecheck && npm run check:secrets
cargo test -p jarvis-core && cargo clippy --workspace --all-targets
```
