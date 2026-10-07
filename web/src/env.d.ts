// SPDX-License-Identifier: AGPL-3.0-only
/// <reference types="vite/client" />
// Element Plus 语言包走 .mjs 入口（见 composables/useLocale.ts），官方未提供该路径的类型
declare module 'element-plus/dist/locale/*.mjs'

interface ImportMetaEnv {
  /** API 基础路径（开发环境用，生产由后端注入 window.__ZAP_BASE__ 决定） */
  readonly VITE_API_URL?: string
  /** 应用基础路径；留空表示部署在根路径，对应 zap.yaml 的 server.url_prefix */
  readonly VITE_BASE_URL?: string
  readonly VITE_USE_PROXY?: string
  /** zap 版本（workspace 统一版本：根 Cargo.toml [workspace.package]；
   *  发布时由 build.sh 解析后通过 ZAP_VERSION 传入，单独构建时由 vite 自行解析） */
  readonly VITE_APP_VERSION?: string
  /** Web 版本：前端包自身版本（web/package.json），与 Zap 版本相互独立 */
  readonly VITE_WEB_VERSION?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}

/** 后端在 index.html 注入的 URL 前缀（zap.yaml 的 server.url_prefix） */
interface Window {
  __ZAP_BASE__?: string
}
