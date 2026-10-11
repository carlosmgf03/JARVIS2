// Controlador de la conversación de voz con OpenAI Realtime.
//
// Responsabilidades:
// - Pedir el micrófono solo cuando el usuario pulsa «Hablar con JARVIS».
// - Obtener de Rust un secreto efímero y abrir la sesión (WebRTC; WebSocket
//   como alternativa documentada si el WebView no tiene WebRTC).
// - Traducir los eventos del SDK a la máquina de estados de la interfaz.
// - Interrumpir, silenciar, ajustar volumen y cerrar liberando todo.
// - Reconectar ante caídas sin duplicar herramientas ni operaciones: cada
//   conexión crea un agente nuevo con el catálogo una sola vez, los eventos de
//   sesiones antiguas se ignoran (generación), el backend deduplica por
//   `call_id` y la nueva sesión recibe un resumen con lo ya ejecutado.

import {
  OpenAIRealtimeWebRTC,
  OpenAIRealtimeWebSocket,
  RealtimeAgent,
  RealtimeSession,
} from '@openai/agents/realtime';
import type { Backend } from '../lib/backend';
import { toJarvisError } from '../lib/backend';
import type { JarvisError, RealtimeSessionResponse, ToolOutcome, TranscriptEntry } from '../lib/types';
import { initialVoiceState, isBusy, isSessionActive, voiceReducer, type VoiceEvent, type VoiceState } from './machine';
import { PcmAudioIO } from './pcmAudio';
import { backoffMs, shouldRetry } from './reconnect';
import { buildRealtimeTools, type ToolHooks } from './tools';
import { carryOverSummary, historyToTranscript, type CompletedOperation } from './transcript';

type Listener = (...args: any[]) => void; // eslint-disable-line @typescript-eslint/no-explicit-any

/** Subconjunto de `RealtimeSession` que usa el controlador (facilita pruebas). */
export interface SessionLike {
  on(event: string, cb: Listener): unknown;
  connect(opts: { apiKey: string }): Promise<void>;
  close(): void;
  interrupt(): void;
  mute(muted: boolean): void;
  sendMessage(text: string): void;
  sendAudio?(audio: ArrayBuffer, opts?: { commit?: boolean }): void;
  readonly usage: { requests: number; inputTokens: number; outputTokens: number; totalTokens: number };
  readonly transport: { on(event: string, cb: Listener): unknown };
}

export interface SessionFactoryInput {
  response: RealtimeSessionResponse;
  stream: MediaStream;
  audioElement: HTMLAudioElement;
  transport: 'webrtc' | 'websocket';
  tools: ReturnType<typeof buildRealtimeTools>;
}

export type SessionFactory = (input: SessionFactoryInput) => SessionLike;

export const sdkSessionFactory: SessionFactory = ({ response, stream, audioElement, transport, tools }) => {
  const s = response.session.settings;
  const agent = new RealtimeAgent({ name: 'JARVIS', instructions: response.instructions, tools, voice: s.voice });
  const layer =
    transport === 'webrtc'
      ? new OpenAIRealtimeWebRTC({ mediaStream: stream, audioElement })
      : new OpenAIRealtimeWebSocket();
  return new RealtimeSession(agent, {
    transport: layer,
    model: s.model,
    tracingDisabled: true,
    historyStoreAudio: false,
    config: {
      outputModalities: ['audio'],
      audio: {
        input: {
          format: { type: 'audio/pcm', rate: 24000 },
          transcription: s.transcription_model ? { model: s.transcription_model, language: s.language } : undefined,
          turnDetection: s.turn_detection,
          noiseReduction: s.noise_reduction ? { type: s.noise_reduction as 'near_field' | 'far_field' } : null,
        },
        output: { voice: s.voice },
      },
    },
  }) as unknown as SessionLike;
};

export interface StartOptions {
  voice: string | null;
  /** Contexto previo (p. ej. conversación de texto) para dar continuidad. */
  carryOver: string | null;
  /** Silencia el micrófono mientras JARVIS habla (altavoces sin auriculares). */
  halfDuplex: boolean;
  saveTranscript: boolean;
}

