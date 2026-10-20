import { useAppStore } from '../store';

/**
 * 切换/保存登录态进度面板（issue #44 遗留项）。
 *
 * 订阅全局 switch-progress / save-login-progress NDJSON 事件管线，
 * switchingTo / savingLogin 非空时渲染进度行——此前仅 DoubaoAccounts 有本地面板，
 * Trae（含「续期 JWT」流程）与 Buddy 页切换/保存全程只有 toast 反馈。
 * 供 Trae / Buddy / Doubao 三个账号页复用；无进行中操作时渲染 null。
 */
export default function SwitchProgressPanel() {
  const switchingTo = useAppStore((s) => s.switchingTo);
  const switchProgress = useAppStore((s) => s.switchProgress);
  const savingLogin = useAppStore((s) => s.savingLogin);
  const saveLoginProgress = useAppStore((s) => s.saveLoginProgress);

  if (!switchingTo && !savingLogin) return null;

  return (
    <div className="mt-5 rounded-lg border border-brand-300 bg-brand-50 p-3 dark:border-brand-700 dark:bg-brand-900/20">
      <div className="mb-1 text-xs font-medium text-brand-700 dark:text-brand-300">
        {switchingTo ? `正在切换至 ${switchingTo}…` : `正在保存 ${savingLogin} 的登录态…`}
      </div>
      <div className="max-h-40 space-y-0.5 overflow-auto font-mono text-xs text-brand-600 dark:text-brand-400">
        {(switchingTo ? switchProgress : saveLoginProgress).length === 0 ? (
          <div>等待中...</div>
        ) : (
          (switchingTo ? switchProgress : saveLoginProgress).map((line, i) => <div key={i}>{line}</div>)
        )}
      </div>
    </div>
  );
}
