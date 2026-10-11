# Hoja de ruta

## Hecho (fase actual)

- Voz a voz con OpenAI Realtime (`@openai/agents` 0.18.0): WebRTC con WebSocket de respaldo, VAD semántico, interrupción por voz y por botón, volumen, silencio, selección de voz, modo altavoz.
- Capa de orquestación en Rust con credenciales en el almacén del sistema y secretos efímeros.
- Herramientas de Obsidian: buscar, leer fragmentos, tareas, proyectos, resumen del día, crear nota, añadir tarea.
- Política de permisos con confirmación en pantalla, verificación de escrituras, idempotencia y auditoría.
- Delegación opcional en Claude e interfaz de texto (Claude o modo comandos).
- Reconexión sin duplicar operaciones; modo de prueba honesto; retención configurable.

## Pendiente de validar

- Pruebas de voz reales con credenciales y hardware (ver `testing.md`).
- Calidad de la cancelación de eco en macOS (WKWebView), Windows (WebView2) y Linux (WebSocket de respaldo).

## Próximas fases

1. **Palabra de activación configurable** (local, sin enviar audio hasta que se active). Se conectará a `VoiceController.start()`; mantener el micrófono apagado por defecto y un indicador visible.
2. **Backend remoto autenticado** para distribuir la app: emisión de secretos efímeros por usuario, límites y cuotas (ver `security.md`).
3. **Más herramientas**: completar tareas (modificación con confirmación y diff), calendario, recordatorios.
4. **Búsqueda semántica** en la Vault con índice local incremental, para recuperar mejores fragmentos con menos tokens.
5. **Resumen persistente opcional** de conversaciones en la Vault (con confirmación), como memoria a largo plazo.
6. **Proveedor de texto OpenAI** como alternativa a Claude cuando Realtime no esté disponible.
7. **Panel de ajustes** para política, retención y Vault sin editar ficheros (nunca para claves).
8. Pruebas E2E de la app con `tauri-driver`.