export interface VoiceSnapshot {
  state: VoiceState;
  transcript: TranscriptEntry[];
  volume: number;
  outputMuted: boolean;
  playbackBlocked: boolean;
  notice: JarvisError | null;
  operations: CompletedOperation[];
}

export interface ControllerDeps {
  backend: Backend;
  sessionFactory?: SessionFactory;
  getUserMedia?: (c: MediaStreamConstraints) => Promise<MediaStream>;
  hasWebRTC?: () => boolean;
  sleep?: (ms: number) => Promise<void>;
  audioElement?: HTMLAudioElement;
}

const MIC_CONSTRAINTS: MediaStreamConstraints = {
  audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 },
  video: false,
};

export function micError(e: unknown): JarvisError {
  const name = (e as { name?: string })?.name;
  if (name === 'NotAllowedError' || name === 'SecurityError') {
    return {
      code: 'microphone_denied',
      message:
        'JARVIS no tiene permiso para usar el micrófono. Concédelo en los ajustes de privacidad del sistema y vuelve a intentarlo.',
      retryable: false,
    };
  }
  return {
    code: 'microphone_unavailable',
    message: 'No se encontró un micrófono disponible. Conecta uno o revisa que no lo esté usando otra aplicación.',
    retryable: false,
  };
}

export class VoiceController {
  private snapshot: VoiceSnapshot = {
    state: initialVoiceState,
    transcript: [],
    volume: 1,
    outputMuted: false,
    playbackBlocked: false,
    notice: null,
    operations: [],
  };
  private listeners = new Set<(s: VoiceSnapshot) => void>();
  private session: SessionLike | null = null;
  private stream: MediaStream | null = null;
  private pcm: PcmAudioIO | null = null;
  private generation = 0;
  private closing = false;
  private options: StartOptions | null = null;
  private sessionId = '';
  private startedAt = 0;
  private model = '';
  private userMuted = false;
  private readonly audio: HTMLAudioElement;
  private readonly sessionFactory: SessionFactory;
  private readonly getUserMedia: (c: MediaStreamConstraints) => Promise<MediaStream>;
  private readonly hasWebRTC: () => boolean;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly backend: Backend;

  constructor(deps: ControllerDeps) {
    this.backend = deps.backend;
    this.sessionFactory = deps.sessionFactory ?? sdkSessionFactory;
    this.getUserMedia =
      deps.getUserMedia ??
      ((c) => {
        if (!navigator.mediaDevices?.getUserMedia) return Promise.reject(new DOMException('', 'NotFoundError'));
        return navigator.mediaDevices.getUserMedia(c);
      });
    this.hasWebRTC = deps.hasWebRTC ?? (() => typeof RTCPeerConnection !== 'undefined');
    this.sleep = deps.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
    this.audio = deps.audioElement ?? document.createElement('audio');
    this.audio.autoplay = true;
  }

  // ------------------------------------------------------------ suscripción

  getSnapshot = (): VoiceSnapshot => this.snapshot;

  subscribe = (fn: (s: VoiceSnapshot) => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  private update(patch: Partial<VoiceSnapshot>) {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const l of this.listeners) l(this.snapshot);
  }

  private dispatch(e: VoiceEvent) {
    const prev = this.snapshot.state;
    const next = voiceReducer(prev, e);
    if (next !== prev) {
      this.update({ state: next });
      this.applyHalfDuplex(prev, next);
    }
  }

  // ------------------------------------------------------------ ciclo de vida

