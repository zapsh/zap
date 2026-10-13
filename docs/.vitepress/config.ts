import { defineConfig } from 'vitepress'
// API 侧栏由 scripts/gen-api.mjs 从后端 api_docs.json 生成，不要手改
import apiSidebarZh from './api-sidebar.json'
import apiSidebarEn from './api-sidebar.en.json'

/**
 * VitePress 配置（中英双语）。
 *
 * 目录约定：**root = 简体中文，`en/` = English**。VitePress 的 locale key 就是
 * 目录名，`root` 特指站点根，所以英文页放在 `docs/en/` 下、locale key 写作 `en`。
 * 每套语言各自持有 nav / sidebar 等主题配置（写在 locales.*.themeConfig 里），
 * 公共项（搜索、仓库、最后更新时间）放在外层 themeConfig 作为默认值。
 */
export default defineConfig({
  title: 'ZAP',
  cleanUrls: true,
  // api/ 与 en/api/ 由 gen-api.mjs 生成，首次检出还没跑生成脚本时不该让构建失败
  ignoreDeadLinks: true,

  locales: {
    root: {
      label: '简体中文',
      lang: 'zh-CN',
      description: '现代化 Linux 服务器 / VPS 控制面板 —— 网站、SSL、数据库、Docker、文件与运维',
      themeConfig: {
        nav: [
          { text: '指南', link: '/guide/install' },
          { text: '用户手册', link: '/guide/user-manual' },
          { text: 'API 参考', link: '/api/' },
        ],
        sidebar: {
          '/guide/': [
            {
              text: '指南',
              items: [
                { text: '安装', link: '/guide/install' },
                { text: '升级', link: '/guide/upgrade' },
                { text: '常见问题', link: '/guide/faq' },
                { text: '用户手册', link: '/guide/user-manual' },
                { text: '插件开发', link: '/guide/plugin-dev' },
                { text: '本地开发', link: '/guide/development' },
                { text: '更新日志', link: '/guide/changelog' },
              ],
            },
          ],
          '/design/': [
            {
              text: '设计',
              items: [
                { text: '项目结构', link: '/design/project-structure' },
                { text: '四层转发 / PHP / 容器', link: '/design/stream-php-docker' },
              ],
            },
          ],
          '/api/': [{ text: 'API 总览', link: '/api/' }, ...apiSidebarZh],
        },
        outlineTitle: '本页目录',
        langMenuLabel: '切换语言',
        lastUpdatedText: '最后更新于',
        docFooter: { prev: '上一篇', next: '下一篇' },
        returnToTopLabel: '回到顶部',
        sidebarMenuLabel: '目录',
        darkModeSwitchLabel: '深色模式',
      },
    },

    en: {
      label: 'English',
      lang: 'en-US',
      description:
        'Modern Linux server / VPS control panel — sites, SSL, databases, Docker, files and ops',
      themeConfig: {
        nav: [
          { text: 'Guide', link: '/en/guide/install' },
          { text: 'User Manual', link: '/en/guide/user-manual' },
          { text: 'API Reference', link: '/en/api/' },
        ],
        sidebar: {
          '/en/guide/': [
            {
              text: 'Guide',
              items: [
                { text: 'Installation', link: '/en/guide/install' },
                { text: 'Upgrade', link: '/en/guide/upgrade' },
                { text: 'FAQ', link: '/en/guide/faq' },
                { text: 'User Manual', link: '/en/guide/user-manual' },
                { text: 'Plugin Development', link: '/en/guide/plugin-dev' },
                { text: 'Local Development', link: '/en/guide/development' },
                { text: 'Changelog', link: '/en/guide/changelog' },
              ],
            },
          ],
          '/en/design/': [
            {
              text: 'Design',
              items: [
                { text: 'Project Structure', link: '/en/design/project-structure' },
                { text: 'Stream / PHP / Containers', link: '/en/design/stream-php-docker' },
              ],
            },
          ],
          '/en/api/': [{ text: 'API Overview', link: '/en/api/' }, ...apiSidebarEn],
        },
        outlineTitle: 'On this page',
        langMenuLabel: 'Change language',
        lastUpdatedText: 'Last updated on',
        docFooter: { prev: 'Previous', next: 'Next' },
        returnToTopLabel: 'Back to top',
        sidebarMenuLabel: 'Menu',
        darkModeSwitchLabel: 'Dark mode',
      },
    },
  },

  themeConfig: {
    search: {
      provider: 'local',
      options: {
        locales: {
          root: {
            translations: {
              button: { buttonText: '搜索', buttonAriaLabel: '搜索文档' },
              modal: {
                noResultsText: '没有找到结果',
                resetButtonTitle: '清除查询条件',
                footer: { selectText: '选择', navigateText: '切换' },
              },
            },
          },
          en: {
            translations: {
              button: { buttonText: 'Search', buttonAriaLabel: 'Search docs' },
              modal: {
                noResultsText: 'No results found',
                resetButtonTitle: 'Clear query',
                footer: { selectText: 'Select', navigateText: 'Navigate' },
              },
            },
          },
        },
      },
    },
    editLink: false,
    lastUpdated: true,
  },
})
