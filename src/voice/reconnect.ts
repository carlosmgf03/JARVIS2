// Política de reintentos ante desconexiones.

import type { ErrorCode, JarvisError } from '../lib/types';

export const MAX_RECONNECT_ATTEMPTS = 4;

const RETRYABLE: ErrorCode[] = [
  'network_error',
  'provider_error',
  'rate_limited',
  'timeout',
  'connection_lost',
];

export function shouldRetry(error: JarvisError, attempt: number): boolean {
  return attempt < MAX_RECONNECT_ATTEMPTS && (error.retryable || RETRYABLE.includes(error.code));
}

/** Espera exponencial con algo de aleatoriedad: ~1 s, 2 s, 4 s, 8 s (máx. 10 s). */
export function backoffMs(attempt: number, random: () => number = Math.random): number {
  const base = Math.min(1000 * 2 ** Math.max(0, attempt - 1), 10_000);
  return Math.round(base * (0.8 + random() * 0.4));
}
