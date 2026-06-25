import type { ChatEvent, ChatMessage, SendMessageRequest, SessionDetail, SessionSummary } from '../types/chat';
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

  return (await response.json()) as T;
}

export const crustApi = {
  async listSessions(): Promise<SessionSummary[]> {
    try {
      return await requestJson<SessionSummary[]>('/sessions');
    } catch {
      return [mockSession.session];
    }
  },

  async getSession(sessionId: string): Promise<SessionDetail> {
    try {
      return await requestJson<SessionDetail>(`/sessions/${sessionId}`);
    } catch {
      return sessionId === mockSession.session.id ? mockSession : { ...mockSession, session: { ...mockSession.session, id: sessionId } };
    }
  },

  async sendMessage(sessionId: string, body: SendMessageRequest): Promise<ChatMessage> {
    try {
      return await requestJson<ChatMessage>(`/sessions/${sessionId}/messages`, {
        method: 'POST',
        body: JSON.stringify(body),
      });
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

  streamSession(sessionId: string, onEvent: (event: ChatEvent) => void, onError?: (error: Error) => void) {
    return openJsonEventStream<ChatEvent>(`${API_BASE_URL}/sessions/${sessionId}/events`, onEvent, onError);
  },
};
