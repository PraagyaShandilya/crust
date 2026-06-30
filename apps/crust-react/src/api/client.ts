import type { ChatEvent, ChatMessage, ContextInfo, CoreInfo, CrustSettings, ModelInfo, PendingApproval, SendMessageRequest, SessionDetail, SessionSummary, SkillInfo, SpaceInfo } from '../types/chat';
import { openJsonEventStream } from './sse';

const API_BASE_URL = import.meta.env.VITE_CRUST_API_URL ?? '/api';

const now = new Date().toISOString();

export const mockSession: SessionDetail = {
  session: {
    id: 'local-preview',
    title: 'Local preview session',
    updatedAt: now,
    status: 'idle',
    tokenCount: 1248,
  },
  messages: [
    {
      id: 'welcome-user',
      sessionId: 'local-preview',
      role: 'user',
      content: 'Show me what the React surface will look like before the gateway exists.',
      createdAt: now,
    },
    {
      id: 'welcome-assistant',
      sessionId: 'local-preview',
      role: 'assistant',
      content: 'This scaffold is wired around planned session, message, tool-call, and stream events. Until the gateway lands, failed API calls stay local and safe.',
      createdAt: now,
      thinking: {
        summary: 'Planning a gateway-shaped UI without assuming the backend is present.',
        startedAt: now,
      },
      toolCalls: [
        {
          id: 'tool-preview',
          name: 'read_file_tool',
          status: 'success',
          input: { path: 'README.md' },
          output: 'Preview data only. Real tool calls will arrive through streamed gateway events.',
        },
      ],
    },
  ],
};

interface GatewaySession {
  id: string;
  name: string;
  created_at?: string;
  edited_at?: string;
  messages?: GatewayMessage[];
  latest_total_tokens?: number;
  cumulative_total_tokens?: number;
  core_profile?: string;
}

interface GatewayMessage {
  role?: string;
  content?: unknown;
}

interface TaggedAgentEvent {
  session_id: string;
  event: Record<string, unknown>;
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
    ...init,
  });

  if (!response.ok) {
    throw new Error(`Crust API ${response.status}: ${response.statusText}`);
  }

  if (response.status === 204 || response.headers.get('content-length') === '0') {
    return undefined as T;
  }

  const text = await response.text();
  if (!text) {
    return undefined as T;
  }

  return JSON.parse(text) as T;
}

function sessionSummaryFromGateway(session: GatewaySession): SessionSummary {
  return {
    id: session.id,
    title: session.name || session.id,
    updatedAt: session.edited_at || session.created_at || new Date().toISOString(),
    status: 'idle',
    tokenCount: session.latest_total_tokens ?? session.cumulative_total_tokens,
    coreKind: session.core_profile,
  };
}

function contentToText(content: unknown): string {
  if (typeof content === 'string') {
    return content;
  }

  if (content && typeof content === 'object' && 'Text' in content) {
    const text = (content as { Text?: unknown }).Text;
    return typeof text === 'string' ? text : JSON.stringify(text ?? '');
  }

  if (content && typeof content === 'object' && 'Parts' in content) {
    return JSON.stringify((content as { Parts?: unknown }).Parts ?? []);
  }

  return content == null ? '' : JSON.stringify(content);
}

function messageFromGateway(sessionId: string, message: GatewayMessage, index: number): ChatMessage {
  const role = String(message.role ?? 'assistant').toLowerCase();
  return {
    id: `${sessionId}-${index}`,
    sessionId,
    role: role === 'user' || role === 'assistant' || role === 'system' || role === 'tool' ? role : 'assistant',
    content: contentToText(message.content),
    createdAt: new Date().toISOString(),
  };
}

function sessionDetailFromGateway(session: GatewaySession): SessionDetail {
  return {
    session: sessionSummaryFromGateway(session),
    messages: (session.messages ?? [])
      .filter((message) => String(message.role ?? '').toLowerCase() !== 'system')
      .map((message, index) => messageFromGateway(session.id, message, index)),
  };
}

let streamingMessageId: string | undefined = undefined;
let streamingHasContent: boolean = false;

function resetStreaming() {
  streamingMessageId = undefined;
  streamingHasContent = false;
}

