import { describe, expect, it, vi } from 'vitest';
import type { Backend } from '../lib/backend';
import { BackendError } from '../lib/backend';
import type { RealtimeSessionResponse, ToolOutcome } from '../lib/types';
import { VoiceController, type SessionFactoryInput, type SessionLike } from './controller';
import { initialVoiceState, voiceReducer } from './machine';
import { backoffMs, shouldRetry } from './reconnect';
import { executeViaBackend, outcomeForModel } from './tools';
import { carryOverSummary, historyToTranscript } from './transcript';

// ---------------------------------------------------------------- dobles

type Cb = (...a: any[]) => void; // eslint-disable-line @typescript-eslint/no-explicit-any

class FakeSession implements SessionLike {
  handlers = new Map<string, Cb[]>();
  transportHandlers = new Map<string, Cb[]>();
  connectedWith: string | null = null;
  closed = false;
  interrupts = 0;
  muted: boolean[] = [];
  sent: string[] = [];
  usage = { requests: 2, inputTokens: 100, outputTokens: 50, totalTokens: 150 };
  transport = {
    on: (ev: string, cb: Cb) => this.transportHandlers.set(ev, [...(this.transportHandlers.get(ev) ?? []), cb]),
  };
  constructor(public input: SessionFactoryInput, private failConnect?: BackendError) {}
  on(ev: string, cb: Cb) {
    this.handlers.set(ev, [...(this.handlers.get(ev) ?? []), cb]);
  }
  emit(ev: string, ...args: unknown[]) {
    for (const h of this.handlers.get(ev) ?? []) h(...args);
  }
  server(type: string) {
    this.emit('transport_event', { type });
  }
  drop() {
    for (const h of this.transportHandlers.get('connection_change') ?? []) h('disconnected');
  }
  async connect({ apiKey }: { apiKey: string }) {
    if (this.failConnect) throw this.failConnect;
    this.connectedWith = apiKey;
  }
  close() {
    this.closed = true;
  }
  interrupt() {
    this.interrupts++;
  }
  mute(m: boolean) {
    this.muted.push(m);
  }
  sendMessage(t: string) {
    this.sent.push(t);
  }
}

function response(n: number): RealtimeSessionResponse {
  return {
    session: {
      ephemeral_key: `ek_test_${n}`,
      expires_at: 0,
      settings: {
        model: 'gpt-realtime-2.1',
        voice: 'marin',
        language: 'es',
        transcription_model: null,
        turn_detection: { type: 'semantic_vad' },
        noise_reduction: 'far_field',
      },
    },
    instructions: 'Eres JARVIS',
    tools: [
      { name: 'add_task', description: 'Añade', parameters: { type: 'object' }, risk: 'write' },
    ],
  };
}

function fakeStream() {
  const track = { stop: vi.fn() };
  return { stream: { getTracks: () => [track] } as unknown as MediaStream, track };
}

function fakeAudio() {
  return {
    autoplay: false,
    volume: 1,
    muted: false,
    srcObject: null,
    play: vi.fn(() => Promise.resolve()),
    pause: vi.fn(),
  } as unknown as HTMLAudioElement;
}

function setup(opts: { createErrors?: BackendError[]; micError?: unknown } = {}) {
  const sessions: FakeSession[] = [];
  const createErrors = [...(opts.createErrors ?? [])];
  let created = 0;
  const backend = {
    kind: 'tauri',
    createRealtimeSession: vi.fn(async (_voice: string | null, _carry: string | null) => {
      const err = createErrors.shift();
      if (err) throw err;
      return response(++created);
    }),
    invokeTool: vi.fn(async (callId: string, name: string): Promise<ToolOutcome> => ({
      call_id: callId,
      tool: name,
      status: 'ok',
      data: { path: 'JARVIS/Tareas.md', verified: true },
      error: null,
      verified: true,
      duration_ms: 3,
      deduplicated: false,
    })),
    recordUsage: vi.fn(async () => {}),
    saveTranscript: vi.fn(async () => null),
  } as unknown as Backend & Record<string, ReturnType<typeof vi.fn>>;
  const { stream, track } = fakeStream();
  const controller = new VoiceController({
    backend,
    sessionFactory: (input) => {
      const s = new FakeSession(input);
      sessions.push(s);
      return s;
    },
    getUserMedia: opts.micError ? () => Promise.reject(opts.micError) : async () => stream,
    hasWebRTC: () => true,
    sleep: async () => {},
    audioElement: fakeAudio(),
  });
  return { controller, backend, sessions, track };
}

const START = { voice: 'marin', carryOver: null, halfDuplex: false, saveTranscript: false };

// ---------------------------------------------------------------- pruebas

