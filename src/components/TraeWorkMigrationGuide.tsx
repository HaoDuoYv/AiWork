import {
  AlertTriangle,
  Archive,
  CheckCircle2,
  FolderInput,
  KeyRound,
  Search,
  ShieldCheck,
} from 'lucide-react';
import { Modal } from './ui';
import { useAppStore } from '../store';

/**
 * Trae 双应用合并升级引导（升级后首启一次性）：
 * 展示 TraeWork 合并迁移的流程与结果（settings 键退役/承接、旧快照池搬迁、
 * 冲突组保留），用户关闭后后端落「已读」标记，之后启动不再展示。
 * 报告由后端迁移完成时写入 kv（traework_merge_report）；无报告（全新安装/
 * 迁移未跑）时本组件不渲染。
 */
export default function TraeWorkMigrationGuide() {
  const open = useAppStore((s) => s.traeworkGuideOpen);
  const report = useAppStore((s) => s.traeworkGuide);
  const dismiss = useAppStore((s) => s.dismissTraeworkGuide);
  if (!report) return null;

  // settings 步明细：优先消费结构化 settings_note（后端新版报告直传）；
  // 旧版报告（升级前已落 kv）无该字段时回退子串匹配，不误显「已跳过」
  const settingsNote =
    report.settings_note ?? report.notes.find((n) => n.includes('trae_path'));
  const finishedAt = new Date(report.finished_at_ms).toLocaleString();

  const steps: { icon: typeof Search; title: string; detail: string }[] = [
    {
      icon: Search,
      title: '检测旧版 Trae Work 存量数据',
      detail: '扫描旧安装路径设置与旧快照池（data/profiles）',
    },
    {
      icon: KeyRound,
      title: '迁移安装路径设置',
      detail: settingsNote ?? '未发现旧安装路径设置（trae_path），已跳过',
    },
    {
      icon: FolderInput,
      title: '搬迁旧版快照池',
      detail:
        report.moved_groups > 0
          ? `${report.moved_groups} 组快照整组迁入新池 profiles_trae（仅移动不复制，绝不覆盖）`
          : '未发现需要搬迁的快照',
    },
    {
      icon: Archive,
      title: '同名冲突槽位处理',
      detail:
        report.legacy_groups > 0
          ? `${report.legacy_groups} 组与新池同 uid 冲突，整组保留至 profiles_trae_work_legacy（原样保留、可随时取回；确认新快照可用后可手动清理该目录）`
          : '无同名冲突',
    },
  ];

  return (
    <Modal
      open={open}
      onClose={() => void dismiss()}
      title="升级引导：Trae 双应用已合并"
      size="lg"
      footer={
        <button className="btn-primary" onClick={() => void dismiss()}>
          知道了
        </button>
      }
    >
      <div className="space-y-4 text-sm">
        <section className="rounded-lg border border-sky-200 bg-sky-50 p-3 dark:border-sky-700 dark:bg-sky-900/20">
          <h3 className="mb-1 flex items-center gap-1.5 font-semibold text-sky-700 dark:text-sky-300">
            <ShieldCheck size={15} /> 官方客户端已合并
          </h3>
          <p className="text-xs leading-relaxed text-sky-800 dark:text-sky-200">
            Trae Work（TRAE SOLO CN）与 Trae CN IDE 已合并为单一客户端（Trae CN）。
            本工具已同步收敛为单 Trae 应用，并在本次启动自动完成存量数据迁移——
            全程仅移动、绝不覆盖，登录态与快照数据零丢失。
          </p>
        </section>

        <section className="rounded-lg border border-amber-200 bg-amber-50 p-3 dark:border-amber-700 dark:bg-amber-900/20">
          <h3 className="mb-1 flex items-center gap-1.5 font-semibold text-amber-700 dark:text-amber-300">
            <AlertTriangle size={15} /> 建议：所有账号重新登录并保存会话
          </h3>
          <p className="text-xs leading-relaxed text-amber-800 dark:text-amber-200">
            两客户端合并后，<b>旧版保存的登录快照</b>来自旧客户端的设备指纹，
            恢复进合并版后可能触发重新登录或设备重新认证。建议把你有 Trae 账号的每个账号在客户端中依次
            <b>手动登录</b>一遍，登录成功后回到本工具「账号管理」页点击该账号行的
            <b>「保存当前登录态」</b>重建快照——此后的「切换」才会稳定复现登录态。
          </p>
          <p className="mt-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            ⚠ 请勿依赖旧快照直接「切换」；若切换后提示实际登录为其他账号或未登录，
            按提示在客户端手动登录该账号后重新「保存当前登录态」即可修复。
          </p>
        </section>

        <section>
          <h3 className="mb-2 font-semibold">迁移流程与结果</h3>
          <ol className="space-y-2.5">
            {steps.map(({ icon: Icon, title, detail }) => (
              <li key={title} className="flex items-start gap-2.5">
                <CheckCircle2 size={16} className="mt-0.5 shrink-0 text-emerald-500" />
                <div className="min-w-0">
                  <p className="flex items-center gap-1.5 text-xs font-semibold">
                    <Icon size={13} className="text-slate-400 dark:text-zinc-500" />
                    {title}
                  </p>
                  <p className="mt-0.5 text-xs leading-relaxed text-slate-600 dark:text-zinc-300">
                    {detail}
                  </p>
                </div>
              </li>
            ))}
          </ol>
        </section>

        {report.notes.length > 0 && (
          <section className="rounded-lg border border-slate-200 p-3 dark:border-zinc-700">
            <h3 className="mb-1.5 font-semibold">迁移明细</h3>
            <ul className="ml-4 list-disc space-y-1 text-xs leading-relaxed text-slate-600 dark:text-zinc-300">
              {report.notes.map((n, i) => (
                <li key={i}>{n}</li>
              ))}
            </ul>
          </section>
        )}

        <p className="text-xs text-slate-400 dark:text-zinc-500">
          迁移完成时间：{finishedAt}。此引导仅展示一次，关闭后可在
          <code className="mx-1 rounded bg-slate-100 px-1 dark:bg-zinc-800">
            data/profiles_trae_work_legacy
          </code>
          目录查看保留的旧版冲突快照。
        </p>
      </div>
    </Modal>
  );
}
