import { useEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../types/chat';
import { MessageItem } from './MessageItem';

interface MessageListProps {
  messages: ChatMessage[];
  loading: boolean;
}

export function MessageList({ messages, loading }: MessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [followingOutput, setFollowingOutput] = useState(true);

  useEffect(() => {
    if (!followingOutput) return;
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [followingOutput, messages]);

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={containerRef}
        className="h-full overflow-y-auto bg-[linear-gradient(180deg,rgba(255,106,0,0.04),rgba(5,5,5,0)_18rem)] px-5 py-6 sm:px-8"
        onScroll={(e) => {
          const el = e.currentTarget;
          setFollowingOutput(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
        }}
      >
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-4">
          {loading && <p className="text-sm text-crust-text-muted/60">Loading messages...</p>}
          {messages.map((message) => (
            <MessageItem key={message.id} message={message} />
          ))}
          <div ref={bottomRef} />
        </div>
      </div>
      <button
        className={`absolute bottom-4 right-5 rounded-full border px-3 py-1.5 text-xs shadow-lg transition sm:right-8 ${
          followingOutput
            ? 'border-crust-mint/30 bg-crust-coal/80 text-crust-mint'
            : 'border-crust-amber/40 bg-crust-coal text-crust-amber shadow-crust-ember/20 hover:border-crust-amber/70'
        }`}
        onClick={() => {
          setFollowingOutput(true);
          bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
        }}
        title={followingOutput ? 'Following output' : 'Output paused. Jump to latest'}
        aria-pressed={followingOutput}
      >
        {followingOutput ? 'Following' : 'Paused'}
      </button>
    </div>
  );
}
