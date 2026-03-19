import { defineConfig } from 'rspress/config'

function normalizeBase(base: string | undefined): string {
  const raw = (base ?? '/').trim()
  if (!raw || raw === '/') return '/'
  const withLeading = raw.startsWith('/') ? raw : `/${raw}`
  return withLeading.endsWith('/') ? withLeading : `${withLeading}/`
}

const docsBase = normalizeBase(process.env.DOCS_BASE)
const localStorybookDevOrigin = process.env.VITE_STORYBOOK_DEV_ORIGIN?.trim() ?? ''
const docsPort = process.env.DOCS_PORT?.trim() ?? '60081'

export default defineConfig({
  root: 'docs',
  base: docsBase,
  builderConfig: {
    source: {
      define: {
        'process.env.RSPRESS_STORYBOOK_DEV_ORIGIN': JSON.stringify(localStorybookDevOrigin),
        'process.env.RSPRESS_DOCS_PORT': JSON.stringify(docsPort),
      },
    },
  },
  title: 'Codex Vibe Monitor 文档',
  description: '面向自部署与项目开发的 Codex Vibe Monitor 文档站。',
  lang: 'zh',
  themeConfig: {
    search: true,
    nav: [
      { text: '首页', link: '/' },
      { text: '部署与开发', link: '/quick-start' },
      { text: '配置参考', link: '/config' },
      { text: '项目介绍', link: '/product' },
      { text: 'Storybook', link: '/storybook.html' },
      { text: 'GitHub', link: 'https://github.com/IvanLi-CN/codex-vibe-monitor' },
    ],
    sidebar: {
      '/': [
        {
          text: '自部署与上手',
          items: [
            { text: '项目首页', link: '/' },
            { text: '部署与开发', link: '/quick-start' },
            { text: '配置参考', link: '/config' },
            { text: '项目介绍', link: '/product' },
          ],
        },
        {
          text: '预览与源码',
          items: [
            { text: 'Storybook 入口', link: '/storybook.html' },
            { text: 'Storybook 导览', link: '/storybook-guide.html' },
            { text: 'GitHub 仓库', link: 'https://github.com/IvanLi-CN/codex-vibe-monitor' },
          ],
        },
      ],
    },
  },
})
