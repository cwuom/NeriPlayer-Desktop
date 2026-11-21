import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config'

// 复用 Vite 的别名与 Vue 插件，测试与生产构建解析同一套模块
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      include: ['src/**/__tests__/**/*.test.ts'],
      environment: 'node',
      restoreMocks: true,
      testTimeout: 10_000,
      coverage: {
        provider: 'v8',
        include: ['src/modules/**/*.ts', 'src/utils/**/*.ts', 'src/stores/**/*.ts'],
        exclude: ['src/**/__tests__/**'],
        // json 供 tools/quality/crap_report.py 计算 CRAP
        reporter: ['text-summary', 'html', 'json-summary', 'json'],
        reportsDirectory: 'coverage',
        // 下限按当前实测值向下取整，只允许上调。include 内未被单测加载的文件按 0% 计；
        // 很多逻辑由 scripts/test-*.mjs 回归脚本覆盖，但这些脚本不计入 v8 覆盖率
        thresholds: {
          lines: 42,
          statements: 42,
          functions: 72,
          branches: 81,
        },
      },
    },
  }),
)