function startNewStreamingMessage(sessionId: string, events: ChatEvent[]) {
  streamingMessageId = `stream-${crypto.randomUUID()}`;
  streamingHasContent = false;
  events.push({
    type: 'message.created',
    message: {
      id: streamingMessageId,
      sessionId,
      role: 'assistant',
      content: '',
      createdAt: new Date().toISOString(),
    },
  });
}

function ensureStreamingMessage(sessionId: string, events: ChatEvent[]) {
  if (!streamingMessageId) {
    startNewStreamingMessage(sessionId, events);
  }
  return streamingMessageId!;
}

function eventFromGateway(tagged: TaggedAgentEvent): ChatEvent[] {
  const event = tagged.event;
  const type = String(event.type ?? '');
  const sessionId = tagged.session_id;
  const events: ChatEvent[] = [];

  if (type === 'user_submitted') {
    resetStreaming();
    events.push({
      type: 'message.created',
      message: {
        id: `user-${crypto.randomUUID()}`,
        sessionId,
        role: 'user',
        content: String(event.prompt ?? ''),
        createdAt: new Date().toISOString(),
      },
    });
    startNewStreamingMessage(sessionId, events);
    return events;
  }

  if (type === 'thinking') {
    if (streamingHasContent) {
      startNewStreamingMessage(sessionId, events);
    } else {
      ensureStreamingMessage(sessionId, events);
    }
    events.push({
      type: 'thinking.delta',
      messageId: streamingMessageId!,
      delta: String(event.text ?? ''),
      kind: String(event.kind ?? ''),
    });
    streamingHasContent = true;
    return events;
  }

  if (type === 'assistant_delta') {
    if (streamingHasContent) {
      startNewStreamingMessage(sessionId, events);
    } else {
      ensureStreamingMessage(sessionId, events);
    }
    events.push({
      type: 'message.delta',
      messageId: streamingMessageId!,
      delta: String(event.text ?? ''),
    });
    streamingHasContent = true;
    return events;
  }

  if (type === 'assistant_final') {
    if (streamingHasContent) {
      startNewStreamingMessage(sessionId, events);
    } else {
      ensureStreamingMessage(sessionId, events);
    }
    events.push({
      type: 'message.delta',
      messageId: streamingMessageId!,
      delta: String(event.text ?? ''),
    });
    streamingHasContent = true;
    return events;
  }

  if (type === 'tool_call_started') {
    ensureStreamingMessage(sessionId, events);
    events.push({
      type: 'tool_call.updated',
      messageId: streamingMessageId!,
      toolCall: {
        id: String(event.id ?? crypto.randomUUID()),
        name: String(event.name ?? 'unknown'),
        status: 'running',
        input: event.args ? tryParseJson(String(event.args)) : undefined,
      },
    });
    streamingHasContent = true;
    return events;
  }

  if (type === 'tool_call_finished') {
    events.push({
      type: 'tool_call.updated',
      messageId: streamingMessageId ?? `orphan-${crypto.randomUUID()}`,
      toolCall: {
        id: String(event.id ?? crypto.randomUUID()),
        name: String(event.name ?? 'unknown'),
        status: 'success',
        output: String(event.result ?? '').slice(0, 8000),
      },
    });
    return events;
  }

  if (type === 'error') {
    events.push({ type: 'stream.error', sessionId, message: String(event.message ?? 'Gateway error') });
    return events;
  }

  if (type === 'finished') {
    resetStreaming();
    events.push({ type: 'stream.completed', sessionId });
    return events;
  }

  return events;
}

function tryParseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

