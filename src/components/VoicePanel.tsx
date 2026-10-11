import type { VoiceController, VoiceSnapshot } from '../voice/controller';
import { isBusy, isSessionActive, PHASE_LABEL } from '../voice/machine';

interface Props {
  controller: VoiceController;
  snap: VoiceSnapshot;
  voiceAvailable: boolean;
  voices: string[];
  voice: string;
  onVoice: (v: string) => void;
  halfDuplex: boolean;
  onHalfDuplex: (v: boolean) => void;
  onStart: () => void;
}

export function VoicePanel({ controller, snap, voiceAvailable, voices, voice, onVoice, halfDuplex, onHalfDuplex, onStart }: Props) {
  const { state } = snap;
  const active = isSessionActive(state);
  const busy = isBusy(state);
  const micLive = (active || state.phase === 'reconnecting') && !state.micMuted;

  return (
    <section className="card voice" aria-label="Conversación de voz">
      <div className={`orb phase-${state.phase}`} aria-hidden="true" />
      <div className="phase" role="status" aria-live="polite">
        {PHASE_LABEL[state.phase]}
        {state.phase === 'reconnecting' && ` (intento ${state.reconnectAttempt})`}
        {state.activeTool && <span className="tag">herramienta: {state.activeTool}</span>}
        {state.awaitingApproval && <span className="tag warn">esperando tu confirmación</span>}
      </div>

      <div className={`mic ${micLive ? 'on' : 'off'}`} aria-live="polite">
        <span className="dot" aria-hidden="true" />
        {micLive ? 'Micrófono activo: el audio se envía a OpenAI' : state.micMuted && active ? 'Micrófono silenciado' : 'Micrófono apagado'}
      </div>

      {state.error && (
        <div className="alert error" role="alert">
          <strong>{state.error.message}</strong>
          <small>código: {state.error.code}</small>
        </div>
      )}
      {snap.notice && (
        <div className="alert warn" role="alert">
          {snap.notice.message}
          <button className="link" onClick={() => controller.dismissNotice()}>
            Cerrar
          </button>
        </div>
      )}
      {snap.playbackBlocked && (
        <div className="alert warn">
          El sistema bloqueó el audio de JARVIS.{' '}
          <button className="link" onClick={() => controller.resumePlayback()}>
            Activar audio
          </button>
        </div>
      )}

      <div className="row center">
        {!active && !busy ? (
          <button className="primary big" disabled={!voiceAvailable} onClick={onStart}>
            {state.phase === 'error' ? 'Reintentar' : 'Hablar con JARVIS'}
          </button>
        ) : (
          <>
            <button
              className="secondary"
              disabled={!active || !(state.phase === 'speaking' || state.phase === 'processing')}
              onClick={() => controller.interrupt()}
            >
              Detener respuesta
            </button>
            <button className="secondary" disabled={!active} onClick={() => controller.setMicMuted(!state.micMuted)}>
              {state.micMuted ? 'Activar micrófono' : 'Silenciar micrófono'}
            </button>
            <button className="danger" onClick={() => void controller.stop()}>
              Finalizar sesión
            </button>
          </>
        )}
      </div>

      <div className="controls">
        <label>
          Volumen
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={snap.volume}
            onChange={(e) => controller.setVolume(Number(e.target.value))}
            aria-label="Volumen de JARVIS"
          />
        </label>
        <label className="check">
          <input type="checkbox" checked={snap.outputMuted} onChange={(e) => controller.setOutputMuted(e.target.checked)} />
          Silenciar a JARVIS
        </label>
        <label>
          Voz
          <select value={voice} disabled={active || busy} onChange={(e) => onVoice(e.target.value)}>
            {voices.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        </label>
        <label className="check" title="Evita que el altavoz se confunda con tu voz si no usas auriculares. Desactiva la interrupción por voz (el botón Detener sigue funcionando).">
          <input type="checkbox" checked={halfDuplex} disabled={active || busy} onChange={(e) => onHalfDuplex(e.target.checked)} />
          Modo altavoz (silenciar micro mientras habla)
        </label>
      </div>
      {state.transport === 'websocket' && (
        <p className="muted small">Transporte WebSocket de respaldo (este WebView no ofrece WebRTC).</p>
      )}
    </section>
  );
}
