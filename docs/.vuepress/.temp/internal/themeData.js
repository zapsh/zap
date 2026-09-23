export const themeData = JSON.parse("{\"navbar\":[{\"text\":\"指南\",\"link\":\"/guide/install.md\"},{\"text\":\"用户手册\",\"link\":\"/guide/user-manual.md\"},{\"text\":\"API 参考\",\"link\":\"/api/\"}],\"sidebar\":{\"/guide/\":[{\"text\":\"指南\",\"children\":[\"/guide/install.md\",\"/guide/upgrade.md\",\"/guide/faq.md\",\"/guide/user-manual.md\",\"/guide/changelog.md\"]}],\"/api/\":[{\"text\":\"API 总览\",\"link\":\"/api/\"},{\"text\":\"健康检查\",\"link\":\"/api/health.html\"},{\"text\":\"认证 Auth\",\"link\":\"/api/auth.html\"},{\"text\":\"用户管理\",\"link\":\"/api/user.html\"},{\"text\":\"角色管理\",\"link\":\"/api/role.html\"},{\"text\":\"菜单管理\",\"link\":\"/api/menu.html\"},{\"text\":\"服务器时间 / 网络\",\"link\":\"/api/network.html\"},{\"text\":\"IP 池管理\",\"link\":\"/api/ip-pool.html\"},{\"text\":\"系统服务与进程\",\"link\":\"/api/process.html\"},{\"text\":\"SSH 服务与密钥\",\"link\":\"/api/ssh.html\"},{\"text\":\"SSH 终端（连接管理）\",\"link\":\"/api/ssh-terminal.html\"},{\"text\":\"系统状态与审计\",\"link\":\"/api/audit.html\"},{\"text\":\"文件管理\",\"link\":\"/api/file.html\"},{\"text\":\"应用商店 AppStore\",\"link\":\"/api/appstore.html\"},{\"text\":\"站点管理\",\"link\":\"/api/site.html\"},{\"text\":\"开发（API Token）\",\"link\":\"/api/token.html\"},{\"text\":\"SSL/TLS（证书管理）\",\"link\":\"/api/ssl.html\"},{\"text\":\"Zap 设置（面板自身配置）\",\"link\":\"/api/zap-config.html\"}]},\"repo\":\"zapsh/zap\",\"docsDir\":\"docs\",\"editLink\":false,\"lastUpdated\":true,\"locales\":{\"/\":{\"selectLanguageName\":\"English\"}},\"colorMode\":\"auto\",\"colorModeSwitch\":true,\"logo\":null,\"selectLanguageText\":\"Languages\",\"selectLanguageAriaLabel\":\"Select language\",\"sidebarDepth\":2,\"editLinkText\":\"Edit this page\",\"contributors\":true,\"contributorsText\":\"Contributors\",\"notFound\":[\"There's nothing here.\",\"How did we get here?\",\"That's a Four-Oh-Four.\",\"Looks like we've got some broken links.\"],\"backToHome\":\"Take me home\",\"openInNewWindow\":\"open in new window\",\"toggleColorMode\":\"toggle color mode\",\"toggleSidebar\":\"toggle sidebar\"}")

if (import.meta.webpackHot) {
  import.meta.webpackHot.accept()
  if (__VUE_HMR_RUNTIME__.updateThemeData) {
    __VUE_HMR_RUNTIME__.updateThemeData(themeData)
  }
}

if (import.meta.hot) {
  import.meta.hot.accept(({ themeData }) => {
    __VUE_HMR_RUNTIME__.updateThemeData(themeData)
  })
}
