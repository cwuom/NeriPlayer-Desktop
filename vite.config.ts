import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

const amllCoreSrc = resolve(__dirname, 'vendor/applemusic-like-lyrics/packages/core/src')

// 预构建依赖重建后 ?v= 版本号可能不变而 chunk 名变了，WebView2 仍按 immutable 复用旧文件，
// 新旧 chunk 混用会加载两份 Vue，路由页面整片空白；开发时改为每次协商缓存
function revalidateOptimizedDeps(): Plugin {
  return {
    name: 'neri-revalidate-optimized-deps',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url?.startsWith('/node_modules/.vite/')) {
          const setHeader = res.setHeader.bind(res)
          res.setHeader = ((name: string, value: number | string | readonly string[]) =>
            setHeader(name, name.toLowerCase() === 'cache-control' ? 'no-cache' : value)) as typeof res.setHeader
        }
        next()
      })
    },
  }
}

export default defineConfig({
  plugins: [vue(), revalidateOptimizedDeps()],
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
      '@amll-core': amllCoreSrc,
      '#interfaces': resolve(amllCoreSrc, 'interfaces.ts'),
      '#utils': resolve(amllCoreSrc, 'utils'),
      '#styles': resolve(amllCoreSrc, 'styles'),
      '#lyric': resolve(amllCoreSrc, 'lyric-player'),
      '#bg': resolve(amllCoreSrc, 'bg-player'),
    },
  },
  clearScreen: false,
  // 仅以项目自身入口做依赖扫描，避免爬到 vendor 下 AMLL playground 的
  // 多个 html 入口（它们会引用 react/jotai 等未安装的包，触发无关报错）
  optimizeDeps: {
    entries: ['index.html'],
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/')) return 'vendor'
          if (id.includes('/vendor/applemusic-like-lyrics/')) return 'lyrics-core'
          return undefined
        },
      },
    },
  },
  server: {
    // 1420 被其它程序占用时 scripts/run-tauri.mjs 会换一个空闲端口，并让 Tauri 打开同一地址
    port: Number(process.env.NERI_DEV_PORT) || 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
})
