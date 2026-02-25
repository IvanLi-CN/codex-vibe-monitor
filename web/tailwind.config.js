/** @type {import('tailwindcss').Config} */
import daisyui from 'daisyui'

export default {
  content: ['./index.html', './src/**/*.{ts,tsx,js,jsx}'],
  theme: {
    extend: {
      keyframes: {
        'pulse-ring': {
          '0%': { transform: 'scale(0.65)', opacity: '0.65' },
          '45%': { transform: 'scale(1.85)', opacity: '0.35' },
          '100%': { transform: 'scale(2.8)', opacity: '0' },
        },
        'pulse-glow': {
          '0%': { opacity: '0.45', boxShadow: '0 0 0 rgba(59, 130, 246, 0.0)' },
          '50%': { opacity: '0.9', boxShadow: '0 0 36px rgba(59, 130, 246, 0.55)' },
          '100%': { opacity: '0', boxShadow: '0 0 0 rgba(59, 130, 246, 0.0)' },
        },
        'pulse-core': {
          '0%': { transform: 'scale(1)', opacity: '1' },
          '50%': { transform: 'scale(1.12)', opacity: '0.75' },
          '100%': { transform: 'scale(1)', opacity: '1' },
        },
        'orbit-spin': {
          '0%': { transform: 'rotate(0deg)' },
          '100%': { transform: 'rotate(360deg)' },
        },
      },
      animation: {
        'pulse-ring': 'pulse-ring 1.4s ease-out',
        'pulse-glow': 'pulse-glow 1.4s ease-out',
        'pulse-core': 'pulse-core 1.4s ease-in-out',
        'orbit-spin': 'orbit-spin 1.1s linear infinite',
      },
    },
  },
  plugins: [daisyui],
  daisyui: {
    themes: ['light --default', 'dark --prefersdark'],
    styled: true,
    base: true,
    utils: true,
  },
}
