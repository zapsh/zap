#!/usr/bin/env node
/**
 * 从后端的机器可读 API 描述生成 VitePress API 参考页（中英双语）。
 *
 * 数据源：zapd/src/routers/api_docs.json（后端维护，单一事实来源 ——
 * 后端改了接口，重跑本脚本即可让文档跟上，不必手改 markdown）。
 *
 * 产物：
 *   api/index.md             中文总览（认证方式、统一响应、接口目录）
 *   api/<slug>.md            每个 API 分组一页（中文）
 *   en/api/index.md          英文总览
 *   en/api/<slug>.md         每个 API 分组一页（英文）
 *   .vitepress/api-sidebar.json     中文侧栏（config.ts 引用）
 *   .vitepress/api-sidebar.en.json  英文侧栏
 *
 * 翻译策略（渐进式，不阻塞发布）：
 *   - 分组名：查 EN_GROUPS 表（键是 api_docs.json 里的中文分组名）
 *   - 摘要 / 说明：优先取 *_en 字段（`summary_en` / `desc_en` / `name_en` /
 *     `description_en` / `title_en` / `auth_intro_en`），没有就回落中文原文。
 *     后端想让哪段话变成英文，就给它加一个 *_en 字段重跑本脚本即可。
 *   - 页面框架（标题层级、表头、总览文案）始终是对应语言。
 *   只要有回落发生，英文页顶部会带一句提示，避免"看起来翻完了其实没有"。
 *
 * 用法：node scripts/gen-api.mjs（npm run gen:api）
 */
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))
const SRC = join(ROOT, '..', 'zapd', 'src', 'routers', 'api_docs.json')
const ZH_DIR = join(ROOT, 'api')
const EN_DIR = join(ROOT, 'en', 'api')
const CONF_DIR = join(ROOT, '.vitepress')
const SIDEBAR_ZH = join(CONF_DIR, 'api-sidebar.json')
const SIDEBAR_EN = join(CONF_DIR, 'api-sidebar.en.json')

const doc = JSON.parse(readFileSync(SRC, 'utf8'))

// VitePress 会把 Markdown 里的 `<token>`、`<host>` 之类当内联 HTML 解析，
// 直接让编译报「Element is missing end tag」。后端文案是纯文本，这里统一转义。
const esc = (s) => String(s ?? '').replace(/</g, '&lt;').replace(/>/g, '&gt;')

// 组名 → URL slug。新分组忘了登记时按 unknown-N 兜底并提醒，
// 这样「新增分组」永远表现为「文档里多出一页」而不是悄悄缺页。
const SLUGS = {
  '健康检查': 'health',
  '认证 Auth': 'auth',
  '用户管理': 'user',
  '角色管理': 'role',
  '菜单管理': 'menu',
  '服务器时间 / 网络': 'network',
  'IP 池管理': 'ip-pool',
  '系统服务与进程': 'process',
  'SSH 服务与密钥': 'ssh',
  'SSH 终端（连接管理）': 'ssh-terminal',
  '系统状态与审计': 'audit',
  '文件管理': 'file',
  '应用商店 AppStore': 'appstore',
  '插件 Plugin': 'plugin',
  '站点管理': 'site',
  '域名站点应用管理': 'site-app',
  '开发（API Token）': 'token',
  'SSL/TLS（证书管理）': 'ssl',
  'Zap 设置（面板自身配置）': 'zap-config',
}

// 中文分组名 → 英文展示名（与 SLUGS 同源，改一边记得改另一边）
const EN_GROUPS = {
  '健康检查': 'Health Check',
  '认证 Auth': 'Authentication',
  '用户管理': 'Users',
  '角色管理': 'Roles',
  '菜单管理': 'Menus',
  '服务器时间 / 网络': 'Server Time & Network',
  'IP 池管理': 'IP Pool',
  '系统服务与进程': 'Services & Processes',
  'SSH 服务与密钥': 'SSH Service & Keys',
  'SSH 终端（连接管理）': 'SSH Terminal (Connections)',
  '系统状态与审计': 'System Status & Audit',
  '文件管理': 'File Management',
  '应用商店 AppStore': 'App Store',
  '插件 Plugin': 'Plugins',
  '站点管理': 'Sites',
  '域名站点应用管理': 'Site Applications',
  '开发（API Token）': 'Development (API Tokens)',
  'SSL/TLS（证书管理）': 'SSL/TLS (Certificates)',
  'Zap 设置（面板自身配置）': 'ZAP Settings',
}

const used = new Set()
const slugify = (name) => {
  if (SLUGS[name]) {
    used.add(name)
    return SLUGS[name]
  }
  console.warn(`⚠ 未登记的 API 分组「${name}」，请把它加进 gen-api.mjs 的 SLUGS / EN_GROUPS`)
  let i = 0
  while (Object.values(SLUGS).includes(`unknown-${i}`) || used.has(`unknown-${i}`)) i++
  return `unknown-${i}`
}

// 只要有字段回落中文，就在英文页顶部标注（见文件头注释）
let fallbackHit = false
/** 取英文文案，缺省回落中文原文并记一笔 */
const en = (obj, key) => {
  const v = obj?.[`${key}_en`]
  if (v) return v
  if (obj?.[key]) fallbackHit = true
  return obj?.[key] ?? ''
}

