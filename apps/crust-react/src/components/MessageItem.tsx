import type { ChatMessage } from '../types/chat';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCallCard } from './ToolCallCard';

export function MessageItem({ message }: { message: ChatMessage }) {
  const isUser = message.role === 'user';

  return (
    <article className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div className={`max-w-[92%] rounded-2xl border p-4 sm:max-w-[80%] ${isUser ? 'border-crust-mint/40 bg-crust-mint/10' : 'border-white/10 bg-white/[0.04]'}`}>
        <div className="mb-2 flex items-center justify-between gap-3 text-xs uppercase tracking-[0.18em] text-slate-400">
          <span>{message.role}</span>
          <time>{new Date(message.createdAt).toLocaleTimeString()}</time>
        </div>
        {message.thinking && <ThinkingBlock thinking={message.thinking} />}
        <p className="whitespace-pre-wrap leading-7 text-slate-100">{message.content}</p>
        {message.toolCalls?.map((toolCall) => <ToolCallCard key={toolCall.id} toolCall={toolCall} />)}
      </div>
    </article>
  );
}
