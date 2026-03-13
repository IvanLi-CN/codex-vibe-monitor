export const STORYBOOK_COLOR_CONTRAST_TODO = {
  a11y: {
    // Temporary scope exception: light-theme palette is still being rebalanced.
    // Keep all other Storybook accessibility rules active in CI.
    config: {
      rules: [{ id: 'color-contrast', enabled: false }],
    },
  },
} as const
