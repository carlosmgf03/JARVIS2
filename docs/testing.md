# Pruebas y aceptación

## Automáticas

```bash
cargo test -p jarvis-core          # 57 pruebas: Vault, validación, política, confirmaciones, idempotencia, Internet, correo IMAP,
                                   # secretos efímeros y errores (OpenAI simulado), delegación y texto (Claude simulado)
npm test                           # 15 pruebas: máquina de estados, reintentos, herramientas, controlador con sesión simulada
npm run typecheck
npm run check:secrets              # sin claves en el repositorio ni en dist/
cargo clippy --workspace --all-targets
```

Todo se ejecuta en CI (`.github/workflows/ci.yml`).

## Lista de aceptación

Estado al entregar este módulo. ✅ comprobado · 🟡 comprobado parcialmente · ⏳ pendiente (requiere credenciales o hardware reales).

| Criterio | Estado | Cómo |
|---|---|---|
| Conexión real a una sesión Realtime con credenciales válidas | ⏳ | Sin clave de OpenAI ni acceso de red a OpenAI en el entorno de desarrollo. Verificado: emisión del secreto contra un servidor simulado (cabecera, cuerpo, modelo, voz, VAD), y en la app real (Tauri bajo Xvfb, micrófono virtual de PulseAudio) el flujo completo hasta el intento de conexión: micrófono → secreto efímero → transporte → error de red claro → 4 reintentos con secreto nuevo → micrófono apagado. |
| Entrada de audio y respuesta hablada en español | ⏳ | Requiere clave y altavoz/micrófono. Ver «Prueba manual». |
| Interrupción de la voz generada | 🟡 | Automático: botón Detener → `session.interrupt()`; voz del usuario → `speech_started` corta la reproducción. Pendiente con audio real. |
| Control del micrófono y cierre de sesión | ✅ | Pruebas del controlador (pistas detenidas, sesión cerrada, métricas registradas); en la app real, el micrófono queda apagado tras error o cierre. |
| Desconexiones sin duplicar operaciones | ✅ | `repeated_call_id_executes_once` (Rust, incluidas llamadas concurrentes) y «reconecta tras una caída…» (frontend: sesión nueva, herramientas una vez, eventos antiguos ignorados, resumen con operaciones). |
| Herramienta segura de Obsidian real | ✅ | Pruebas con Vault en disco; en la app real `/resumen` (daily_summary) y `/tarea` sobre una Vault de ejemplo; CLI `jarvis tool search_notes`. |
| Rechazo de argumentos inválidos | ✅ | `rejects_invalid_arguments`, `invalid_calls_are_rejected_and_audited`, `symlinks_cannot_escape_the_vault`; CLI con `../../etc/passwd` → `permission_denied`. |
| Aprobación de una operación de riesgo | ✅ | Pruebas de aprobar/denegar/expirar; en la app real, diálogo «Añadir la tarea…» → Autorizar → tarea escrita y verificada; en la CLI, denegar → `approval_denied` sin escribir. |
| Sin claves en el frontend ni en el repositorio | ✅ | `npm run check:secrets` (repo + `dist/`), prueba de que la respuesta a la interfaz no contiene la clave larga, `Debug` oculta secretos. |
| Interfaz de texto sin Realtime | ✅ | `text_interface_works_without_any_credentials`; en la app real, modo comandos sin ninguna clave. Con Claude: `text_interface_with_claude_runs_tools` (simulado). |
| Hora, tiempo, tráfico, vuelos, búsqueda, páginas y correo | 🟡 | Automático con servidores simulados: tráfico de Bogotá (alias casa, ruta con retraso, incidentes ordenados por gravedad), vuelos (días cercanos, el más económico, fechas pasadas y códigos inválidos), hora de cualquier país (país → capital, zonas IANA, aviso de países con varias zonas), Open-Meteo, API Responses, páginas (texto, redirecciones, bloqueo de direcciones locales), abrir URL solo tras confirmación, IMAP de solo lectura (EXAMINE + BODY.PEEK, sin comandos de escritura), contraseña errónea. Pendiente con servicios reales: el entorno de desarrollo bloquea Open-Meteo, OpenAI y servidores de correo. |
| Modo producción (app instalada) | 🟡 | La app compilada con la interfaz embebida arranca, lee `config.toml` sin variables de entorno y respeta la CSP (comprobado en Linux). Pendiente: instalador en un Mac real. |
| Documentación de comandos, permisos y arranque | ✅ | `README.md`, `docs/configuration.md`, `docs/security.md`. |

## Prueba manual con credenciales (pendiente)

1. `jarvis secrets set openai` y `export JARVIS_VAULT_PATH=…`.
2. `jarvis doctor` → «Modelo Realtime: disponible para esta cuenta».
3. `npm run app:dev` → **Hablar con JARVIS** → conceder micrófono.
4. Decir «Hola JARVIS, ¿qué tengo hoy?» → debe usar `daily_summary` y responder en español, breve.
5. Mientras habla, decir «espera» → debe callar y atender. Repetir con el botón **Detener respuesta**.
6. «Apúntame llamar a Marta mañana» → diálogo de confirmación → **Autorizar** → la tarea aparece en `JARVIS/Tareas.md` y en «Actividad verificada». Repetir con **Cancelar**: JARVIS debe decir que no se hizo.
7. Desconectar la red unos segundos → «Reconectando…» → al volver, continuar sin repetir la tarea.
8. **Finalizar sesión** → el indicador del sistema operativo debe mostrar el micrófono liberado.
9. Sin auriculares, comprobar que la voz de JARVIS no dispara turnos; si lo hace, activar «Modo altavoz».
