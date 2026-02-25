import '../src/index.css'
import type { Preview } from '@storybook/react-vite'

const COMMON_VIEWPORTS = {
  mobile390: {
    name: 'Mobile 390',
    styles: { width: '390px', height: '844px' },
    type: 'mobile',
  },
  mobile430: {
    name: 'Mobile 430',
    styles: { width: '430px', height: '932px' },
    type: 'mobile',
  },
  tablet768: {
    name: 'Tablet 768',
    styles: { width: '768px', height: '1024px' },
    type: 'tablet',
  },
  laptop1024: {
    name: 'Laptop 1024',
    styles: { width: '1024px', height: '768px' },
    type: 'desktop',
  },
  desktop1280: {
    name: 'Desktop 1280',
    styles: { width: '1280px', height: '900px' },
    type: 'desktop',
  },
  desktop1440: {
    name: 'Desktop 1440',
    styles: { width: '1440px', height: '900px' },
    type: 'desktop',
  },
}

const preview: Preview = {
  parameters: {
    layout: 'fullscreen',
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    a11y: {
      // 'todo' - show a11y violations in the test UI only
      // 'error' - fail CI on a11y violations
      // 'off' - skip a11y checks entirely
      test: 'todo',
    },
    viewport: {
      options: COMMON_VIEWPORTS,
    },
  },
  initialGlobals: {
    viewport: { value: 'desktop1280', isRotated: false },
  },
}

export default preview
