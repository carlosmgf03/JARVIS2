// Herramientas de la sesión Realtime.
//
// El catálogo llega de la capa de orquestación (fuente única). Aquí solo se
// envuelve cada herramienta para que su ejecución se delegue en Rust, que
// valida argumentos, aplica la política de permisos, pide confirmación y
// ejecuta. El frontend nunca ejecuta operaciones por su cuenta.

import { tool } from '@openai/agents/realtime';
import type { Backend } from '../lib/backend';
import { toJarvisError } from '../lib/backend';
import type { ToolOutcome, ToolSpec } from '../lib/types';

export interface ToolHooks {
  onStart?: (name: string, callId: string) => void;
  onEnd?: (outcome: ToolOutcome) => void;
}

/** Lo que ve el modelo. Debe reflejar el estado real, nunca una suposición. */
export function outcomeForModel(o: ToolOutcome): string {
  if (o.status === 'ok') return JSON.stringify({ ok: true, verified: o.verified, result: o.data });
  return JSON.stringify({
    ok: false,
    denied: o.status === 'denied',
    error: o.error ? { code: o.error.code, message: o.error.message } : null,
    instruction:
      o.status === 'denied'
        ? 'La operación NO se ha realizado. Infórmalo al usuario sin afirmar lo contrario.'
        : 'La operación ha fallado. No digas que se ha completado.',
  });
}

function newCallId(): string {
  return `rt-${crypto.randomUUID()}`;
}

export async function executeViaBackend(
  backend: Backend,
  spec: ToolSpec,
  input: unknown,
  callId: string,
  hooks: ToolHooks,
): Promise<string> {
  hooks.onStart?.(spec.name, callId);
  let outcome: ToolOutcome;
  try {
    outcome = await backend.invokeTool(callId, spec.name, input);
  } catch (e) {
    const err = toJarvisError(e);
    outcome = {
      call_id: callId,
      tool: spec.name,
      status: 'error',
      data: null,
      error: err,
      verified: false,
      duration_ms: 0,
      deduplicated: false,
    };
  }
  hooks.onEnd?.(outcome);
  return outcomeForModel(outcome);
}

export function buildRealtimeTools(specs: ToolSpec[], backend: Backend, hooks: ToolHooks = {}) {
  return specs.map((spec) =>
    tool({
      name: spec.name,
      description: spec.description,
      // Esquema JSON tal cual lo define Rust; la validación estricta se hace allí.
      parameters: spec.parameters as never,
      strict: false,
      execute: async (input: unknown, _ctx?: unknown, details?: { toolCall?: { callId?: string } }) =>
        executeViaBackend(backend, spec, input, details?.toolCall?.callId || newCallId(), hooks),
    }),
  );
}
