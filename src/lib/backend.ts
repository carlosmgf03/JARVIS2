// Puente con la capa de orquestación (Rust). La interfaz no tiene acceso a
// claves, a la Vault ni a la red de los proveedores salvo la sesión Realtime
// abierta con un secreto efímero. Todo lo demás pasa por aquí.

import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  ApprovalEvent,
  ApprovalRequest,
  AppStatus,
  AuditEntry,
  JarvisError,
  RealtimeSessionResponse,
  TextReply,
  ToolOutcome,
  TranscriptEntry,
  UsageRecord,
  UsageSummary,
} from './types';

export interface Backend {
  readonly kind: 'tauri' | 'unavailable';
  status(): Promise<AppStatus>;
  createRealtimeSession(voice: string | null, carryOver: string | null): Promise<RealtimeSessionResponse>;
  invokeTool(callId: string, name: string, args: unknown): Promise<ToolOutcome>;
  decideApproval(id: string, approved: boolean): Promise<void>;
  pendingApprovals(): Promise<ApprovalRequest[]>;
  onApproval(cb: (ev: ApprovalEvent) => void): Promise<() => void>;
  sendText(message: string, recentContext: string | null): Promise<TextReply>;
  resetText(): Promise<void>;
  auditRecent(limit: number): Promise<AuditEntry[]>;
  saveTranscript(sessionId: string, entries: TranscriptEntry[]): Promise<string | null>;
  clearTranscripts(): Promise<number>;
  recordUsage(record: UsageRecord): Promise<void>;
  usageSummary(): Promise<UsageSummary>;
}

export class BackendError extends Error implements JarvisError {
  code: JarvisError['code'];
  retryable: boolean;
  constructor(e: JarvisError) {
    super(e.message);
    this.code = e.code;
    this.retryable = e.retryable;
  }
}

export function toJarvisError(e: unknown): JarvisError {
  if (e instanceof BackendError) return { code: e.code, message: e.message, retryable: e.retryable };
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
    const o = e as JarvisError;
    return { code: o.code, message: String(o.message), retryable: Boolean(o.retryable) };
  }
  return { code: 'internal', message: e instanceof Error ? e.message : String(e), retryable: false };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new BackendError(toJarvisError(e));
  }
}

class TauriBackend implements Backend {
  readonly kind = 'tauri' as const;
  status() {
    return call<AppStatus>('app_status');
  }
  createRealtimeSession(voice: string | null, carryOver: string | null) {
    return call<RealtimeSessionResponse>('realtime_session', { voice, carryOver });
  }
  invokeTool(callId: string, name: string, args: unknown) {
    return call<ToolOutcome>('tool_invoke', { callId, name, arguments: args ?? {} });
  }
  decideApproval(id: string, approved: boolean) {
    return call<void>('approval_decide', { id, approved });
  }
  pendingApprovals() {
    return call<ApprovalRequest[]>('approvals_pending');
  }
  async onApproval(cb: (ev: ApprovalEvent) => void) {
    return listen<ApprovalEvent>('jarvis://approval', (e) => cb(e.payload));
  }
  sendText(message: string, recentContext: string | null) {
    return call<TextReply>('text_send', { message, recentContext });
  }
  resetText() {
    return call<void>('text_reset');
  }
  auditRecent(limit: number) {
    return call<AuditEntry[]>('audit_recent', { limit });
  }
  saveTranscript(sessionId: string, entries: TranscriptEntry[]) {
    return call<string | null>('transcript_save', { sessionId, entries });
  }
  clearTranscripts() {
    return call<number>('transcripts_clear');
  }
  recordUsage(record: UsageRecord) {
    return call<void>('usage_record', { record });
  }
  usageSummary() {
    return call<UsageSummary>('usage_summary');
  }
}

const unavailable = (): never => {
  throw new BackendError({
    code: 'backend_unavailable',
    message:
      'La capa de orquestación no está disponible. Abre JARVIS con `npm run app:dev` (Tauri); en un navegador normal solo se muestra la interfaz de prueba.',
    retryable: false,
  });
};

/** Navegador sin Tauri: interfaz de prueba, sin voz real ni herramientas. */
class UnavailableBackend implements Backend {
  readonly kind = 'unavailable' as const;
  async status(): Promise<AppStatus> {
    return {
      backend: 'unavailable',
      config_error: null,
      status: {
        openai_key: 'missing',
        anthropic_key: 'missing',
        vault: { ok: false, path: null, error: 'Backend no disponible' },
        realtime_model: '—',
        default_voice: 'marin',
        voices: ['marin', 'cedar'],
        transcription_enabled: false,
        text_provider: 'commands',
        policy: { write: 'confirm', delegate: 'allow' },
        save_transcripts: false,
        transcript_retention_days: 0,
        voice_available: false,
        limitations: [
          'Se está ejecutando en un navegador sin la capa de orquestación (Tauri). No hay voz real ni acceso a la Vault.',
        ],
      },
    };
  }
  createRealtimeSession = async () => unavailable();
  invokeTool = async () => unavailable();
  decideApproval = async () => unavailable();
  pendingApprovals = async () => [];
  onApproval = async () => () => {};
  sendText = async () => unavailable();
  resetText = async () => {};
  auditRecent = async () => [];
  saveTranscript = async () => null;
  clearTranscripts = async () => 0;
  recordUsage = async () => {};
  usageSummary = async (): Promise<UsageSummary> => ({
    sessions: 0,
    requests: 0,
    input_tokens: 0,
    output_tokens: 0,
    total_tokens: 0,
    duration_seconds: 0,
  });
}

let instance: Backend | null = null;

export function getBackend(): Backend {
  if (!instance) instance = isTauri() ? new TauriBackend() : new UnavailableBackend();
  return instance;
}
