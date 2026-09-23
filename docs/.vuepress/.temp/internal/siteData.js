export const siteData = JSON.parse("{\"base\":\"/\",\"lang\":\"zh-CN\",\"title\":\"ZAP\",\"description\":\"现代化 Linux 服务器 / VPS 控制面板 —— 网站、SSL、数据库、Docker、文件与运维\",\"head\":[],\"locales\":{\"/\":{\"lang\":\"zh-CN\",\"title\":\"ZAP\",\"description\":\"现代化 Linux 服务器 / VPS 控制面板 —— 网站、SSL、数据库、Docker、文件与运维\"}}}")

if (import.meta.webpackHot) {
  import.meta.webpackHot.accept()
  __VUE_HMR_RUNTIME__.updateSiteData?.(siteData)
}

if (import.meta.hot) {
  import.meta.hot.accept((m) => {
    __VUE_HMR_RUNTIME__.updateSiteData?.(m.siteData)
  })
}
