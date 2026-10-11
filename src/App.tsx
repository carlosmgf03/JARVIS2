import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { ApprovalDialog } from './components/ApprovalDialog';
import { CloseIcon, MicIcon, MicOffIcon, PanelIcon, PowerIcon, SpeakerIcon, StopIcon } from './components/Icons';
import { MicTest } from './components/MicTest';
import { Reactor } from './components/Reactor';
import { TextChat } from './components/TextChat';
import { getBackend } from './lib/backend';
import type { ApprovalRequest, AppStatus, AuditEntry, TranscriptEntry, UsageSummary } from './lib/types';
import { VoiceController } from './voice/controller';
import { isBusy, isSessionActive, PHASE_LABEL, TOOL_LABEL } from './voice/machine';
import { carryOverSummary } from './voice/transcript';

function usePref<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(`jarvis.${key}`);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (nv: T) => {
      setV(nv);
      try {
        localStorage.setItem(`jarvis.${key}`, JSON.stringify(nv));
      } catch {
        /* preferencias opcionales */
      }
    },
    [key],
  );
  return [v, set];
}

export default function App() {
  const backend = useMemo(() => getBackend(), []);
  const controller = useMemo(() => new VoiceController({ backend }), [backend]);
  const snap = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const levels = useCallback(() => controller.levels(), [controller]);
  const [app, setApp] = useState<AppStatus | null>(null);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [panelOpen, setPanelOpen] = useState(false);
  const [voice, setVoice] = usePref<string | null>('voice', null);
  const [halfDuplex, setHalfDuplex] = usePref('halfDuplex', false);
  const [showTranscript, setShowTranscript] = usePref('showTranscript', true);
  const [autoStart, setAutoStart] = usePref('autoStart', false);
  // Últimos turnos escritos: se pasan como contexto al iniciar la voz.
  const [textHistory, setTextHistory] = useState<TranscriptEntry[]>([]);

  const refreshActivity = useCallback(() => {
    void backend.auditRecent(15).then(setAudit);
    void backend.usageSummary().then(setUsage);
  }, [backend]);

  useEffect(() => {
    void backend.status().then(setApp);
    void backend.pendingApprovals().then((p) => setApprovals((a) => [...a, ...p.filter((r) => !a.some((x) => x.id === r.id))]));
    refreshActivity();
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void backend
      .onApproval((ev) => {
        if (ev.kind === 'requested') setApprovals((a) => (a.some((r) => r.id === ev.request.id) ? a : [...a, ev.request]));
        else {
          setApprovals((a) => a.filter((r) => r.id !== ev.id));
          refreshActivity();
        }
      })
      .then((u) => {
        // Si el efecto ya se limpió (StrictMode), se elimina el listener al llegar.
        if (disposed) u();
        else unlisten = u;
      });
    return () => {
      disposed = true;
      unlisten?.();
      void controller.stop();
    };
  }, [backend, controller, refreshActivity]);

  // Avisar a la máquina de estados (y a la interfaz) si hay confirmaciones pendientes.
  useEffect(() => {
    controller.setApprovalPending(approvals.length > 0);
  }, [approvals.length, controller]);

  useEffect(() => {
    if (!snap.state.activeTool) refreshActivity();
  }, [snap.state.activeTool, refreshActivity]);

  const status = app?.status;
  const voiceAvailable = Boolean(status?.voice_available);
  const { state } = snap;
  const active = isSessionActive(state);
  const busy = isBusy(state);
  const live = active || busy;
  const micLive = (active || state.phase === 'reconnecting') && !state.micMuted;
  const currentVoice = voice ?? status?.default_voice ?? 'cedar';

  const start = () =>
    void controller.start({
      voice: currentVoice,
      carryOver: carryOverSummary(textHistory),
      halfDuplex,
      saveTranscript: Boolean(status?.save_transcripts),
    });

  // «Empezar a escuchar al abrir»: una sola vez, cuando ya se conoce el estado.
  const [autoStarted, setAutoStarted] = useState(false);
  useEffect(() => {
    if (autoStart && !autoStarted && voiceAvailable && state.phase === 'idle') {
      setAutoStarted(true);
      start();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoStart, autoStarted, voiceAvailable]);

  let statusText = PHASE_LABEL[state.phase];
  if (state.awaitingApproval) statusText = 'Esperando tu confirmación';
  else if (state.activeTool) statusText = TOOL_LABEL[state.activeTool] ?? 'Procesando';
  else if (state.phase === 'reconnecting') statusText = `Reconectando · intento ${state.reconnectAttempt}`;
  else if (state.micMuted && active) statusText = 'Micrófono silenciado';

  const recent = snap.transcript.slice(-4);

  return (
    <main className={`hud ${live ? 'live' : 'rest'}`}>
      <header className="hud-top">
        <div className={`mic-indicator ${micLive ? 'on' : ''}`} role="status" aria-live="polite">
          <span className="dot" aria-hidden="true" />
          {micLive ? 'MIC ACTIVO · audio enviado a OpenAI' : 'MIC APAGADO'}
        </div>
        <button className="icon-btn" onClick={() => setPanelOpen(true)} aria-label="Abrir panel de texto y ajustes" title="Texto y ajustes">
          <PanelIcon />
        </button>
      </header>

      <Reactor phase={state.phase} levels={levels} compact={!live} />

      {!live ? (
        <section className="splash" aria-label="Inicio">
          <p className="kicker">Sistema operativo de voz</p>
          <h1 className="title">JARVIS</h1>
          <p className="tagline">Inteligencia, lista cuando tú lo estés.</p>

          {state.error && (
            <div className="hud-alert" role="alert">
              {state.error.message}
              <small>código: {state.error.code}</small>
            </div>
          )}

          <button className="init-btn" disabled={!voiceAvailable} onClick={start}>
            <MicIcon />
            {state.phase === 'error' ? 'Reintentar' : 'Inicializar JARVIS'}
          </button>
          <p className="fine">El micrófono se solicita al inicializar · el audio se procesa en OpenAI</p>

          {app?.config_error && <div className="hud-alert">Configuración no válida: {app.config_error}</div>}
          {status && !voiceAvailable && (
            <div className="test-mode" role="note">
              <strong>MODO DE PRUEBA — la conversación de voz real no está disponible</strong>
              {status.limitations.map((l) => (
                <span key={l}>{l}</span>
              ))}
              <span>Puedes probar el micrófono aquí (no se envía audio) y usar el texto desde el panel ☰.</span>
              <MicTest />
            </div>
          )}
          {status && voiceAvailable && status.limitations.length > 0 && (
            <p className="fine warn">{status.limitations.join(' ')}</p>
          )}
        </section>
      ) : (
        <>
          <p className="status-label" role="status" aria-live="polite">
            {statusText}
          </p>
          {snap.notice && (
            <div className="hud-alert floating" role="alert">
              {snap.notice.message}
              <button className="link" onClick={() => controller.dismissNotice()}>
                Cerrar
              </button>
            </div>
          )}
          {snap.playbackBlocked && (
            <div className="hud-alert floating">
              El sistema bloqueó el audio de JARVIS.
              <button className="link" onClick={() => controller.resumePlayback()}>
                Activar audio
              </button>
            </div>
          )}

          <nav className="controls" aria-label="Controles de voz">
            <button
              className={`ctl ${state.micMuted ? 'off' : ''}`}
              disabled={!active}
              onClick={() => controller.setMicMuted(!state.micMuted)}
              aria-label={state.micMuted ? 'Activar micrófono' : 'Silenciar micrófono'}
            >
              {state.micMuted ? <MicOffIcon /> : <MicIcon />}
              <span>{state.micMuted ? 'Activar' : 'Silenciar'}</span>
            </button>
            <button
              className="ctl"
              disabled={!active || !(state.phase === 'speaking' || state.phase === 'processing')}
              onClick={() => controller.interrupt()}
              aria-label="Detener respuesta"
            >
              <StopIcon />
              <span>Detener</span>
            </button>
            <div className="ctl volume">
              <button className="bare" onClick={() => controller.setOutputMuted(!snap.outputMuted)} aria-label={snap.outputMuted ? 'Activar voz de JARVIS' : 'Silenciar voz de JARVIS'}>
                <SpeakerIcon muted={snap.outputMuted} />
              </button>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={snap.volume}
                onChange={(e) => controller.setVolume(Number(e.target.value))}
                aria-label="Volumen de JARVIS"
              />
            </div>
            <button className="ctl end" onClick={() => void controller.stop()} aria-label="Finalizar sesión y apagar el micrófono">
              <PowerIcon />
              <span>Finalizar</span>
            </button>
          </nav>
        </>
      )}

      {showTranscript && recent.length > 0 && (
        <ol className="hud-transcript" aria-label="Transcripción">
          {recent.map((t, i) => (
            <li key={`${i}-${t.text.slice(0, 12)}`} className={t.role} style={{ opacity: 0.45 + (0.55 * (i + 1)) / recent.length }}>
              <span className="who">{t.role === 'user' ? 'Usuario' : 'JARVIS'}</span>
              <p>{t.text}</p>
            </li>
          ))}
        </ol>
      )}

      <aside className={`panel ${panelOpen ? 'open' : ''}`} aria-hidden={!panelOpen} aria-label="Texto, ajustes y actividad">
        <header className="row between">
          <h2>Panel</h2>
          <button className="icon-btn" onClick={() => setPanelOpen(false)} aria-label="Cerrar panel">
            <CloseIcon />
          </button>
        </header>

        {backend.kind === 'tauri' && status && (
          <TextChat
            backend={backend}
            provider={status.text_provider}
            sendToVoice={(t) => controller.sendText(t)}
            voiceContext={() => carryOverSummary(snap.transcript, snap.operations)}
            onExchange={(user, assistant) => {
              const ts = new Date().toISOString();
              setTextHistory((h) => [...h, { role: 'user' as const, text: user, ts }, { role: 'assistant' as const, text: assistant, ts }].slice(-10));
              refreshActivity();
            }}
          />
        )}

        <section className="panel-section" aria-label="Ajustes de voz">
          <h3>Voz</h3>
          <label>
            Voz de JARVIS
            <select value={currentVoice} disabled={live} onChange={(e) => setVoice(e.target.value)}>
              {(status?.voices ?? []).map((v) => (
                <option key={v} value={v}>
                  {v}
                  {v === status?.default_voice ? ' (predeterminada)' : ''}
                </option>
              ))}
            </select>
          </label>
          <label className="check" title="Evita que el altavoz se confunda con tu voz si no usas auriculares. Desactiva la interrupción por voz (el botón Detener sigue funcionando).">
            <input type="checkbox" checked={halfDuplex} disabled={live} onChange={(e) => setHalfDuplex(e.target.checked)} />
            Modo altavoz (silenciar micro mientras JARVIS habla)
          </label>
          <label className="check">
            <input type="checkbox" checked={showTranscript} onChange={(e) => setShowTranscript(e.target.checked)} />
            Mostrar la conversación en pantalla
          </label>
          <label className="check" title="Al abrir JARVIS (por ejemplo con «Oye Siri, abre JARVIS») empieza a escuchar sin pulsar el botón.">
            <input type="checkbox" checked={autoStart} onChange={(e) => setAutoStart(e.target.checked)} />
            Empezar a escuchar al abrir JARVIS
          </label>
          {status && (
            <p className="fine">
              Modelo {status.realtime_model}
              {state.transport === 'websocket' ? ' · transporte WebSocket de respaldo' : ''}
            </p>
          )}
        </section>

        {status && (
          <section className="panel-section" aria-label="Capacidades">
            <h3>Capacidades</h3>
            <ul className="caps">
              <li className="on">Notas y tareas de Obsidian{status.vault.ok ? '' : ' — Vault no configurada'}</li>
              <li className="on">Tiempo{status.weather_location ? ` · ciudad: ${status.weather_location}` : ' · sin ciudad predeterminada'}</li>
              <li className={status.web_search ? 'on' : 'off'}>Búsqueda en Internet{status.web_search ? '' : ' — falta la clave de OpenAI'}</li>
              <li className="on">Leer y abrir páginas web (abrir pide confirmación)</li>
              <li className={status.traffic ? 'on' : 'off'}>
                {status.traffic ? `Tráfico en vivo · ${status.traffic_city}` : `Tráfico en vivo — falta la clave de TomTom (usará noticias de ${status.traffic_city})`}
              </li>
              <li className={status.flights ? 'on' : 'off'}>
                {status.flights ? 'Vuelos: precios y horarios (Google Flights)' : 'Vuelos — falta la clave de SerpApi (usará la búsqueda web)'}
              </li>
              <li className={status.email ? 'on' : 'off'}>{status.email ? `Correo (solo lectura) · ${status.email}` : 'Correo — no configurado'}</li>
            </ul>
          </section>
        )}

        <section className="panel-section" aria-label="Actividad">
          <h3>Actividad verificada</h3>
          {audit.length ? (
            <ul className="audit">
              {audit.map((a) => (
                <li key={`${a.call_id}-${a.ts}`} className={a.status}>
                  <code>{a.tool}</code> {a.status}
                  {a.code ? ` (${a.code})` : ''} · {a.origin} · {a.duration_ms} ms{a.target ? ` · ${a.target}` : ''}
                </li>
              ))}
            </ul>
          ) : (
            <p className="fine">Sin operaciones todavía.</p>
          )}
          {usage && (
            <p className="fine">
              Uso (30 días): {usage.sessions} sesiones · {usage.total_tokens.toLocaleString('es')} tokens · {Math.round(usage.duration_seconds / 60)} min
            </p>
          )}
        </section>

        <section className="panel-section" aria-label="Privacidad">
          <h3>Privacidad</h3>
          <p className="fine">
            Mientras la sesión de voz está activa, el audio del micrófono se envía a OpenAI para procesarlo (no es local). Las herramientas
            leen solo los fragmentos de la Vault necesarios.{' '}
            {status?.save_transcripts
              ? `Las transcripciones se guardan localmente ${status.transcript_retention_days} días.`
              : 'Las transcripciones no se guardan.'}{' '}
            El audio nunca se guarda.
          </p>
        </section>
      </aside>
      {panelOpen && <div className="scrim" onClick={() => setPanelOpen(false)} aria-hidden="true" />}

      <ApprovalDialog
        requests={approvals}
        onDecide={(id, ok) => {
          setApprovals((a) => a.filter((r) => r.id !== id));
          void backend.decideApproval(id, ok).catch(() => undefined);
        }}
      />
    </main>
  );
}