const UI = {
  zh: {
    paramTable: ['| 参数 | 类型 | 必填 | 说明 |', '| --- | --- | :-: | --- |'],
    yes: '是',
    no: '否',
    overviewTitle: 'API 总览',
    auth: '认证',
    unified: '统一响应',
    unifiedNote: '`code` 非 0 表示失败，`message` 是可读的错误说明。',
    index: '接口目录',
    groupCol: '分组',
    countCol: '接口数',
    summary: (n, base) => `> 共 ${n} 个接口，基址 \`${base}\`。左侧目录按功能分组。`,
  },
  en: {
    paramTable: ['| Parameter | Type | Required | Description |', '| --- | --- | :-: | --- |'],
    yes: 'Yes',
    no: 'No',
    overviewTitle: 'API Overview',
    auth: 'Authentication',
    unified: 'Unified Response',
    unifiedNote:
      'Any `code` other than 0 means failure; `message` carries a human-readable error description.',
    index: 'Endpoint Index',
    groupCol: 'Group',
    countCol: 'Endpoints',
    summary: (n, base) => `> ${n} endpoints in total, base path \`${base}\`. The sidebar groups them by feature.`,
  },
}

/** endpoint → markdown 小节 */
const renderEndpoint = (e, t, lang) => {
  const lines = []
  const summary = lang === 'en' ? en(e, 'summary') : e.summary
  lines.push(`## ${e.method} \`${e.path}\``, '')
  lines.push(esc(summary), '')
  lines.push('```bash')
  const hasBody = e.method !== 'GET' && e.params?.length
  lines.push(`curl -X ${e.method} "https://<host>${doc.base_path}${e.path}" \\`)
  lines.push(`  -H "Authorization: Bearer <token>"${hasBody ? ' \\' : ''}`)
  if (hasBody) {
    const body = Object.fromEntries(e.params.map((p) => [p.name, p.example ?? `<${p.type}>`]))
    lines.push(`  -H "Content-Type: application/json" \\`)
    lines.push(`  -d '${JSON.stringify(body, null, 2).replace(/\n/g, '\n   ')}'`)
  }
  lines.push('```', '')
  if (e.params?.length) {
    lines.push(...t.paramTable)
    for (const p of e.params) {
      const desc = lang === 'en' ? en(p, 'desc') : p.desc
      lines.push(`| ${esc(p.name)} | ${esc(p.type)} | ${p.required ? t.yes : t.no} | ${esc(desc)} |`)
    }
    lines.push('')
  }
  return lines.join('\n')
}

/** 生成一套语言的分组页 + 总览；返回侧栏用的目录 */
function build(lang, dir, linkPrefix) {
  const t = UI[lang]
  mkdirSync(dir, { recursive: true })
  const toc = []

  for (const group of doc.groups) {
    const slug = slugify(group.name)
    const file = slug === 'health' && toc.length === 0 ? 'health' : slug
    const name = lang === 'en' ? EN_GROUPS[group.name] ?? en(group, 'name') : group.name
    const desc = lang === 'en' ? en(group, 'description') : group.description
    const md = [
      '---',
      `title: ${name}`,
      '---',
      '',
      `# ${name}`,
      '',
      desc ? `> ${esc(desc)}` : '',
      '',
      ...group.endpoints.map((e) => renderEndpoint(e, t, lang)),
    ]
      .join('\n')
      .replace(/\n{3,}/g, '\n\n')
    writeFileSync(join(dir, `${file}.md`), md + '\n')
    toc.push({ text: name, link: `${linkPrefix}${file}`, endpoints: group.endpoints.length })
  }

  // ── 总览页 ──────────────────────────────────────────────
  const title = lang === 'en' ? doc.title_en ?? doc.title : doc.title
  const authIntro = lang === 'en' ? doc.auth_intro_en ?? doc.auth_intro : doc.auth_intro
  const notice =
    lang === 'en' && fallbackHit
      ? [
          '::: warning Translation in progress',
          'Some endpoint descriptions have no English text yet and fall back to the Chinese source.',
          'Add `summary_en` / `desc_en` fields in `zapd/src/routers/api_docs.json` and re-run `npm run gen:api`.',
          ':::',
          '',
        ]
      : []

  const overview = [
    '---',
    `title: ${t.overviewTitle}`,
    '---',
    '',
    `# ${esc(title)}`,
    '',
    ...notice,
    t.summary(toc.reduce((s, g) => s + g.endpoints, 0), doc.base_path),
    '',
    `## ${t.auth}`,
    '',
    ...(authIntro ?? []).map((l) => `- ${esc(l)}`),
    '',
    `## ${t.unified}`,
    '',
    '```json',
    '{ "code": 0, "message": "OK", "data": ... }',
    '```',
    '',
    t.unifiedNote,
    '',
    `## ${t.index}`,
    '',
    `| ${t.groupCol} | ${t.countCol} |`, `| --- | -: |`,
    ...toc.map((g) => `| [${g.text}](${g.link}) | ${g.endpoints} |`),
  ].join('\n')
  writeFileSync(join(dir, 'index.md'), overview + '\n')

  return toc
}

const tocZh = build('zh', ZH_DIR, '/api/')
fallbackHit = false
const tocEn = build('en', EN_DIR, '/en/api/')

// .vitepress 目录可能还不存在（首次生成时）
mkdirSync(CONF_DIR, { recursive: true })
const sidebar = (toc) =>
  JSON.stringify(toc.map(({ text, link }) => ({ text, link })), null, 2) + '\n'
writeFileSync(SIDEBAR_ZH, sidebar(tocZh))
writeFileSync(SIDEBAR_EN, sidebar(tocEn))

console.log(
  `✓ 生成 ${tocZh.length} 个分组页 + 总览：api/（中文）与 en/api/（英文），侧栏已更新`,
)
