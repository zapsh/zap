export const redirects = JSON.parse("{}")

export const routes = Object.fromEntries([
  ["/", { loader: () => import(/* webpackChunkName: "index.html" */"/home/zap/code/zap/docs/README.md"), meta: {"title":""} }],
  ["/guide/changelog.html", { loader: () => import(/* webpackChunkName: "guide_changelog.html" */"/home/zap/code/zap/docs/guide/changelog.md"), meta: {"title":"Zap 更新日志（中文版）"} }],
  ["/guide/faq.html", { loader: () => import(/* webpackChunkName: "guide_faq.html" */"/home/zap/code/zap/docs/guide/faq.md"), meta: {"title":"常见问题（FAQ）"} }],
  ["/guide/install.html", { loader: () => import(/* webpackChunkName: "guide_install.html" */"/home/zap/code/zap/docs/guide/install.md"), meta: {"title":"安装"} }],
  ["/guide/upgrade.html", { loader: () => import(/* webpackChunkName: "guide_upgrade.html" */"/home/zap/code/zap/docs/guide/upgrade.md"), meta: {"title":"升级指南"} }],
  ["/guide/user-manual.html", { loader: () => import(/* webpackChunkName: "guide_user-manual.html" */"/home/zap/code/zap/docs/guide/user-manual.md"), meta: {"title":"用户手册"} }],
  ["/api/", { loader: () => import(/* webpackChunkName: "api_index.html" */"/home/zap/code/zap/docs/api/README.md"), meta: {"title":"API 总览"} }],
  ["/api/appstore.html", { loader: () => import(/* webpackChunkName: "api_appstore.html" */"/home/zap/code/zap/docs/api/appstore.md"), meta: {"title":"应用商店 AppStore"} }],
  ["/api/audit.html", { loader: () => import(/* webpackChunkName: "api_audit.html" */"/home/zap/code/zap/docs/api/audit.md"), meta: {"title":"系统状态与审计"} }],
  ["/api/auth.html", { loader: () => import(/* webpackChunkName: "api_auth.html" */"/home/zap/code/zap/docs/api/auth.md"), meta: {"title":"认证 Auth"} }],
  ["/api/file.html", { loader: () => import(/* webpackChunkName: "api_file.html" */"/home/zap/code/zap/docs/api/file.md"), meta: {"title":"文件管理"} }],
  ["/api/health.html", { loader: () => import(/* webpackChunkName: "api_health.html" */"/home/zap/code/zap/docs/api/health.md"), meta: {"title":"健康检查"} }],
  ["/api/ip-pool.html", { loader: () => import(/* webpackChunkName: "api_ip-pool.html" */"/home/zap/code/zap/docs/api/ip-pool.md"), meta: {"title":"IP 池管理"} }],
  ["/api/menu.html", { loader: () => import(/* webpackChunkName: "api_menu.html" */"/home/zap/code/zap/docs/api/menu.md"), meta: {"title":"菜单管理"} }],
  ["/api/network.html", { loader: () => import(/* webpackChunkName: "api_network.html" */"/home/zap/code/zap/docs/api/network.md"), meta: {"title":"服务器时间 / 网络"} }],
  ["/api/process.html", { loader: () => import(/* webpackChunkName: "api_process.html" */"/home/zap/code/zap/docs/api/process.md"), meta: {"title":"系统服务与进程"} }],
  ["/api/role.html", { loader: () => import(/* webpackChunkName: "api_role.html" */"/home/zap/code/zap/docs/api/role.md"), meta: {"title":"角色管理"} }],
  ["/api/site.html", { loader: () => import(/* webpackChunkName: "api_site.html" */"/home/zap/code/zap/docs/api/site.md"), meta: {"title":"站点管理"} }],
  ["/api/ssh-terminal.html", { loader: () => import(/* webpackChunkName: "api_ssh-terminal.html" */"/home/zap/code/zap/docs/api/ssh-terminal.md"), meta: {"title":"SSH 终端（连接管理）"} }],
  ["/api/ssh.html", { loader: () => import(/* webpackChunkName: "api_ssh.html" */"/home/zap/code/zap/docs/api/ssh.md"), meta: {"title":"SSH 服务与密钥"} }],
  ["/api/ssl.html", { loader: () => import(/* webpackChunkName: "api_ssl.html" */"/home/zap/code/zap/docs/api/ssl.md"), meta: {"title":"SSL/TLS（证书管理）"} }],
  ["/api/token.html", { loader: () => import(/* webpackChunkName: "api_token.html" */"/home/zap/code/zap/docs/api/token.md"), meta: {"title":"开发（API Token）"} }],
  ["/api/user.html", { loader: () => import(/* webpackChunkName: "api_user.html" */"/home/zap/code/zap/docs/api/user.md"), meta: {"title":"用户管理"} }],
  ["/api/zap-config.html", { loader: () => import(/* webpackChunkName: "api_zap-config.html" */"/home/zap/code/zap/docs/api/zap-config.md"), meta: {"title":"Zap 设置（面板自身配置）"} }],
  ["/404.html", { loader: () => import(/* webpackChunkName: "404.html" */"/home/zap/code/zap/docs/.vuepress/.temp/pages/404.html.vue"), meta: {"title":""} }],
]);

if (import.meta.webpackHot) {
  import.meta.webpackHot.accept()
  __VUE_HMR_RUNTIME__.updateRoutes?.(routes)
  __VUE_HMR_RUNTIME__.updateRedirects?.(redirects)
}

if (import.meta.hot) {
  import.meta.hot.accept((m) => {
    __VUE_HMR_RUNTIME__.updateRoutes?.(m.routes)
    __VUE_HMR_RUNTIME__.updateRedirects?.(m.redirects)
  })
}