  async start(options: StartOptions): Promise<void> {
    const s = this.snapshot.state;
    if (isSessionActive(s) || isBusy(s)) return; // evita sesiones duplicadas
    this.options = options;
    this.closing = false;
    this.userMuted = false;
    this.sessionId = crypto.randomUUID();
    this.startedAt = Date.now();
    this.update({ transcript: [], notice: null, playbackBlocked: false, operations: [] });
    this.dispatch({ type: 'START' });

    try {
      this.stream = await this.getUserMedia(MIC_CONSTRAINTS);
    } catch (e) {
      this.fail(micError(e));
      return;
    }
    if (this.closing) return this.releaseMedia();
    this.dispatch({ type: 'MIC_GRANTED' });

    try {
      await this.connect(options.carryOver);
    } catch (e) {
      const err = toJarvisError(e);
      if (shouldRetry(err, 0) && !this.closing) {
        await this.reconnect(err);
      } else {
        this.fail(err);
      }
    }
  }

  private async connect(carryOver: string | null): Promise<void> {
    const gen = ++this.generation;
    const response = await this.backend.createRealtimeSession(this.options?.voice ?? null, carryOver);
    if (this.closing || gen !== this.generation) return;
    this.model = response.session.settings.model;
    const transport = this.hasWebRTC() ? 'webrtc' : 'websocket';
    const hooks: ToolHooks = {
      onStart: (name) => gen === this.generation && this.dispatch({ type: 'TOOL_STARTED', name }),
      onEnd: (o) => this.recordOperation(o, gen),
    };
    const session = this.sessionFactory({
      response,
      stream: this.stream!,
      audioElement: this.audio,
      transport,
      tools: buildRealtimeTools(response.tools, this.backend, hooks),
    });
    this.session = session;
    this.wire(session, gen, transport);

    if (transport === 'websocket') {
      this.pcm = new PcmAudioIO(
        this.stream!,
        (chunk) => gen === this.generation && session.sendAudio?.(chunk),
        () => gen === this.generation && this.dispatch({ type: 'AUDIO_STOPPED' }),
      );
      await this.pcm.start();
      this.pcm.setVolume(this.snapshot.outputMuted ? 0 : this.snapshot.volume);
    }

    try {
      await session.connect({ apiKey: response.session.ephemeral_key });
    } catch (e) {
      throw connectError(e, transport);
    }
    if (this.closing || gen !== this.generation) {
      session.close();
      return;
    }
    this.applyOutput();
    if (this.userMuted) session.mute(true);
    this.dispatch({ type: 'CONNECTED', transport });
    if (transport === 'webrtc') {
      this.audio.play().catch(() => this.update({ playbackBlocked: true }));
    }
  }

  private wire(session: SessionLike, gen: number, transport: 'webrtc' | 'websocket') {
    const live = () => gen === this.generation && !this.closing;
    session.on('transport_event', (ev: { type?: string; error?: { message?: string; code?: string } }) => {
      if (!live()) return;
      switch (ev?.type) {
        case 'input_audio_buffer.speech_started':
          this.pcm?.stopPlayback();
          return this.dispatch({ type: 'SPEECH_STARTED' });
        case 'input_audio_buffer.speech_stopped':
          return this.dispatch({ type: 'SPEECH_STOPPED' });
        case 'response.created':
          return this.dispatch({ type: 'RESPONSE_STARTED' });
        case 'response.done':
          return this.dispatch({ type: 'RESPONSE_DONE' });
        case 'output_audio_buffer.started':
          return this.dispatch({ type: 'AUDIO_STARTED' });
        case 'output_audio_buffer.stopped':
        case 'output_audio_buffer.cleared':
          return this.dispatch({ type: 'AUDIO_STOPPED' });
        case 'error':
          return this.update({ notice: providerEventError(ev.error) });
      }
    });
    session.on('audio_start', () => live() && transport === 'websocket' && this.dispatch({ type: 'AUDIO_STARTED' }));
    session.on('audio', (ev: { data: ArrayBuffer }) => live() && this.pcm?.play(ev.data));
    session.on('audio_interrupted', () => {
      if (!live()) return;
      this.pcm?.stopPlayback();
      this.dispatch({ type: 'INTERRUPTED' });
    });
    session.on('history_updated', (history: unknown[]) => {
      if (!live()) return;
      const fresh = historyToTranscript(history);
      // Tras una reconexión el historial empieza de cero: se conserva lo previo.
      const prefix = this.snapshot.transcript.filter((t) => (t as TranscriptEntry & { gen?: number }).gen !== gen);
      this.update({ transcript: [...prefix, ...fresh.map((t) => ({ ...t, gen }))] });
    });
    session.on('error', (e: { error?: unknown }) => {
      if (!live()) return;
      this.update({ notice: providerEventError(e?.error) });
    });
    session.transport.on('connection_change', (status: string) => {
      if (status === 'disconnected' && live()) {
        void this.reconnect({ code: 'connection_lost', message: 'Se perdió la conexión con OpenAI.', retryable: true });
      }
    });
  }

