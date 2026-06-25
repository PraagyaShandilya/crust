import { FormEvent, useState } from 'react';

interface ComposerProps {
  disabled: boolean;
  onSend: (content: string) => void;
}

export function Composer({ disabled, onSend }: ComposerProps) {
  const [content, setContent] = useState('');

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = content.trim();

    if (!trimmed) {
      return;
    }

    onSend(trimmed);
    setContent('');
  }

  return (
    <form className="border-t border-white/10 bg-crust-panel/80 p-4 sm:p-6" onSubmit={handleSubmit}>
      <div className="mx-auto max-w-3xl rounded-2xl border border-white/10 bg-crust-ink p-2 shadow-xl shadow-black/20">
        <textarea
          className="min-h-24 w-full resize-none rounded-xl bg-transparent px-3 py-2 text-slate-100 outline-none placeholder:text-slate-500"
          placeholder="Ask Crust to inspect, edit, or explain..."
          value={content}
          onChange={(event) => setContent(event.target.value)}
        />
        <div className="flex items-center justify-between gap-3 border-t border-white/10 pt-2">
          <span className="px-2 text-xs text-slate-500">Enter submits after the gateway behavior is finalized; use Send for this preview.</span>
          <button
            className="rounded-xl bg-crust-mint px-4 py-2 font-medium text-crust-ink disabled:cursor-not-allowed disabled:opacity-60"
            disabled={disabled || !content.trim()}
            type="submit"
          >
            {disabled ? 'Sending...' : 'Send'}
          </button>
        </div>
      </div>
    </form>
  );
}
