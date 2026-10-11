// Conversión del historial del SDK a transcripción y resumen de continuidad.

import type { TranscriptEntry } from '../lib/types';

interface ContentPart {
  type?: string;
  text?: string | null;
  transcript?: string | null;
}

interface HistoryItem {
  type?: string;
  role?: string;
  itemId?: string;
  content?: ContentPart[];
}

/** Extrae los mensajes de usuario y asistente con texto o transcripción. */
export function historyToTranscript(history: readonly unknown[], now: () => Date = () => new Date()): TranscriptEntry[] {
  const out: TranscriptEntry[] = [];
  for (const raw of history) {
    const item = raw as HistoryItem;
    if (item?.type !== 'message' || (item.role !== 'user' && item.role !== 'assistant')) continue;
    const text = (item.content ?? [])
      .map((p) => (p.type === 'input_text' || p.type === 'output_text' ? p.text : p.transcript) ?? '')
      .join(' ')
      .trim();
    if (!text) continue;
    out.push({ role: item.role, text, ts: now().toISOString() });
  }
  return out;
}

export interface CompletedOperation {
  tool: string;
  status: string;
  detail?: string;
}

/**
 * Resumen compacto para continuar la conversación en una nueva sesión
 * (reconexión o cambio entre voz y texto). Incluye las operaciones ya
 * realizadas para que el modelo no las repita.
 */
export function carryOverSummary(
  entries: TranscriptEntry[],
  operations: CompletedOperation[] = [],
  maxChars = 2000,
): string | null {
  const lines: string[] = [];
  for (const e of entries.slice(-10)) {
    const text = e.text.length > 300 ? `${e.text.slice(0, 299)}…` : e.text;
    lines.push(`${e.role === 'user' ? 'Usuario' : 'JARVIS'}: ${text}`);
  }
  if (operations.length) {
    lines.push('Operaciones ya ejecutadas en esta conversación (no las repitas):');
    for (const op of operations.slice(-8)) {
      lines.push(`- ${op.tool}: ${op.status}${op.detail ? ` (${op.detail})` : ''}`);
    }
  }
  if (!lines.length) return null;
  let text = lines.join('\n');
  if (text.length > maxChars) text = `…${text.slice(text.length - maxChars + 1)}`;
  return text;
}