  private async reconnect(cause: JarvisError): Promise<void> {
    if (this.snapshot.state.phase === 'reconnecting') return;
    this.teardownSession();
    let lastError = cause;
    for (let attempt = 1; shouldRetry(lastError, attempt - 1); attempt++) {
      this.dispatch({ type: 'CONNECTION_LOST', attempt });
      await this.sleep(backoffMs(attempt));
      if (this.closing) return;
      try {
        await this.connect(carryOverSummary(this.snapshot.transcript, this.snapshot.operations));
        return;
      } catch (e) {
        lastError = toJarvisError(e);
        this.teardownSession();
      }
    }
    if (!this.closing) this.fail(lastError.code === 'connection_lost' ? lastError : { ...lastError });
  }

  private recordOperation(o: ToolOutcome, gen: number) {
    const detail = (o.data as { path?: string } | null)?.path;
    this.update({
      operations: [...this.snapshot.operations, { tool: o.tool, status: o.status, detail }].slice(-20),
    });
    if (gen === this.generation) this.dispatch({ type: 'TOOL_FINISHED' });
  }

  /** Finaliza la sesión y apaga el micrófono. */
  async stop(): Promise<void> {
    this.closing = true;
    this.generation++;
    const usage = this.session?.usage;
    this.teardownSession();
    this.releaseMedia();
    await this.persist(usage);
    this.dispatch({ type: 'STOPPED' });
    this.update({ playbackBlocked: false });
  }

  private async persist(usage: SessionLike['usage'] | undefined) {
    const duration = Math.round((Date.now() - this.startedAt) / 1000);
    if (usage && usage.requests > 0) {
      await this.backend
        .recordUsage({
          model: this.model,
          requests: usage.requests,
          input_tokens: usage.inputTokens,
          output_tokens: usage.outputTokens,
          total_tokens: usage.totalTokens,
          duration_seconds: duration,
        })
        .catch(() => undefined);
    }
    if (this.options?.saveTranscript && this.snapshot.transcript.length) {
      const entries = this.snapshot.transcript.map(({ role, text, ts }) => ({ role, text, ts }));
      await this.backend.saveTranscript(this.sessionId, entries).catch(() => undefined);
    }
  }

  private teardownSession() {
    const s = this.session;
    this.session = null;
    this.generation++;
    if (s) {
      try {
        s.close();
      } catch {
        /* ya cerrada */
      }
    }
    void this.pcm?.close();
    this.pcm = null;
  }

  private releaseMedia() {
    for (const t of this.stream?.getTracks() ?? []) t.stop();
    this.stream = null;
    this.audio.pause();
    this.audio.srcObject = null;
  }

  private fail(error: JarvisError) {
    this.closing = true;
    this.teardownSession();
    this.releaseMedia();
    this.dispatch({ type: 'FAILED', error });
    this.update({ notice: null });
  }

  // --------------------------------------------------------------- controles

  /** Detiene la respuesta hablada en curso (botón «Detener»). */
  interrupt(): void {
    if (!this.session) return;
    this.session.interrupt();
    this.pcm?.stopPlayback();
    this.dispatch({ type: 'INTERRUPTED' });
  }

