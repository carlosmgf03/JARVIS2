# AGENTS.md — JARVIS

Asistente personal de voz en español. Conversación voz a voz con **OpenAI Realtime** (WebRTC), memoria persistente en **Obsidian**, **Claude** como especialista opcional y una interfaz de texto que comparte herramientas y memoria.

## Mapa del repositorio

| Ruta | Qué es |
|---|---|
| `crates/jarvis-core/` | Capa de orquestación en Rust (sin Tauri): herramientas, validación, política de permisos, confirmaciones, Vault, credenciales, secretos efímeros de Realtime, Claude, interfaz de texto, auditoría. CLI `jarvis`. |
| `src-tauri/` | App de escritorio Tauri 2. Capa fina: comandos → `jarvis-core`. Permisos de micrófono por SO. |
| `src/` | Interfaz React + TypeScript. `src/voice/` = sesión Realtime (`@openai/agents/realtime`). |
| `docs/` | Arquitectura, seguridad, configuración, voz, pruebas y hoja de ruta. |

## Comandos

```bash
npm ci                          # dependencias del frontend
npm test && npm run typecheck   # pruebas y tipos del frontend
npm run check:secrets           # sin claves en el repo ni en dist/
cargo test -p jarvis-core       # pruebas de la capa de orquestación
cargo clippy --workspace --all-targets
npm run app:dev                 # app de escritorio (necesita dependencias de Tauri del SO)
cargo run -p jarvis-core --bin jarvis -- doctor
```

## Invariantes (no romper)

1. **Las claves de larga duración nunca llegan al frontend**, al repositorio, a la Vault, a ficheros distribuidos ni a los logs. Solo `jarvis-core` las lee (entorno o almacén del sistema). La interfaz recibe únicamente secretos efímeros `ek_…`.
2. **Realtime es el motor de voz.** No sustituirlo por cadenas STT → texto → TTS ni por otro proveedor sin decisión explícita.
3. **El catálogo de herramientas vive en `crates/jarvis-core/src/tools/`.** Voz y texto lo obtienen de ahí; no definir herramientas en el frontend.
4. **Toda herramienta se ejecuta en `Executor::invoke`**: validación estricta → política → confirmación → ejecución → verificación → auditoría. Nada de comandos arbitrarios.
5. **Solo se comunica éxito tras verificar.** Escrituras releídas; `ok: true` solo si se verificó.
6. **Escrituras en la Vault: crear o añadir, nunca sobrescribir ni borrar.** Rutas relativas, sin `..`, sin carpetas ocultas, dentro de la raíz tras resolver enlaces.
7. **Idempotencia por `call_id`**: una reconexión nunca repite una operación.
8. **Privacidad**: no enviar la Vault completa; no guardar audio; transcripciones desactivadas por defecto; no registrar contenido personal.
9. **Modo de prueba honesto**: sin credenciales se muestra claramente; nunca simular una conversación de voz real.

## Convenciones

- Textos de usuario, mensajes de error y documentación en español.
- Errores tipados (`ErrorCode`) con mensaje para el usuario; nunca incluir secretos (usa `secrets::redact`).
- Cambios en voz, seguridad o herramientas: actualiza el documento correspondiente de `docs/` y añade pruebas.