describe('máquina de estados', () => {
  it('recorre escucha → habla del usuario → proceso → respuesta → escucha', () => {
    let s = voiceReducer(initialVoiceState, { type: 'START' });
    s = voiceReducer(s, { type: 'MIC_GRANTED' });
    s = voiceReducer(s, { type: 'CONNECTED', transport: 'webrtc' });
    expect(s.phase).toBe('listening');
    s = voiceReducer(s, { type: 'SPEECH_STARTED' });
    expect(s.phase).toBe('user_speaking');
    s = voiceReducer(s, { type: 'SPEECH_STOPPED' });
    expect(s.phase).toBe('processing');
    s = voiceReducer(s, { type: 'AUDIO_STARTED' });
    expect(s.phase).toBe('speaking');
    // Interrupción por voz: el usuario vuelve a hablar.
    s = voiceReducer(s, { type: 'SPEECH_STARTED' });
    expect(s.phase).toBe('user_speaking');
  });

  it('ignora eventos de audio sin sesión activa', () => {
    expect(voiceReducer(initialVoiceState, { type: 'AUDIO_STARTED' }).phase).toBe('idle');
  });
});

describe('reintentos', () => {
  it('solo reintenta errores transitorios y con límite', () => {
    expect(shouldRetry({ code: 'network_error', message: '', retryable: true }, 0)).toBe(true);
    expect(shouldRetry({ code: 'invalid_api_key', message: '', retryable: false }, 0)).toBe(false);
    expect(shouldRetry({ code: 'model_unavailable', message: '', retryable: false }, 0)).toBe(false);
    expect(shouldRetry({ code: 'connection_lost', message: '', retryable: true }, 4)).toBe(false);
    expect(backoffMs(1, () => 0.5)).toBe(1000);
    expect(backoffMs(4, () => 0.5)).toBe(8000);
  });
});

describe('transcripción y continuidad', () => {
  it('extrae texto y transcripciones de audio', () => {
    const t = historyToTranscript([
      { type: 'message', role: 'user', content: [{ type: 'input_audio', transcript: '¿Qué tengo hoy?' }] },
      { type: 'function_call', name: 'daily_summary' },
      { type: 'message', role: 'assistant', content: [{ type: 'output_audio', transcript: 'Tienes una demo.' }] },
    ]);
    expect(t.map((e) => e.text)).toEqual(['¿Qué tengo hoy?', 'Tienes una demo.']);
    const summary = carryOverSummary(t, [{ tool: 'add_task', status: 'ok', detail: 'JARVIS/Tareas.md' }])!;
    expect(summary).toContain('Usuario: ¿Qué tengo hoy?');
    expect(summary).toContain('no las repitas');
  });
});

describe('herramientas', () => {
  it('delega en el backend con el call_id del modelo y comunica el estado real', async () => {
    const { backend } = setup();
    const out = await executeViaBackend(backend, response(1).tools[0], { text: 'x' }, 'call_abc', {});
    expect(backend.invokeTool).toHaveBeenCalledWith('call_abc', 'add_task', { text: 'x' });
    expect(JSON.parse(out).ok).toBe(true);
  });

  it('nunca presenta como éxito un error o una denegación', async () => {
    const denied: ToolOutcome = {
      call_id: 'c',
      tool: 'add_task',
      status: 'denied',
      data: null,
      error: { code: 'approval_denied', message: 'No autorizado', retryable: false },
      verified: false,
      duration_ms: 1,
      deduplicated: false,
    };
    expect(JSON.parse(outcomeForModel(denied))).toMatchObject({ ok: false, denied: true });
    const { backend } = setup();
    (backend.invokeTool as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new BackendError({ code: 'backend_unavailable', message: 'sin backend', retryable: false }),
    );
    const out = JSON.parse(await executeViaBackend(backend, response(1).tools[0], {}, 'c2', {}));
    expect(out.ok).toBe(false);
    expect(out.error.code).toBe('backend_unavailable');
  });
});

