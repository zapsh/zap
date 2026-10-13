import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import { loadEnv, defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
// import vueDevTools from 'vite-plugin-vue-devtools'
import UnoCSS from 'unocss/vite'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import { ElementPlusResolver } from 'unplugin-vue-components/resolvers'
// import { visualizer } from "rollup-plugin-visualizer";
import Icons from 'unplugin-icons/vite'

// zapd - footer version
function workspaceVersion(): string {
  const fromEnv = process.env.ZAP_VERSION?.trim()
  if (fromEnv) return fromEnv
  try {
    const cargo = readFileSync(
      fileURLToPath(new URL('../Cargo.toml', import.meta.url)),
      'utf-8',
    )
    return cargo.match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1] ?? ''
  } catch {
    return ''
  }
}

const APP_VERSION = workspaceVersion()

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  console.log(mode)
  const env = loadEnv(mode, process.cwd())
  const useProxy = env.VITE_USE_PROXY === 'true'

  return {
    base: './',
    plugins: [
      vue(),
      // vueDevTools(),
      UnoCSS(),
      AutoImport({
        resolvers: [ElementPlusResolver()],
        imports:['vue','vue-router','pinia']
      }),
      Components({
        resolvers: [ElementPlusResolver()],
      }),
      // visualizer({
      //   gzipSize: true,
      //   brotliSize: true,
      //   emitFile: false,
      //   filename: "a.html", //分析图生成的文件名
      //   open:true //如果存在本地服务端口，将在打包后自动展示
      // }),
      // Material Symbols 图标：`~icons/material-symbols/*` 在构建期被编译成 Vue
      // 组件（统一出口见 src/icons/index.ts），只有用到的图标会进产物。
      Icons({
        // 关闭自动安装：内网/离线构建时 npm 拉取图标包会失败。
        // 需要的图标集需显式安装（当前只用 @iconify-json/material-symbols）。
        autoInstall: false,
        compiler: 'vue3',
      }),
    ],
    resolve: {
      alias: {
        '@': fileURLToPath(new URL('./src', import.meta.url)),
      },
    },
    server: {
      host: "0.0.0.0",
      port: 5173,
      allowedHosts: [
        '.cn',
        '127.0.0.1',
        'localhost'
      ],
      proxy: useProxy
        ? {
            '/api': {
              target: 'https://127.0.0.1:2663',
              changeOrigin: true,
              secure: false,
              // rewrite: (path) => path.replace(/^\/api/, ''),
            },
          }
        : undefined,
    },
    define: {
      // Zap 版本：workspace 统一版本（根 Cargo.toml [workspace.package]），
      // 由 build.sh 通过 ZAP_VERSION 传入（单独构建时自行解析 Cargo.toml）
      "import.meta.env.VITE_APP_VERSION": JSON.stringify(APP_VERSION),
      // Web 版本：前端包自身版本（web/package.json），与 Zap 版本相互独立
      "import.meta.env.VITE_WEB_VERSION": JSON.stringify(process.env.npm_package_version),
    },
   
  }
})