  setMicMuted(muted: boolean): void {
    this.userMuted = muted;
    this.session?.mute(muted);
    if (this.pcm) this.pcm.captureEnabled = !muted;
    this.dispatch({ type: 'MUTE', muted });
  }

  setVolume(volume: number): void {
    this.update({ volume: Math.max(0, Math.min(1, volume)) });
    this.applyOutput();
  }

  setOutputMuted(muted: boolean): void {
    this.update({ outputMuted: muted });
    this.applyOutput();
  }

  resumePlayback(): void {
    this.audio
      .play()
      .then(() => this.update({ playbackBlocked: false }))
      .catch(() =>
        this.update({
          notice: { code: 'playback_failed', message: 'El sistema bloqueó la reproducción de audio.', retryable: true },
        }),
      );
  }

  /** Envía texto escrito dentro de la sesión de voz activa. */
  sendText(text: string): boolean {
    if (!this.session || !isSessionActive(this.snapshot.state)) return false;
    this.session.sendMessage(text);
    return true;
  }

  setApprovalPending(pending: boolean): void {
    this.dispatch({ type: 'APPROVAL_PENDING', pending });
  }

  dismissNotice(): void {
    this.update({ notice: null });
  }

  private applyOutput() {
    const v = this.snapshot.outputMuted ? 0 : this.snapshot.volume;
    this.audio.volume = v;
    this.audio.muted = this.snapshot.outputMuted;
    this.pcm?.setVolume(v);
  }

  private applyHalfDuplex(prev: VoiceState, next: VoiceState) {
    if (!this.options?.halfDuplex || !this.session || this.userMuted) return;
    if (prev.phase !== 'speaking' && next.phase === 'speaking') {
      this.session.mute(true);
      if (this.pcm) this.pcm.captureEnabled = false;
    } else if (prev.phase === 'speaking' && next.phase !== 'speaking') {
      this.session.mute(false);
      if (this.pcm) this.pcm.captureEnabled = true;
    }
  }
}

/** Traduce un fallo al abrir la conexión (WebRTC o WebSocket) a un error claro. */
export function connectError(e: unknown, transport: 'webrtc' | 'websocket'): JarvisError {
  const known = e as Partial<JarvisError>;
  if (known && typeof known.code === 'string' && typeof known.message === 'string') return known as JarvisError;
  const raw = e instanceof Error ? e.message : '';
  const detail = raw.replace(/(sk-|ek_)[A-Za-z0-9_-]{6,}/g, '[REDACTADO]').slice(0, 200);
  if (/\b401\b|unauthori[sz]ed/i.test(raw)) {
    return { code: 'invalid_api_key', message: 'OpenAI rechazó el secreto temporal de la sesión.', retryable: false };
  }
  return {
    code: 'network_error',
    message: `No se pudo establecer la conexión de voz con OpenAI (transporte ${transport === 'webrtc' ? 'WebRTC' : 'WebSocket'}). Revisa la conexión a Internet, el cortafuegos o el proxy.${detail ? ` Detalle: ${detail}` : ''}`,
    retryable: true,
  };
}

function providerEventError(raw: unknown): JarvisError {
  const e = (raw ?? {}) as { message?: string; code?: string; type?: string };
  const code = String(e.code ?? e.type ?? '');
  if (code.includes('rate_limit')) {
    return { code: 'rate_limited', message: 'OpenAI ha limitado las solicitudes. Espera unos segundos.', retryable: true };
  }
  if (code.includes('model')) {
    return { code: 'model_unavailable', message: 'El modelo Realtime configurado no está disponible.', retryable: false };
  }
  if (typeof Event !== 'undefined' && raw instanceof Event) {
    return { code: 'network_error', message: 'Error de conexión con el servicio de voz.', retryable: true };
  }
  const message = e.message ?? (raw instanceof Error ? raw.message : 'Error del proveedor de voz.');
  return { code: 'provider_error', message: String(message).replace(/(sk-|ek_)[A-Za-z0-9_-]{6,}/g, '[REDACTADO]'), retryable: true };
}
