import { viteBundler } from '@vuepress/bundler-vite'
import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'
// API 侧栏由 scripts/gen-api.mjs 从后端 api_docs.json 生成，不要手改
import apiSidebar from './api-sidebar.json'

export default defineUserConfig({
  bundler: viteBundler(),
  lang: 'zh-CN',
  title: 'ZAP',
  description: '现代化 Linux 服务器 / VPS 控制面板 —— 网站、SSL、数据库、Docker、文件与运维',
  theme: defaultTheme({
    navbar: [
      { text: '指南', link: '/guide/install.md' },
      { text: '用户手册', link: '/guide/user-manual.md' },
      { text: 'API 参考', link: '/api/' },
    ],
    sidebar: {
      '/guide/': [
        {
          text: '指南',
          children: [
            '/guide/install.md',
            '/guide/upgrade.md',
            '/guide/faq.md',
            '/guide/user-manual.md',
            '/guide/changelog.md',
          ],
        },
      ],
      '/api/': [{ text: 'API 总览', link: '/api/' }, ...apiSidebar],
    },
    repo: 'zapsh/zap',
    docsDir: 'docs',
    editLink: false,
    lastUpdated: true,
  }),
})
