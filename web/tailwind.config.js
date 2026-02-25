/** @type {import('tailwindcss').Config} */

const withOpacity = (token) => `oklch(var(${token}) / <alpha-value>)`

export default {
  content: ['./index.html', './src/**/*.{ts,tsx,js,jsx}'],
  theme: {
    extend: {
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
  plugins: [],
}
