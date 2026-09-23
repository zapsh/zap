#!/usr/bin/env node
/**
 * 从后端的机器可读 API 描述生成 VuePress API 参考页。
 *
 * 数据源：zapd/src/routers/api_docs.json（后端维护，单一事实来源 ——
 * 后端改了接口，重跑本脚本即可让文档跟上，不必手改 markdown）。
 *
 * 产物：
 *   api/README.md          总览（认证方式、统一响应、接口目录）
 *   api/<slug>.md          每个 API 分组一页
 *   .vuepress/api-sidebar.json   侧栏（config.ts 引用它）
 *
 * 用法：node scripts/gen-api.mjs（npm run gen:api）
 */
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))
const SRC = join(ROOT, '..', 'zapd', 'src', 'routers', 'api_docs.json')
const API_DIR = join(ROOT, 'api')
const SIDEBAR = join(ROOT, '.vuepress', 'api-sidebar.json')

const doc = JSON.parse(readFileSync(SRC, 'utf8'))

// VuePress 会把 Markdown 里的 `<token>`、`<host>` 之类当内联 HTML 解析，
// 直接让编译报「Element is missing end tag」。后端文案是纯文本，这里统一转义。
const esc = (s) => String(s ?? '').replace(/</g, '&lt;').replace(/>/g, '&gt;')

// 组名 → URL slug 与展示名。新分组忘了登记时按 unknown-N 兜底并提醒，
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
  '站点管理': 'site',
  '开发（API Token）': 'token',
  'SSL/TLS（证书管理）': 'ssl',
  'Zap 设置（面板自身配置）': 'zap-config',
}

const used = new Set()
const slugify = (name) => {
  if (SLUGS[name]) {
    used.add(name)
    return SLUGS[name]
  }
  console.warn(`⚠ 未登记的 API 分组「${name}」，请把它加进 gen-api.mjs 的 SLUGS`)
  let i = 0
  while (Object.values(SLUGS).includes(`unknown-${i}`) || used.has(`unknown-${i}`)) i++
  return `unknown-${i}`
}

/** endpoint → markdown 小节 */
const renderEndpoint = (e) => {
  const lines = []
  lines.push(`## ${e.method} \`${e.path}\``, '')
  lines.push(esc(e.summary), '')
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
    lines.push('| 参数 | 类型 | 必填 | 说明 |', '| --- | --- | :-: | --- |')
    for (const p of e.params) {
      lines.push(`| ${esc(p.name)} | ${esc(p.type)} | ${p.required ? '是' : '否'} | ${esc(p.desc)} |`)
    }
    lines.push('')
  }
  return lines.join('\n')
}

mkdirSync(API_DIR, { recursive: true })

const toc = []
for (const group of doc.groups) {
  const slug = slugify(group.name)
  const file = slug === 'health' && toc.length === 0 ? 'health' : slug
  const md = [
    '---',
    `title: ${group.name}`,
    '---',
    '',
    `# ${group.name}`,
    '',
    group.description ? `> ${esc(group.description)}` : '',
    '',
    ...group.endpoints.map(renderEndpoint),
  ]
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
  writeFileSync(join(API_DIR, `${file}.md`), md + '\n')
  toc.push({ text: group.name, link: `/api/${file}.html`, endpoints: group.endpoints.length })
}

// ── 总览页 ────────────────────────────────────────────────
const overview = [
  '---',
  'title: API 总览',
  '---',
  '',
  `# ${doc.title}`,
  '',
  `> 共 ${toc.reduce((s, g) => s + g.endpoints, 0)} 个接口，基址 \`${doc.base_path}\`。左侧目录按功能分组。`,
  '',
  '## 认证',
  '',
  ...doc.auth_intro.map((l) => `- ${esc(l)}`),
  '',
  '## 统一响应',
  '',
  '```json',
  '{ "code": 0, "message": "OK", "data": ... }',
  '```',
  '',
  '`code` 非 0 表示失败，`message` 是可读的错误说明。',
  '',
  '## 接口目录',
  '',
  '| 分组 | 接口数 |', '| --- | -: |',
  ...toc.map((g) => `| [${g.text}](${g.link}) | ${g.endpoints} |`),
].join('\n')
writeFileSync(join(API_DIR, 'README.md'), overview + '\n')

// .vuepress 目录可能还不存在（首次生成时）
mkdirSync(dirname(SIDEBAR), { recursive: true })
writeFileSync(SIDEBAR, JSON.stringify(toc.map(({ text, link }) => ({ text, link })), null, 2) + '\n')

console.log(`✓ 生成 ${toc.length} 个分组页 + 总览（api/），侧栏已更新`)
