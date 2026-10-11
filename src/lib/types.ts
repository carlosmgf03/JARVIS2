// Tipos compartidos con la capa de orquestación (crates/jarvis-core).

export type ErrorCode =
  | 'missing_api_key'
  | 'invalid_api_key'
  | 'rate_limited'
  | 'quota_exceeded'
  | 'model_unavailable'
  | 'provider_error'
  | 'network_error'
  | 'timeout'
  | 'invalid_arguments'
  | 'unknown_tool'
  | 'tool_unavailable'
  | 'permission_denied'
  | 'approval_denied'
  | 'approval_timeout'
  | 'vault_not_configured'
  | 'vault_io'
  | 'not_found'
  | 'already_exists'
  | 'verification_failed'
  | 'secret_store'
  | 'config'
  | 'internal'
  // Solo frontend:
  | 'backend_unavailable'
  | 'microphone_denied'
  | 'microphone_unavailable'
  | 'webrtc_unavailable'
  | 'playback_failed'
  | 'connection_lost';

export interface JarvisError {
  code: ErrorCode;
  message: string;
  retryable: boolean;
}

export type RiskLevel = 'read' | 'write' | 'delegate' | 'web' | 'open_url' | 'email';
export type SecretSource = 'environment' | 'keychain' | 'missing';

export interface ToolSpec {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  risk: RiskLevel;
}

export interface Status {
  openai_key: SecretSource;
  anthropic_key: SecretSource;
  vault: { ok: boolean; path: string | null; error: string | null };
  realtime_model: string;
  default_voice: string;
  voices: string[];
  transcription_enabled: boolean;
  text_provider: 'claude' | 'commands';
  policy: Record<'write' | 'delegate' | 'web' | 'open' | 'email', 'allow' | 'confirm' | 'deny'>;
  save_transcripts: boolean;
  transcript_retention_days: number;
  web_search: boolean;
  email: string | null;
  weather_location: string | null;
  traffic: boolean;
  traffic_city: string;
  flights: boolean;
  voice_available: boolean;
  limitations: string[];
}

export interface AppStatus {
  status: Status;
  config_error: string | null;
  backend: 'tauri' | 'unavailable';
}

export interface SessionSettings {
  model: string;
  voice: string;
  language: string;
  transcription_model: string | null;
  turn_detection: Record<string, unknown>;
  noise_reduction: string | null;
}

export interface RealtimeSessionResponse {
  session: { ephemeral_key: string; expires_at: number; settings: SessionSettings };
  instructions: string;
  tools: ToolSpec[];
}

export type OutcomeStatus = 'ok' | 'error' | 'denied';

export interface ToolOutcome {
  call_id: string;
  tool: string;
  status: OutcomeStatus;
  data: unknown;
  error: JarvisError | null;
  verified: boolean;
  duration_ms: number;
  deduplicated: boolean;
}

export interface ApprovalRequest {
  id: string;
  call_id: string;
  tool: string;
  summary: string;
  preview: string | null;
  risk: RiskLevel;
  origin: string;
  expires_at_ms: number;
}

export type ApprovalEvent =
  | { kind: 'requested'; request: ApprovalRequest }
  | { kind: 'resolved'; id: string; outcome: 'approved' | 'denied' | 'timed_out' };

export interface TextReply {
  reply: string;
  provider: 'claude' | 'commands';
  tools: { tool: string; status: OutcomeStatus; code: ErrorCode | null }[];
}

export interface AuditEntry {
  ts: string;
  call_id: string;
  tool: string;
  origin: string;
  status: string;
  code: ErrorCode | null;
  duration_ms: number;
  target: string | null;
  deduplicated: boolean;
}

export interface TranscriptEntry {
  role: 'user' | 'assistant';
  text: string;
  ts: string;
}

export interface UsageRecord {
  model: string;
  requests: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  duration_seconds: number;
}

export interface UsageSummary extends Omit<UsageRecord, 'model'> {
  sessions: number;
}
