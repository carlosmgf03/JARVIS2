import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { ApprovalDialog } from './components/ApprovalDialog';
import { Messages } from './components/Messages';
import { MicTest } from './components/MicTest';
import { TextChat } from './components/TextChat';
import { VoicePanel } from './components/VoicePanel';
import { getBackend } from './lib/backend';
import type { ApprovalRequest, AppStatus, AuditEntry, TranscriptEntry, UsageSummary } from './lib/types';
import { VoiceController } from './voice/controller';
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
  const [app, setApp] = useState<AppStatus | null>(null);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [voice, setVoice] = usePref<string | null>('voice', null);
  const [halfDuplex, setHalfDuplex] = usePref('halfDuplex', false);
  const [showTranscript, setShowTranscript] = usePref('showTranscript', true);
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
  const testMode = !status?.voice_available;

  const start = () =>
    void controller.start({
      voice: voice ?? status?.default_voice ?? null,
      carryOver: carryOverSummary(textHistory),
      halfDuplex,
      saveTranscript: Boolean(status?.save_transcripts),
    });

  return (
    <main>
      <header className="top">
        <h1>JARVIS</h1>
        {status && (
          <span className="muted small">
            {status.realtime_model} · voz {voice ?? status.default_voice} · texto: {status.text_provider}
          </span>
        )}
      </header>

      {app?.config_error && <div className="alert error">Configuración no válida: {app.config_error}. Se usan valores por defecto.</div>}

      {status && testMode && (
        <section className="banner test" role="note">
          <strong>MODO DE PRUEBA — la conversación de voz real no está disponible.</strong>
          <ul>
            {status.limitations.map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
          <p>
            Puedes probar el micrófono localmente (no se envía audio) y usar la interfaz de texto. Para activar la voz, configura la
            clave de OpenAI (ver docs/configuration.md) y reinicia JARVIS.
          </p>
          <MicTest />
        </section>
      )}
      {status && !testMode && status.limitations.length > 0 && (
        <div className="alert warn">{status.limitations.join(' ')}</div>
      )}

      <p className="privacy small">
        Privacidad: mientras la sesión de voz está activa, el audio del micrófono se envía a OpenAI para su procesamiento (no es
        local). Las herramientas leen solo los fragmentos de la Vault necesarios.{' '}
        {status?.save_transcripts
          ? `Las transcripciones se guardan localmente ${status.transcript_retention_days} días.`
          : 'Las transcripciones no se guardan.'}{' '}
        El audio nunca se guarda.
      </p>

      <div className="grid">
        <VoicePanel
          controller={controller}
          snap={snap}
          voiceAvailable={Boolean(status?.voice_available)}
          voices={status?.voices ?? []}
          voice={voice ?? status?.default_voice ?? 'marin'}
          onVoice={setVoice}
          halfDuplex={halfDuplex}
          onHalfDuplex={setHalfDuplex}
          onStart={start}
        />

        <section className="card transcript" aria-label="Transcripción">
          <header className="row between">
            <h2>Transcripción</h2>
            <label className="check small">
              <input type="checkbox" checked={showTranscript} onChange={(e) => setShowTranscript(e.target.checked)} />
              Mostrar
            </label>
          </header>
          {showTranscript ? (
            snap.transcript.length ? (
              <Messages count={snap.transcript.length}>
                {snap.transcript.map((t, i) => (
                  <li key={i} className={`msg ${t.role}`}>
                    <pre>{t.text}</pre>
                  </li>
                ))}
              </Messages>
            ) : (
              <p className="muted">Aquí aparecerá la conversación de voz.</p>
            )
          ) : (
            <p className="muted">Transcripción oculta.</p>
          )}
        </section>

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

        <section className="card activity" aria-label="Actividad">
          <h2>Actividad verificada</h2>
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
            <p className="muted">Sin operaciones todavía.</p>
          )}
          {usage && (
            <p className="muted small">
              Uso (30 días): {usage.sessions} sesiones · {usage.total_tokens.toLocaleString('es')} tokens ·{' '}
              {Math.round(usage.duration_seconds / 60)} min
            </p>
          )}
        </section>
      </div>

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
