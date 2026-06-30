export type SessionStatus = 'idle' | 'running' | 'error';

export type ChatRole = 'system' | 'user' | 'assistant' | 'tool';

export interface SessionSummary {
  id: string;
  title: string;
  updatedAt: string;
  status: SessionStatus;
  tokenCount?: number;
  coreKind?: string;
}

export interface ToolCall {
  id: string;
  name: string;
  status: 'pending' | 'running' | 'success' | 'error';
  input?: unknown;
  output?: string;
  error?: string;
}

export interface ThinkingState {
  summary: string;
  startedAt: string;
}

export interface ChatMessage {
  id: string;
  sessionId: string;
  role: ChatRole;
  content: string;
  createdAt: string;
  thinking?: ThinkingState;
  toolCalls?: ToolCall[];
}

export type ChatEvent =
  | { type: 'message.created'; message: ChatMessage }
  | { type: 'message.delta'; messageId: string; delta: string }
  | { type: 'thinking.delta'; messageId: string; delta: string; kind: string }
  | { type: 'thinking.updated'; messageId: string; thinking: ThinkingState }
  | { type: 'tool_call.updated'; messageId: string; toolCall: ToolCall }
  | { type: 'session.updated'; session: SessionSummary }
  | { type: 'stream.completed'; sessionId: string }
  | { type: 'stream.error'; sessionId: string; message: string };

export interface SendMessageRequest {
  content: string;
}

export interface SessionDetail {
  session: SessionSummary;
  messages: ChatMessage[];
}

export interface CoreInfo {
  kind: string;
  display_name: string;
  description: string;
  interaction_mode: string;
  default_model: string | null;
}

export interface ModelInfo {
  id: string;
  name: string;
}

export interface CrustSettings {
  default_core: string;
  default_model: string | null;
  max_agent_steps: number | null;
}

export interface SpaceInfo {
  id: string;
  name: string;
  session_id: string;
  cwd: string;
  status: string;
  task: string | null;
  created_at: string;
  updated_at: string;
}

export interface SkillInfo {
  name: string;
  description: string;
}

export interface PendingApproval {
  id: string;
  session_id: string;
  tool_name: string;
  args: string;
}

export interface ContextInfo {
  model: string;
  context_window: number;
  context_messages: number;
  estimated_tokens: number;
  estimated_ratio: number;
  last_prompt_tokens: number;
  last_api_ratio: number;
  has_summary: boolean;
  compacted_until: number;
  total_messages: number;
}
