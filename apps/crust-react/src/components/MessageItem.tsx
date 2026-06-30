import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { ChatMessage, ToolCall } from '../types/chat';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCallCard } from './ToolCallCard';

export function MessageItem({ message }: { message: ChatMessage }) {
  const isUser = message.role === 'user';
  const isTool = message.role === 'tool';
  const isAssistant = message.role === 'assistant';
  const hasToolCalls = message.toolCalls && message.toolCalls.length > 0;
  const isFinalAssistant = isAssistant && !hasToolCalls && message.content.length > 0;

  if (isTool) {
    return null;
  }

  if (isAssistant && !message.content && !message.thinking && !hasToolCalls) {
    return null;
  }

  if (isUser) {
    return (
      <article className="flex justify-end">
        <div className="max-w-[80%] rounded-2xl border border-crust-mint/60 bg-crust-mint/10 px-4 py-2 shadow-lg shadow-crust-ember/10">
          <p className="whitespace-pre-wrap text-sm leading-6 text-crust-text">{message.content}</p>
        </div>
      </article>
    );
  }

  return (
    <div className="grid grid-cols-[1rem_minmax(0,1fr)] gap-x-3">
      <div className="flex flex-col items-center pt-1">
        <span className={`h-2 w-2 rounded-full ${message.thinking ? 'bg-crust-amber' : hasToolCalls ? 'bg-crust-mint' : 'bg-crust-text-muted/40'}`} />
        <span className="mt-1 min-h-0 flex-1 border-l border-crust-mint/15" />
        {isFinalAssistant && <span className="mb-1 h-2 w-2 rounded-full bg-crust-mint" />}
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        {message.thinking && <ThinkingBlock thinking={message.thinking} />}
        {hasToolCalls && message.toolCalls!.map((toolCall) => (
          <ToolCallCard key={toolCall.id} toolCall={toolCall} />
        ))}
        {isFinalAssistant && message.content && (
          <div className="rounded-2xl border border-crust-mint/30 bg-crust-coal px-4 py-3">
            <div className="mb-2 flex items-center gap-2 text-[0.65rem] uppercase tracking-[0.2em] text-crust-mint/70">
              <span className="h-px flex-1 bg-crust-mint/20" />
              <span>Final Response</span>
              <span className="h-px flex-1 bg-crust-mint/20" />
            </div>
            <div className="prose prose-invert prose-sm max-w-none prose-headings:text-crust-text prose-p:text-crust-text prose-a:text-crust-mint prose-code:rounded prose-code:bg-crust-panel/60 prose-code:px-1 prose-code:py-0.5 prose-code:text-crust-amber prose-pre:rounded-lg prose-pre:border prose-pre:border-crust-mint/15 prose-pre:bg-crust-panel/40">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
            </div>
          </div>
        )}
        {isAssistant && !isFinalAssistant && message.content && (
          <p className="whitespace-pre-wrap px-1 text-sm leading-6 text-crust-text-muted">{message.content}</p>
        )}
      </div>
    </div>
  );
}