export const crustApi = {
  async createSession(name = 'React Session'): Promise<SessionSummary> {
    try {
      const session = await requestJson<GatewaySession>('/sessions', {
        method: 'POST',
        body: JSON.stringify({ name }),
      });
      return sessionSummaryFromGateway(session);
    } catch {
      return { ...mockSession.session, id: `local-${crypto.randomUUID()}`, title: name };
    }
  },

  async listSessions(): Promise<SessionSummary[]> {
    try {
      const sessions = await requestJson<GatewaySession[]>('/sessions');
      return sessions.map(sessionSummaryFromGateway);
    } catch {
      return [mockSession.session];
    }
  },

  async getSession(sessionId: string): Promise<SessionDetail> {
    try {
      const session = await requestJson<GatewaySession>(`/sessions/${sessionId}`);
      return sessionDetailFromGateway(session);
    } catch {
      return sessionId === mockSession.session.id ? mockSession : { ...mockSession, session: { ...mockSession.session, id: sessionId } };
    }
  },

  async sendMessage(sessionId: string, body: SendMessageRequest): Promise<ChatMessage> {
    try {
      await requestJson<{ session_id: string; status: string }>(`/sessions/${sessionId}/messages`, {
        method: 'POST',
        body: JSON.stringify({ prompt: body.content }),
      });
      return {
        id: `user-${crypto.randomUUID()}`,
        sessionId,
        role: 'user',
        content: body.content,
        createdAt: new Date().toISOString(),
      };
    } catch {
      return {
        id: `local-${crypto.randomUUID()}`,
        sessionId,
        role: 'user',
        content: body.content,
        createdAt: new Date().toISOString(),
      };
    }
  },

  async deleteSession(sessionId: string): Promise<void> {
    await requestJson<void>(`/sessions/${sessionId}`, { method: 'DELETE' });
  },

  async renameSession(sessionId: string, name: string): Promise<SessionSummary> {
    const session = await requestJson<GatewaySession>(`/sessions/${sessionId}`, {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    });
    return sessionSummaryFromGateway(session);
  },

  async clearSession(sessionId: string): Promise<void> {
    await requestJson(`/sessions/${sessionId}/clear`, { method: 'POST' });
  },

  async compactSession(sessionId: string): Promise<{ cutoff: number }> {
    return requestJson(`/sessions/${sessionId}/compact`, { method: 'POST' });
  },

  async getContext(sessionId: string): Promise<ContextInfo> {
    return requestJson(`/sessions/${sessionId}/context`);
  },

  streamSession(sessionId: string, onEvent: (event: ChatEvent) => void, onError?: (error: Error) => void) {
    resetStreaming();
    return openJsonEventStream<TaggedAgentEvent>(
      `${API_BASE_URL}/sessions/${sessionId}/events`,
      (tagged) => {
        const normalized = eventFromGateway(tagged);
        for (const event of normalized) {
          onEvent(event);
        }
      },
      onError,
    );
  },

  async listCores(): Promise<CoreInfo[]> {
    return requestJson<CoreInfo[]>('/cores');
  },

  async listModels(): Promise<ModelInfo[]> {
    return requestJson<ModelInfo[]>('/models');
  },

  async getSettings(): Promise<CrustSettings> {
    return requestJson<CrustSettings>('/settings');
  },

  async updateSettings(settings: CrustSettings): Promise<CrustSettings> {
    return requestJson<CrustSettings>('/settings', {
      method: 'PUT',
      body: JSON.stringify(settings),
    });
  },

  async listSpaces(): Promise<SpaceInfo[]> {
    const response = await requestJson<{ spaces: SpaceInfo[] }>('/spaces');
    return response.spaces;
  },

  async createSpace(id: string): Promise<void> {
    await requestJson<void>('/spaces', {
      method: 'POST',
      body: JSON.stringify({ id }),
    });
  },

  async stopSpace(id: string): Promise<void> {
    await requestJson<void>(`/spaces/${id}/stop`, { method: 'POST' });
  },

  async getSpace(id: string): Promise<SpaceInfo> {
    return requestJson<SpaceInfo>(`/spaces/${id}`);
  },

  async listSkills(): Promise<SkillInfo[]> {
    return requestJson<SkillInfo[]>('/skills');
  },

  async listApprovals(sessionId: string): Promise<PendingApproval[]> {
    return requestJson<PendingApproval[]>(`/sessions/${sessionId}/approvals`);
  },

  async approveApproval(sessionId: string, approvalId: string): Promise<void> {
    await requestJson<void>(`/sessions/${sessionId}/approvals/${approvalId}/approve`, { method: 'POST' });
  },

  async rejectApproval(sessionId: string, approvalId: string, reason: string): Promise<void> {
    await requestJson<void>(`/sessions/${sessionId}/approvals/${approvalId}/reject`, {
      method: 'POST',
      body: JSON.stringify({ reason }),
    });
  },
};
