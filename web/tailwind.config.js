/** @type {import('tailwindcss').Config} */

const withOpacity = (token) => `oklch(var(${token}) / <alpha-value>)`

export default {
  content: ['./index.html', './src/**/*.{ts,tsx,js,jsx}'],
  theme: {
    extend: {
      screens: {
        desktop1660: '1660px',
      },
      colors: {
        'base-100': withOpacity('--color-base-100'),
        'base-200': withOpacity('--color-base-200'),
        'base-300': withOpacity('--color-base-300'),
        'base-content': withOpacity('--color-base-content'),
        primary: withOpacity('--color-primary'),
        'primary-content': withOpacity('--color-primary-content'),
        secondary: withOpacity('--color-secondary'),
        'secondary-content': withOpacity('--color-secondary-content'),
        accent: withOpacity('--color-accent'),
        'accent-content': withOpacity('--color-accent-content'),
        neutral: withOpacity('--color-neutral'),
        'neutral-content': withOpacity('--color-neutral-content'),
        info: withOpacity('--color-info'),
        'info-content': withOpacity('--color-info-content'),
        success: withOpacity('--color-success'),
        'success-content': withOpacity('--color-success-content'),
        warning: withOpacity('--color-warning'),
        'warning-content': withOpacity('--color-warning-content'),
        error: withOpacity('--color-error'),
        'error-content': withOpacity('--color-error-content'),
      },
      borderRadius: {
        box: 'var(--radius-box)',
        btn: 'var(--radius-field)',
      },
      keyframes: {
        'signal-ring': {
          '0%': { transform: 'scale(0.72)', opacity: '0' },
          '18%': { opacity: '0.58' },
          '62%': { opacity: '0.18' },
          '100%': { transform: 'scale(1.92)', opacity: '0' },
        },
        'signal-glow': {
          '0%, 100%': { transform: 'scale(0.92)', opacity: '0.18' },
          '50%': { transform: 'scale(1.16)', opacity: '0.52' },
        },
        'signal-halo': {
          '0%, 100%': { transform: 'scale(0.86)', opacity: '0.16' },
          '50%': { transform: 'scale(1.18)', opacity: '0.44' },
        },
        'signal-core': {
          '0%, 100%': { transform: 'scale(1)' },
          '50%': { transform: 'scale(1.055)' },
        },
        'orbit-spin': {
          '0%': { transform: 'rotate(0deg)' },
          '100%': { transform: 'rotate(360deg)' },
        },
      },
      animation: {
        'signal-ring': 'signal-ring 2.3s cubic-bezier(0.16, 1, 0.3, 1) infinite',
        'signal-glow': 'signal-glow 2.6s cubic-bezier(0.25, 1, 0.5, 1) infinite',
        'signal-halo': 'signal-halo 2.2s cubic-bezier(0.25, 1, 0.5, 1) infinite',
        'signal-core': 'signal-core 2.2s cubic-bezier(0.25, 1, 0.5, 1) infinite',
        'orbit-spin': 'orbit-spin 1.15s linear infinite',
      },
    },
  },
  plugins: [],
}
