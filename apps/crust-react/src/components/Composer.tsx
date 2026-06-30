import { FormEvent, useMemo, useRef, useState } from 'react';

interface ComposerProps {
  disabled: boolean;
  onSend: (content: string) => void;
}

interface SlashCommand {
  name: string;
  description: string;
}

const SLASH_COMMANDS: SlashCommand[] = [
  { name: '/new', description: 'Create a new session' },
  { name: '/clear', description: 'Clear messages in the current session' },
  { name: '/compact', description: 'Compact old messages into a summary' },
  { name: '/context', description: 'Show context status for the session' },
  { name: '/delete', description: 'Delete the current session' },
  { name: '/switch', description: 'Switch to another session' },
  { name: '/agents', description: 'List active scoped agents' },
  { name: '/agent', description: 'Spawn a new scoped agent' },
  { name: '/agent-cancel', description: 'Cancel a scoped agent' },
  { name: '/agent-result', description: 'Fetch a scoped agent result' },
  { name: '/skills', description: 'List available skills' },
  { name: '/skill', description: 'Invoke a skill by name' },
  { name: '/langgraph', description: 'LangGraph commands' },
  { name: '/langgraph list', description: 'List LangGraph graphs' },
  { name: '/langgraph add', description: 'Register a new LangGraph graph' },
  { name: '/langgraph run', description: 'Start a LangGraph run' },
  { name: '/langgraph runs', description: 'List recent LangGraph runs' },
  { name: '/langgraph result', description: 'Fetch a LangGraph run result' },
  { name: '/langgraph cancel', description: 'Cancel a LangGraph run' },
  { name: '/spaces', description: 'List workspaces' },
  { name: '/space-create', description: 'Create a new workspace' },
  { name: '/space-attach', description: 'Attach a session to a workspace' },
  { name: '/space-stop', description: 'Stop a workspace' },
  { name: '/settings', description: 'Open settings' },
  { name: '/exit', description: 'Exit the application' },
];

const PREFIX_FILL_COMMANDS = new Set(['/skill', '/langgraph run']);

export function Composer({ disabled, onSend }: ComposerProps) {
  const [content, setContent] = useState('');
  const historyRef = useRef<string[]>([]);
  const historyIndexRef = useRef<number | null>(null);
  const draftRef = useRef('');
  const [slashIndex, setSlashIndex] = useState(0);

  const trimmedContent = content.trimStart();
  const slashMatches = useMemo(() => {
    if (!trimmedContent.startsWith('/') || /\s/.test(trimmedContent)) {
      return [];
    }
    const query = trimmedContent.toLowerCase();
    return SLASH_COMMANDS.filter((cmd) => cmd.name.toLowerCase().startsWith(query));
  }, [trimmedContent]);

  const slashOpen = slashMatches.length > 0;
  const effectiveSlashIndex = slashIndex >= slashMatches.length ? 0 : slashIndex;

  function send(text: string) {
    const trimmed = text.trim();
    if (!trimmed || disabled) return;
    historyRef.current = historyRef.current.filter((h) => h !== trimmed);
    historyRef.current.push(trimmed);
    historyIndexRef.current = null;
    draftRef.current = '';
    onSend(trimmed);
    setContent('');
    setSlashIndex(0);
  }

  function selectCommand(cmd: SlashCommand) {
    const fill = PREFIX_FILL_COMMANDS.has(cmd.name) ? `${cmd.name} ` : `${cmd.name} `;
    setContent(fill);
    setSlashIndex(0);
  }

  function navigateHistory(direction: 'up' | 'down') {
    const history = historyRef.current;
    if (history.length === 0) return;

    if (direction === 'up') {
      if (historyIndexRef.current === null) {
        draftRef.current = content;
        historyIndexRef.current = history.length - 1;
      } else if (historyIndexRef.current > 0) {
        historyIndexRef.current -= 1;
      }
    } else {
      if (historyIndexRef.current === null) return;
      if (historyIndexRef.current < history.length - 1) {
        historyIndexRef.current += 1;
      } else {
        historyIndexRef.current = null;
        setContent(draftRef.current);
        return;
      }
    }

    const entry = history[historyIndexRef.current];
    if (entry !== undefined) {
      setContent(entry);
    }
  }

  return (
    <form className="shrink-0 border-t border-crust-mint/20 bg-crust-panel/90 p-4 sm:p-5" onSubmit={(e: FormEvent<HTMLFormElement>) => { e.preventDefault(); send(content); }}>
      <div className="relative mx-auto max-w-6xl">
        {slashOpen && (
          <div className="absolute bottom-full left-0 right-0 mb-2 max-h-64 overflow-y-auto rounded-xl border border-crust-mint/25 bg-crust-coal shadow-xl shadow-crust-ember/20">
            {slashMatches.map((cmd, i) => (
              <button
                key={cmd.name}
                type="button"
                className={`flex w-full items-center justify-between gap-3 border-b border-crust-mint/10 px-3 py-2 text-left last:border-b-0 ${
                  i === effectiveSlashIndex ? 'bg-crust-mint/15' : 'hover:bg-crust-mint/5'
                }`}
                onMouseEnter={() => setSlashIndex(i)}
                onClick={() => selectCommand(cmd)}
              >
                <span className="font-mono text-xs text-crust-mint">{cmd.name}</span>
                <span className="truncate text-xs text-crust-text-muted/70">{cmd.description}</span>
              </button>
            ))}
          </div>
        )}
        <div className="rounded-2xl border border-crust-mint/25 bg-crust-coal p-2 shadow-xl shadow-crust-ember/10">
          <textarea
            className="min-h-20 w-full resize-none rounded-xl bg-transparent px-3 py-2 text-crust-text outline-none placeholder:text-crust-text-muted/30"
            placeholder="Ask Crust to inspect, edit, or explain... (type / for commands)"
            value={content}
            onChange={(event) => {
              setContent(event.target.value);
              if (historyIndexRef.current !== null) {
                draftRef.current = event.target.value;
              }
              setSlashIndex(0);
            }}
            onKeyDown={(event) => {
              if (slashOpen) {
                if (event.key === 'ArrowDown') {
                  event.preventDefault();
                  setSlashIndex((prev) => (prev + 1) % slashMatches.length);
                  return;
                }
                if (event.key === 'ArrowUp') {
                  event.preventDefault();
                  setSlashIndex((prev) => (prev - 1 + slashMatches.length) % slashMatches.length);
                  return;
                }
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault();
                  selectCommand(slashMatches[effectiveSlashIndex]);
                  return;
                }
                if (event.key === 'Escape') {
                  event.preventDefault();
                  setSlashIndex(0);
                  setContent('');
                  return;
                }
              }

              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                send(content);
              } else if (event.key === 'ArrowUp' && !event.shiftKey) {
                event.preventDefault();
                navigateHistory('up');
              } else if (event.key === 'ArrowDown' && !event.shiftKey) {
                event.preventDefault();
                navigateHistory('down');
              }
            }}
          />
          <div className="flex items-center justify-between gap-3 border-t border-crust-mint/15 pt-2">
            <span className="px-2 text-xs text-crust-text-muted/45">Enter to send · Shift+Enter for newline · ↑/↓ for history · / for commands</span>
            <button
              className="rounded-xl bg-crust-mint px-4 py-2 font-semibold text-crust-ink shadow-lg shadow-crust-ember/30 disabled:cursor-not-allowed disabled:opacity-60"
              disabled={disabled || !content.trim()}
              type="submit"
            >
              {disabled ? 'Sending...' : 'Send'}
            </button>
          </div>
        </div>
      </div>
    </form>
  );
}
