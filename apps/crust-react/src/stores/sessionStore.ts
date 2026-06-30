import { create } from 'zustand';
import type { ChatEvent, ChatMessage, SessionSummary, ToolCall } from '../types/chat';

export type AgentState = 'idle' | 'thinking' | 'tool' | 'done' | 'error';

interface SessionState {
  sessions: SessionSummary[];
  activeSessionId?: string;
  messagesBySession: Record<string, ChatMessage[]>;
  agentState: AgentState;
  setSessions: (sessions: SessionSummary[]) => void;
  setActiveSession: (sessionId: string) => void;
  setMessages: (sessionId: string, messages: ChatMessage[]) => void;
  appendMessage: (message: ChatMessage) => void;
  removeSession: (sessionId: string) => void;
  updateSession: (sessionId: string, updates: Partial<SessionSummary>) => void;
  applyEvent: (event: ChatEvent) => void;
}

function upsertToolCall(toolCalls: ToolCall[] | undefined, next: ToolCall): ToolCall[] {
  const existing = toolCalls ?? [];
  const index = existing.findIndex((toolCall) => toolCall.id === next.id);

  if (index === -1) {
    return [...existing, next];
  }

  return existing.map((toolCall) => (toolCall.id === next.id ? next : toolCall));
}

export const useSessionStore = create<SessionState>((set) => ({
  sessions: [],
  messagesBySession: {},
  agentState: 'idle',

  setSessions: (sessions) =>
    set((state) => ({
      sessions,
      activeSessionId: state.activeSessionId ?? sessions[0]?.id,
    })),

  setActiveSession: (sessionId) => set({ activeSessionId: sessionId, agentState: 'idle' }),

  setMessages: (sessionId, messages) =>
    set((state) => ({
      messagesBySession: {
        ...state.messagesBySession,
        [sessionId]: messages,
      },
    })),

  appendMessage: (message) =>
    set((state) => ({
      messagesBySession: {
        ...state.messagesBySession,
        [message.sessionId]: [...(state.messagesBySession[message.sessionId] ?? []), message],
      },
    })),

  removeSession: (sessionId) =>
    set((state) => {
      const sessions = state.sessions.filter((s) => s.id !== sessionId);
      const messagesBySession = { ...state.messagesBySession };
      delete messagesBySession[sessionId];
      const activeSessionId =
        state.activeSessionId === sessionId
          ? sessions[0]?.id
          : state.activeSessionId;
      return { sessions, messagesBySession, activeSessionId };
    }),

  updateSession: (sessionId, updates) =>
    set((state) => ({
      sessions: state.sessions.map((s) =>
        s.id === sessionId ? { ...s, ...updates } : s,
      ),
    })),

  applyEvent: (event) =>
    set((state) => {
      if (event.type === 'session.updated') {
        return {
          sessions: state.sessions.map((session) => (session.id === event.session.id ? event.session : session)),
        };
      }

      let agentState = state.agentState;

      if (event.type === 'message.created') {
        const existing = state.messagesBySession[event.message.sessionId] ?? [];
        if (existing.some((m) => m.id === event.message.id)) {
          return {};
        }
        if (event.message.role === 'user') {
          agentState = 'thinking';
        }
        return {
          agentState,
          messagesBySession: {
            ...state.messagesBySession,
            [event.message.sessionId]: [...existing, event.message],
          },
        };
      }

      if (event.type === 'thinking.delta' || event.type === 'thinking.updated') {
        agentState = 'thinking';
      }

      if (event.type === 'tool_call.updated') {
        agentState = 'tool';
      }

      if (event.type === 'stream.completed') {
        agentState = 'done';
      }

      if (event.type === 'stream.error') {
        agentState = 'error';
      }

      const messagesBySession = Object.fromEntries(
        Object.entries(state.messagesBySession).map(([sessionId, messages]) => [
          sessionId,
          messages.map((message) => {
            if ('messageId' in event && message.id !== event.messageId) {
              return message;
            }

            if (event.type === 'message.delta') {
              return { ...message, content: `${message.content}${event.delta}` };
            }

            if (event.type === 'thinking.delta') {
              const existing = message.thinking;
              return {
                ...message,
                thinking: {
                  summary: existing ? `${existing.summary}${event.delta}` : event.delta,
                  startedAt: existing?.startedAt ?? new Date().toISOString(),
                },
              };
            }

            if (event.type === 'thinking.updated') {
              return { ...message, thinking: event.thinking };
            }

            if (event.type === 'tool_call.updated') {
              return { ...message, toolCalls: upsertToolCall(message.toolCalls, event.toolCall) };
            }

            return message;
          }),
        ]),
      );

      return { messagesBySession, agentState };
    }),
}));
