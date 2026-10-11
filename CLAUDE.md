# CLAUDE.md

@AGENTS.md

## Notas para Claude Code

- Antes de tocar voz, herramientas o credenciales, lee el documento específico: `docs/voice-realtime.md`, `docs/security.md`, `docs/architecture.md`. No dupliques aquí ese detalle.
- Verifica siempre con `cargo test -p jarvis-core`, `npm test`, `npm run typecheck` y `npm run check:secrets` antes de dar algo por terminado.
- La documentación de OpenAI puede no ser accesible desde el entorno; la fuente de verdad del SDK es el código instalado en `node_modules/@openai/agents-realtime/dist`. La versión está fijada en `package.json`; si la cambias, revisa `docs/voice-realtime.md`.
- Las pruebas de voz real (micrófono, altavoz, credenciales) no son automatizables aquí: distingue en tus informes lo comprobado automáticamente de lo pendiente (ver `docs/testing.md`).
