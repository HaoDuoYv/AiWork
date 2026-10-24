// ESLint 扁平配置（P1 质量门禁）：只做「大概率是真问题」的检查，不引入格式类争吵。
// - js.configs.recommended：未定义变量、重复声明、无效正则等确定性错误
// - tseslint 非类型感知的 recommended：语法/语义类明显错误（类型检查由 tsc 负责）
// - react-hooks 的 rules-of-hooks：Hook 只能在组件/自定义 Hook 顶层调用（true bug）
//
// 刻意**不启用**（存量代码与其设计模式冲突，启用即 90+ 误报，会淹没真问题）：
// - react-hooks v7 的 React Compiler 规则（set-state-in-effect / purity /
//   immutability / refs …）：本项目刻意在 effect 内同步 setState、渲染期读
//   new Date()、event-callback 里把最新 props 写进 ref，均为有意设计；
// - no-explicit-any：与 Rust DTO 契约处有类型收窄断言的真实用法；
// - no-unused-vars：由 tsc（noUnusedLocals 视情况）与 review 负责。
// 新增代码请勿依赖上述放宽——能写规范就写规范。
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

export default tseslint.config(
  { ignores: ['dist/**', 'node_modules/**', 'src-tauri/**', 'release/**', 'scripts/**'] },
  {
    // 存量代码里大量 `eslint-disable-next-line react-hooks/exhaustive-deps` 是历史遗留：
    // 该规则当前未启用，这些注释无害也不阻断；只在真启用对应规则后再清理。
    linterOptions: { reportUnusedDisableDirectives: 'off' },
  },
  js.configs.recommended,
  ...tseslint.configs.recommended.map((c) => ({
    ...c,
    rules: {
      ...c.rules,
      '@typescript-eslint/no-unused-vars': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
    },
  })),
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'no-empty': ['error', { allowEmptyCatch: true }],
    },
  },
);
