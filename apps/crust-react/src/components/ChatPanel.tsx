import { useEffect } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { crustApi } from '../api/client';
import { useSessionStore } from '../stores/sessionStore';
import { Composer } from './Composer';
import { MessageList } from './MessageList';

export function ChatPanel() {
  const activeSessionId = useSessionStore((state) => state.activeSessionId);
  const messages = useSessionStore((state) => (state.activeSessionId ? state.messagesBySession[state.activeSessionId] ?? [] : []));
  const setMessages = useSessionStore((state) => state.setMessages);
  const appendMessage = useSessionStore((state) => state.appendMessage);

  const sessionQuery = useQuery({
    queryKey: ['session', activeSessionId],
    queryFn: () => crustApi.getSession(activeSessionId!),
    enabled: Boolean(activeSessionId),
  });

  useEffect(() => {
    if (sessionQuery.data) {
      setMessages(sessionQuery.data.session.id, sessionQuery.data.messages);
    }
  }, [sessionQuery.data, setMessages]);

  const sendMutation = useMutation({
    mutationFn: (content: string) => crustApi.sendMessage(activeSessionId!, { content }),
    onSuccess: (message) => {
      appendMessage(message);
      appendMessage({
        id: `assistant-${crypto.randomUUID()}`,
        sessionId: message.sessionId,
        role: 'assistant',
        content: 'Gateway is not connected yet. This local reply confirms the React shell can accept input and render planned event-shaped messages.',
        createdAt: new Date().toISOString(),
        thinking: {
          summary: 'Waiting for the future gateway stream endpoint to replace this mock-safe response.',
          startedAt: new Date().toISOString(),
        },
      });
    },
  });

  if (!activeSessionId) {
    return <div className="grid flex-1 place-items-center text-slate-400">No session selected.</div>;
  }

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-white/10 px-4 py-4 sm:px-6">
        <p className="text-xs uppercase tracking-[0.24em] text-crust-mint">Phase 6 React Preview</p>
        <h2 className="mt-1 text-2xl font-semibold">Chat Console</h2>
        <p className="mt-1 text-sm text-slate-400">Gateway endpoints are abstracted behind API clients and safe local fallbacks.</p>
      </div>
      <MessageList loading={sessionQuery.isLoading} messages={messages} />
      <Composer disabled={sendMutation.isPending} onSend={(content) => sendMutation.mutate(content)} />
    </section>
  );
}
