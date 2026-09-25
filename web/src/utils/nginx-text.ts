/**
 * 给管理员看的「nginx 配置示例 / 语法提示」文案。
 *
 * 这些字符串里带 nginx 的花括号（`stream { }`），**不能放进 i18n**：vue-i18n 会把
 * `{ }` 当插值解析，空花括号直接抛 `SyntaxError: 7`（"Empty placeholder"），
 * 渲染到该条文案时整个组件渲染中断、弹「系统错误」。
 * 所以单独放这里，按语言原样取用，不经过 vue-i18n 的编译。
 * （`$backend` 这类裸变量其实是安全的，放在这儿只是为了和同类文案集中管理。）
 *
 * 用法：`nginxText(locale.value, 'advancedTip')`
 */
const NGINX_TEXT: Record<string, Record<string, string>> = {
  'zh-CN': {
    advancedTip:
      '直接写 upstream / map / server 等指令，面板原样插入（不要再写 stream { }）；适合 SNI 分流、自定义负载等玩法',
    advancedPlaceholder:
      'upstream backend_api {\n    server 10.0.1.10:443;\n}\n\nserver {\n    listen 443;\n    ssl_preread on;\n    proxy_pass backend_api;\n}',
    extraTip: '插进 server { } 内部的额外指令，一行一条（不要写 { }）',
    sslPrereadTip:
      '开启后按 SNI 分流：需在「全局配置」里用 map $ssl_preread_server_name $backend 定义变量，并在下面填 proxy_pass 变量',
    proxyPassTip: '以 $ 开头的变量（如 $backend）；留空则转发到本规则的后端（组）',
    globalTip:
      '写在这儿的指令会出现在 stream { } 顶部，供所有规则引用：resolver / map / 公共 upstream / log_format 等',
    globalPlaceholder: 'resolver 10.0.0.2 valid=10s;\nresolver_timeout 10s;',
  },
  'en-US': {
    advancedTip:
      'Write upstream / map / server directives directly; the panel inserts them verbatim (no nested stream { }). Use it for SNI routing, custom balancing, etc.',
    advancedPlaceholder:
      'upstream backend_api {\n    server 10.0.1.10:443;\n}\n\nserver {\n    listen 443;\n    ssl_preread on;\n    proxy_pass backend_api;\n}',
    extraTip: 'Additional directives inside server { }, one per line (no { })',
    sslPrereadTip:
      'Route by SNI: define the variable with map $ssl_preread_server_name $backend under Global config, then set the proxy_pass variable below',
    proxyPassTip:
      'A variable starting with $ (e.g. $backend); empty forwards to this rule target group',
    globalTip:
      'Directives here land at the top of stream { } and can be referenced by every rule: resolver / map / shared upstream / log_format, etc.',
    globalPlaceholder: 'resolver 10.0.0.2 valid=10s;\nresolver_timeout 10s;',
  },
}

/** 取含 nginx 语法的文案（绕过 vue-i18n 的 message 解析），缺失时退回中文 */
export function nginxText(locale: string, key: string): string {
  return NGINX_TEXT[locale]?.[key] ?? NGINX_TEXT['zh-CN'][key] ?? ''
}
