import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { login, getUserInfo, logout as logoutApi } from '@/api/user'
import { useTagsStore } from '@/stores/tags'
import { setToken, removeToken, setTokenExpire } from '@/utils/auth'
import { ElMessage } from 'element-plus'
import { t } from '@/i18n'

/**
 * 默认头像：内联 SVG（data URI）。
 * 面板可能部署在纯内网，不能引用任何外网图片（原先用的是 cube.elemecdn.com，
 * 内网下会加载失败导致头像空白），这里直接内联一个灰色人像图标。
 */
const DEFAULT_AVATAR =
  'data:image/svg+xml,' +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">' +
      '<circle cx="512" cy="512" r="512" fill="#d9d9d9"/>' +
      '<circle cx="512" cy="380" r="150" fill="#ffffff"/>' +
      '<path d="M512 570c-143 0-260 90-282 207-9 47 21 87 70 87h424c49 0 79-40 70-87-22-117-139-207-282-207z" fill="#ffffff"/>' +
      '</svg>',
  )

export const useUserStore = defineStore(
  'user',
  () => {
    const token = ref('')
    const userId = ref<number>(0)
    const name = ref('')
    const avatar = ref('')
    const roles = ref<string[]>([])
    const permissions = ref<string[]>([])
    /** 当前账号类型：0=普通用户/客户 / 1=成员（子账号，共享父账号的家目录与系统账号） */
    const userKind = ref<number>(0)
    /** 只读账号：共享可见但不可改（后端只放行查看类权限点） */
    const readOnly = ref<boolean>(false)
    const email = ref('')
    const phone = ref('')
    const nickname = ref('')

    const userInfo = computed(() => ({
      id: userId.value,
      name: name.value || t('app.user'),
      username: name.value || t('app.user'),
      nickname: nickname.value || name.value || t('app.user'),
      avatar: avatar.value || DEFAULT_AVATAR,
      roles: roles.value,
      permissions: permissions.value,
      /** 账号类型：成员（子账号）不能再建成员，也不显示部分账号级功能 */
      user_kind: userKind.value,
      /** 只读账号：页面可据此隐藏写操作，后端也会直接拒绝 */
      read_only: readOnly.value,
      email: email.value || '',
      phone: phone.value || '',
      introduction: t('app.welcome'),
    }))

    // 登录
    async function loginAction(userInfo: { username: string; password: string }) {
      try {
        const res = await login(userInfo)
        if (res.access_token) {
          token.value = res.access_token
          setToken(res.access_token)
          setTokenExpire(res.expire_in)
          return Promise.resolve(res)
        }
        return Promise.reject(new Error(t('app.loginFailed')))
      } catch (error) {
        return Promise.reject(error)
      }
    }

    // 获取用户信息
    async function getInfoAction() {
      try {
        const res = await getUserInfo()
        if (res) {
          userId.value = res.data.id ?? 0
          name.value = res.data.username
          nickname.value = res.data.nickname
          avatar.value = res.data.avatar
          roles.value = res.data.roles
          permissions.value = res.data.permissions
          userKind.value = res.data.user_kind ?? 0
          readOnly.value = res.data.read_only ?? false
          email.value = res.data.email ?? ''
          phone.value = res.data.phone ?? ''
          return Promise.resolve(res)
        }
        return Promise.reject(new Error(t('app.fetchUserFailed')))
      } catch (error) {
        return Promise.reject(error)
      }
    }

    /**
     * 清理上一账号生成的动态路由与菜单。
     * 用动态 import 规避 user → router/permission 的循环依赖；
     * 否则「同会话换账号登录」时 guard 会因残留菜单直接放行，导致低权限账号看到高权限菜单。
     */
    async function clearDynamicRoutes() {
      try {
        const [{ resetRouter }, { usePermissionStore }] = await Promise.all([
          import('@/router'),
          import('@/stores/permission'),
        ])
        usePermissionStore().setRoutes([])
        // 标签栏也清空：否则换账号后会残留上一个账号的页面路径
        useTagsStore().reset()
        resetRouter()
      } catch {
        // 清理失败不影响登出本身
      }
    }

    // 退出登录
    async function logout() {
      try {
        await logoutApi()
        await clearDynamicRoutes()
        token.value = ''
        userId.value = 0
        name.value = ''
        nickname.value = ''
        avatar.value = ''
        email.value = ''
        roles.value = []
        permissions.value = []
        userKind.value = 0
        readOnly.value = false
        removeToken()
        ElMessage({ type: 'success', message: t('app.logoutSuccess') })
        return Promise.resolve()
      } catch (error) {
        return Promise.reject(error)
      }
    }

    // 重置 Token（不调用后端）
    async function resetToken() {
      await clearDynamicRoutes()
      token.value = ''
      userId.value = 0
      name.value = ''
      nickname.value = ''
      avatar.value = ''
      email.value = ''
      roles.value = []
      permissions.value = []
      userKind.value = 0
      readOnly.value = false
      removeToken()
      return Promise.resolve()
    }

    return {
      token,
      name,
      avatar,
      roles,
      permissions,
      readOnly,
      userInfo, // 导出计算属性
      login: loginAction,
      getInfoAction,
      logout,
      resetToken,
    }
  },
  { persist: true },
) // 使用 Pinia 的持久化插件
