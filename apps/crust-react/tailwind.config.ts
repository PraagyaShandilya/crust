import type { Config } from 'tailwindcss';
import typography from '@tailwindcss/typography';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        crust: {
          ink: '#050505',
          panel: '#0b0b0b',
          coal: '#111111',
          line: '#2a1608',
          mint: '#ff6a00',
          amber: '#ff9a1f',
          ember: '#ff3d00',
          text: {
            DEFAULT: '#fff7ed',
            muted: '#ffedd5',
            subtle: '#fdba74',
          },
        },
      },
      fontFamily: {
        sans: ['Inter', 'ui-sans-serif', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'ui-monospace', 'SFMono-Regular', 'monospace'],
      },
    },
  },
  plugins: [typography],
} satisfies Config;
