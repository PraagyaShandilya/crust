import { create } from 'zustand';
import type { ChatEvent, ChatMessage, SessionSummary, ToolCall } from '../types/chat';

interface SessionState {
  sessions: SessionSummary[];
  activeSessionId?: string;
  messagesBySession: Record<string, ChatMessage[]>;
  setSessions: (sessions: SessionSummary[]) => void;
  setActiveSession: (sessionId: string) => void;
  setMessages: (sessionId: string, messages: ChatMessage[]) => void;
  appendMessage: (message: ChatMessage) => void;
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

  setSessions: (sessions) =>
    set((state) => ({
      sessions,
      activeSessionId: state.activeSessionId ?? sessions[0]?.id,
    })),

  setActiveSession: (sessionId) => set({ activeSessionId: sessionId }),

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

  applyEvent: (event) =>
    set((state) => {
      if (event.type === 'session.updated') {
        return {
          sessions: state.sessions.map((session) => (session.id === event.session.id ? event.session : session)),
        };
      }

      if (event.type === 'message.created') {
        return {
          messagesBySession: {
            ...state.messagesBySession,
            [event.message.sessionId]: [...(state.messagesBySession[event.message.sessionId] ?? []), event.message],
          },
        };
      }

      if (event.type === 'stream.completed' || event.type === 'stream.error') {
        return {
          sessions: state.sessions.map((session) =>
            session.id === event.sessionId ? { ...session, status: event.type === 'stream.error' ? 'error' : 'idle' } : session,
          ),
        };
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

      return { messagesBySession };
    }),
}));
