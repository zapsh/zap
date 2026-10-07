// SPDX-License-Identifier: AGPL-3.0-only
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import 'element-plus/dist/index.css'
// Element Plus 官方深色主题变量（配合 <html class="dark"> 生效，见 composables/useTheme.ts）
import 'element-plus/theme-chalk/dark/css-vars.css'
import piniaPluginPersistedstate from 'pinia-plugin-persistedstate'
import App from './App.vue'
import router from './router'
import 'virtual:uno.css'
// 导入全局样式
import './assets/styles/index.css'
// 使用Element Plus的消息提示
import { ElMessage } from 'element-plus'
// 国际化：语言包 / 切换逻辑见 @/i18n，这里只负责挂载
import { setupI18n, t } from './i18n'

// 图标统一由 @/icons 提供（Material Symbols，构建期按需打包，离线可用），
// 不注册任何在线图标集合，内网部署下也不会发起图标请求。

const app = createApp(App)

// 使用插件
const pinia = createPinia()
pinia.use(piniaPluginPersistedstate)
app.use(pinia)
app.use(router)
setupI18n(app)

// 全局错误处理
app.config.errorHandler = (err, instance, info) => {
  console.error('[全局错误]', err)
  console.error('[错误组件]', instance)
  console.error('[错误信息]', info)

  // 错误分类处理（这里在组件上下文之外，用 @/i18n 的全局 t；语言切换后新报错即为新语言）
  if (err instanceof Error) {
    if (err.message.includes('Network Error')) {
      ElMessage.error(t('error.networkRetry'))
    } else if (err.message.includes('timeout')) {
      ElMessage.error(t('error.timeout'))
    } else if (info.includes('component')) {
      ElMessage.error(t('error.renderFailed'))
    } else {
      ElMessage.error(t('error.system'))
    }
  }

  // 实际项目中可以在这里添加错误上报逻辑
  // 例如：sendErrorToServer(err, instance, info)
}

app.mount('#app')
