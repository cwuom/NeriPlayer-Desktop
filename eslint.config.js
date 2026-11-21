import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import globals from 'globals'
import tseslint from 'typescript-eslint'

// 规则分两档：error 为必须立即修复的正确性问题；warn 为已存量的技术债，
// 由 package.json 中 lint 脚本的 --max-warnings 作为基线只减不增
export default tseslint.config(
  {
    ignores: ['dist/**', 'coverage/**', 'node_modules/**', 'src-tauri/**', 'vendor/**', 'docs/**', 'tools/**', '**/*.d.ts'],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/essential'],
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: { parser: tseslint.parser },
    },
  },
  {
    files: ['src/**/*.{ts,vue}'],
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      '@typescript-eslint/no-explicit-any': 'warn',
      '@typescript-eslint/no-unused-vars': ['error', {
        argsIgnorePattern: '^_',
        varsIgnorePattern: '^_',
        caughtErrorsIgnorePattern: '^_',
      }],
      'no-console': 'error',
      // typescript-eslint 官方建议：TS 类型检查已覆盖未定义标识符，no-undef 会误报 DOM 类型名
      'no-undef': 'off',
      eqeqeq: ['error', 'always', { null: 'ignore' }],
    },
  },
  {
    // 统一日志工具是唯一允许直接写 console 的出口
    files: ['src/utils/logger.ts'],
    rules: { 'no-console': 'off' },
  },
  {
    // 测试会用 node:fs 读取源码做契约断言，需要 Node 全局
    files: ['src/**/__tests__/**/*.ts'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  {
    // Issue 分流等自动化脚本运行在 GitHub Actions 的 Node 环境，使用 CommonJS
    files: ['.github/scripts/**/*.cjs'],
    languageOptions: {
      sourceType: 'commonjs',
      globals: { ...globals.node },
    },
    rules: {
      '@typescript-eslint/no-require-imports': 'off',
    },
  },
  {
    // 首屏前同步执行的主题初始化脚本，运行在浏览器全局作用域
    files: ['public/**/*.js'],
    languageOptions: {
      sourceType: 'script',
      globals: { ...globals.browser },
    },
  },
  {
    files: ['*.config.{js,ts}', 'scripts/**/*.mjs'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
)