describe('VoiceController', () => {
  it('abre la sesión con el secreto efímero y gestiona estados', async () => {
    const { controller, backend, sessions } = setup();
    await controller.start(START);
    expect(backend.createRealtimeSession).toHaveBeenCalledTimes(1);
    expect(sessions[0].connectedWith).toBe('ek_test_1');
    expect(controller.getSnapshot().state.phase).toBe('listening');
    sessions[0].server('input_audio_buffer.speech_started');
    expect(controller.getSnapshot().state.phase).toBe('user_speaking');
    sessions[0].server('response.created');
    sessions[0].server('output_audio_buffer.started');
    expect(controller.getSnapshot().state.phase).toBe('speaking');
    controller.interrupt();
    expect(sessions[0].interrupts).toBe(1);
    expect(controller.getSnapshot().state.phase).toBe('listening');
  });

  it('no abre dos sesiones si se pulsa iniciar dos veces', async () => {
    const { controller, sessions } = setup();
    await Promise.all([controller.start(START), controller.start(START)]);
    expect(sessions).toHaveLength(1);
  });

  it('finalizar cierra la sesión, apaga el micrófono y registra solo métricas', async () => {
    const { controller, backend, sessions, track } = setup();
    await controller.start(START);
    await controller.stop();
    expect(sessions[0].closed).toBe(true);
    expect(track.stop).toHaveBeenCalled();
    expect(controller.getSnapshot().state.phase).toBe('idle');
    expect(backend.recordUsage).toHaveBeenCalledWith(expect.objectContaining({ total_tokens: 150 }));
    expect(backend.saveTranscript).not.toHaveBeenCalled();
  });

  it('informa de la falta de permiso de micrófono sin contactar con OpenAI', async () => {
    const { controller, backend } = setup({ micError: new DOMException('denied', 'NotAllowedError') });
    await controller.start(START);
    expect(controller.getSnapshot().state.error?.code).toBe('microphone_denied');
    expect(backend.createRealtimeSession).not.toHaveBeenCalled();
  });

  it('muestra errores no recuperables (sin clave) y libera el micrófono', async () => {
    const { controller, track } = setup({
      createErrors: [new BackendError({ code: 'missing_api_key', message: 'Falta la clave', retryable: false })],
    });
    await controller.start(START);
    expect(controller.getSnapshot().state.phase).toBe('error');
    expect(controller.getSnapshot().state.error?.code).toBe('missing_api_key');
    expect(track.stop).toHaveBeenCalled();
  });

  it('reintenta errores de red al conectar', async () => {
    const { controller, sessions } = setup({
      createErrors: [new BackendError({ code: 'network_error', message: 'red', retryable: true })],
    });
    await controller.start(START);
    expect(controller.getSnapshot().state.phase).toBe('listening');
    expect(sessions).toHaveLength(1);
  });

  it('reconecta tras una caída sin duplicar herramientas ni operaciones', async () => {
    const { controller, backend, sessions } = setup();
    await controller.start(START);
    const first = sessions[0];
    // Una herramienta se ejecuta en la primera sesión.
    const addTask = first.input.tools[0] as unknown as { invoke: (ctx: unknown, input: string, d: unknown) => Promise<unknown> };
    await addTask.invoke({ context: {} }, JSON.stringify({ text: 'Llamar' }), { toolCall: { callId: 'call_1' } });
    first.emit('history_updated', [
      { type: 'message', role: 'user', content: [{ type: 'input_audio', transcript: 'Apunta llamar' }] },
    ]);

    first.drop();
    await vi.waitFor(() => expect(controller.getSnapshot().state.phase).toBe('listening'));
    expect(sessions).toHaveLength(2);
    expect(first.closed).toBe(true);
    expect(sessions[1].input.tools).toHaveLength(1);
    const carry = (backend.createRealtimeSession as ReturnType<typeof vi.fn>).mock.calls[1][1] as string;
    expect(carry).toContain('Apunta llamar');
    expect(carry).toContain('add_task: ok');
    expect(backend.invokeTool).toHaveBeenCalledTimes(1);

    // Los eventos tardíos de la sesión antigua se ignoran.
    first.server('output_audio_buffer.started');
    expect(controller.getSnapshot().state.phase).toBe('listening');
    // La transcripción previa se conserva.
    expect(controller.getSnapshot().transcript[0].text).toBe('Apunta llamar');
  });

  it('convierte fallos de conexión opacos en errores de red reintentables', async () => {
    const { controller, backend, sessions } = setup();
    let attempts = 0;
    const original = FakeSession.prototype.connect;
    FakeSession.prototype.connect = async function () {
      attempts++;
      throw new Event('error');
    };
    try {
      await controller.start(START);
    } finally {
      FakeSession.prototype.connect = original;
    }
    const err = controller.getSnapshot().state.error!;
    expect(err.code).toBe('network_error');
    expect(err.message).toContain('WebRTC');
    expect(attempts).toBe(5); // intento inicial + 4 reintentos
    expect(backend.createRealtimeSession).toHaveBeenCalledTimes(5);
    expect(sessions.every((s) => s.closed)).toBe(true);
  });

  it('el modo altavoz silencia el micro mientras JARVIS habla', async () => {
    const { controller, sessions } = setup();
    await controller.start({ ...START, halfDuplex: true });
    sessions[0].server('output_audio_buffer.started');
    sessions[0].server('output_audio_buffer.stopped');
    expect(sessions[0].muted).toEqual([true, false]);
  });
});
