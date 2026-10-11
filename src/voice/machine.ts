// Máquina de estados de la conversación de voz (pura y testeable).

import type { JarvisError } from '../lib/types';

export type Phase =
  | 'idle' // sin sesión, micrófono apagado
  | 'requesting_mic'
  | 'connecting'
  | 'listening' // sesión activa, esperando al usuario
  | 'user_speaking'
  | 'processing' // el modelo prepara respuesta o se ejecuta una herramienta
  | 'speaking' // JARVIS está hablando
  | 'reconnecting'
  | 'error';

export interface VoiceState {
  phase: Phase;
  micMuted: boolean;
  activeTool: string | null;
  awaitingApproval: boolean;
  reconnectAttempt: number;
  error: JarvisError | null;
  transport: 'webrtc' | 'websocket' | null;
}

export type VoiceEvent =
  | { type: 'START' }
  | { type: 'MIC_GRANTED' }
  | { type: 'CONNECTED'; transport: 'webrtc' | 'websocket' }
  | { type: 'SPEECH_STARTED' }
  | { type: 'SPEECH_STOPPED' }
  | { type: 'RESPONSE_STARTED' }
  | { type: 'AUDIO_STARTED' }
  | { type: 'AUDIO_STOPPED' }
  | { type: 'RESPONSE_DONE' }
  | { type: 'INTERRUPTED' }
  | { type: 'TOOL_STARTED'; name: string }
  | { type: 'TOOL_FINISHED' }
  | { type: 'APPROVAL_PENDING'; pending: boolean }
  | { type: 'MUTE'; muted: boolean }
  | { type: 'CONNECTION_LOST'; attempt: number }
  | { type: 'FAILED'; error: JarvisError }
  | { type: 'STOPPED' };

export const initialVoiceState: VoiceState = {
  phase: 'idle',
  micMuted: false,
  activeTool: null,
  awaitingApproval: false,
  reconnectAttempt: 0,
  error: null,
  transport: null,
};

const ACTIVE: Phase[] = ['listening', 'user_speaking', 'processing', 'speaking'];

export function isSessionActive(s: VoiceState): boolean {
  return ACTIVE.includes(s.phase);
}

export function isBusy(s: VoiceState): boolean {
  return s.phase === 'requesting_mic' || s.phase === 'connecting' || s.phase === 'reconnecting';
}

export function voiceReducer(s: VoiceState, e: VoiceEvent): VoiceState {
  switch (e.type) {
    case 'START':
      return { ...initialVoiceState, phase: 'requesting_mic' };
    case 'MIC_GRANTED':
      return s.phase === 'requesting_mic' || s.phase === 'reconnecting' ? { ...s, phase: 'connecting' } : s;
    case 'CONNECTED':
      return { ...s, phase: 'listening', transport: e.transport, reconnectAttempt: 0, error: null };
    case 'STOPPED':
      return { ...initialVoiceState };
    case 'FAILED':
      return { ...initialVoiceState, phase: 'error', error: e.error };
    case 'CONNECTION_LOST':
      return { ...s, phase: 'reconnecting', reconnectAttempt: e.attempt, activeTool: null };
    case 'MUTE':
      return { ...s, micMuted: e.muted };
    case 'APPROVAL_PENDING':
      return { ...s, awaitingApproval: e.pending };
    default:
      break;
  }
  if (!isSessionActive(s)) return s;
  switch (e.type) {
    case 'SPEECH_STARTED':
      // El usuario habla: si JARVIS hablaba, el servidor interrumpe la respuesta.
      return { ...s, phase: 'user_speaking' };
    case 'SPEECH_STOPPED':
    case 'RESPONSE_STARTED':
      return s.phase === 'speaking' ? s : { ...s, phase: 'processing' };
    case 'AUDIO_STARTED':
      return { ...s, phase: 'speaking' };
    case 'AUDIO_STOPPED':
    case 'INTERRUPTED':
      return s.phase === 'speaking' ? { ...s, phase: s.activeTool ? 'processing' : 'listening' } : s;
    case 'RESPONSE_DONE':
      return s.phase === 'processing' && !s.activeTool ? { ...s, phase: 'listening' } : s;
    case 'TOOL_STARTED':
      return { ...s, activeTool: e.name, phase: s.phase === 'speaking' ? 'speaking' : 'processing' };
    case 'TOOL_FINISHED':
      return { ...s, activeTool: null };
    default:
      return s;
  }
}

export const PHASE_LABEL: Record<Phase, string> = {
  idle: 'En reposo',
  requesting_mic: 'Solicitando micrófono',
  connecting: 'Inicializando',
  listening: 'En espera',
  user_speaking: 'Escuchando',
  processing: 'Procesando',
  speaking: 'Respondiendo',
  reconnecting: 'Reconectando',
  error: 'Error',
};

/** Qué está haciendo JARVIS mientras ejecuta cada herramienta. */
export const TOOL_LABEL: Record<string, string> = {
  search_notes: 'Buscando en tus notas',
  read_note: 'Leyendo nota',
  list_tasks: 'Consultando tareas',
  list_projects: 'Consultando proyectos',
  daily_summary: 'Preparando el resumen del día',
  create_note: 'Creando nota',
  add_task: 'Añadiendo tarea',
  delegate_to_claude: 'Consultando a Claude',
  get_weather: 'Consultando el tiempo',
  web_search: 'Buscando en Internet',
  fetch_page: 'Leyendo la página',
  open_url: 'Abriendo el navegador',
  list_emails: 'Revisando tu correo',
  read_email: 'Leyendo el correo',
};
