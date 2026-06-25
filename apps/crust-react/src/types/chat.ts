export type SessionStatus = 'idle' | 'running' | 'error';

export type ChatRole = 'system' | 'user' | 'assistant' | 'tool';

export interface SessionSummary {
  id: string;
  title: string;
  updatedAt: string;
  status: SessionStatus;
  tokenCount?: number;
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
