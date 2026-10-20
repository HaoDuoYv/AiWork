/**
 * 智能体接入中心（三项目合并的**必做能力**）
 *
 * 原 TraeWorkAssistant 只有「把本网关注册进 CC Switch」的单向能力，
 * 缺「把本机其他智能体的配置一次性导进来」的反向能力。本页补齐：
 *
 * 1. **扫描**：内置 13 个智能体注册表（Claude Code / Codex CLI / Gemini CLI /
 *    Qwen Code / OpenCode / Continue / Cline / Roo Code / Cursor / Windsurf /
 *    Cherry Studio / Chatbox / Aider）+ 任意「额外目录」（自动识别
 *    qoder2api-hub / workbuddy2api-hub 这类兄弟网关的端口与模型目录）；
 * 2. **导入**：把上游（base_url + api_key + 模型名）并入「自定义模型」资源池，
 *    可选同时建专用 ck_ Key、把 MCP server 合并进统一注册表；
 * 3. **接入**：把本网关写回目标智能体配置（Claude Code / Codex / OpenCode /
 *    Continue / Aider 可自动落盘并备份，其余给出可复制片段）。
 *
 * 红线：后端只回 Key 掩码；前端任何地方不展示完整 Key。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Plug,
  RefreshCw,
  DownloadCloud,
  ArrowUpRight,
  Copy,
  ShieldCheck,
  CheckCircle2,
  Circle,
  AlertTriangle,
  FolderPlus,
  Server,
  History,
  Trash2,
} from 'lucide-react';
import PageHeader from '../../components/PageHeader';
import { Badge, EmptyState, Modal, Spinner, StatCard } from '../../components/ui';
import { useAppStore } from '../../store';
import { api } from '../../lib/tauri';
import { withMinDelay } from '../../lib/delay';
import { cn } from '../../lib/cn';
import type {
  AgentConnectReport,
  AgentGatewayInfo,
  AgentImportOptions,
  AgentImportPlan,
  AgentImportRecord,
  AgentImportReport,
  AgentMcpServer,
  AgentProbe,
  AgentScanResult,
} from '../../types';

/** 导入选项默认值（与 Rust `AgentImportOptions::default()` 对齐） */
const DEFAULT_OPTS: AgentImportOptions = {
  create_models: true,
  create_keys: false,
  enable_models: true,
  overwrite: false,
  register_mcp: true,
};

function fmtTime(ts: number): string {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export default function AgentHub() {
  const pushToast = useAppStore((s) => s.pushToast);
  const [scan, setScan] = useState<AgentScanResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState('');
  const [activeKey, setActiveKey] = useState<string>('');
  const [selected, setSelected] = useState<string[]>([]);
  const [extraRoots, setExtraRoots] = useState<string[]>([]);
  const [rootInput, setRootInput] = useState('');
  const [opts, setOpts] = useState<AgentImportOptions>(DEFAULT_OPTS);
  const [plan, setPlan] = useState<AgentImportPlan | null>(null);
  const [gateway, setGateway] = useState<AgentGatewayInfo | null>(null);
  const [history, setHistory] = useState<AgentImportRecord[]>([]);
  const [mcp, setMcp] = useState<AgentMcpServer[]>([]);
  const [report, setReport] = useState<AgentImportReport | null>(null);
  const [connectResult, setConnectResult] = useState<AgentConnectReport | null>(null);
  const [showOpts, setShowOpts] = useState(false);
  const autoSelected = useRef(false);

  const probes = useMemo<AgentProbe[]>(() => {
    const list = [...(scan?.probes ?? []), ...(scan?.extras ?? [])];
    // 已安装优先，其余按注册表顺序
    return list.sort((a, b) => Number(b.installed) - Number(a.installed));
  }, [scan]);

  const active = useMemo(
    () => probes.find((p) => p.key === activeKey) ?? probes[0] ?? null,
    [probes, activeKey],
  );

  const loadAux = useCallback(async () => {
    const [g, h, m] = await Promise.all([
      api.agents.gatewayInfo().catch(() => null),
      api.agents.history().catch(() => [] as AgentImportRecord[]),
      api.agents.mcpRegistry().catch(() => [] as AgentMcpServer[]),
    ]);
    setGateway(g);
    setHistory([...h].reverse());
    setMcp(m);
  }, []);

  const rescan = useCallback(
    async (roots?: string[]) => {
      setLoading(true);
      try {
        const r = await withMinDelay(api.agents.scan(roots ?? extraRoots), 260);
        setScan(r);
        if (!activeKey && r.probes.length > 0) {
          const firstInstalled = r.probes.find((p) => p.installed);
          setActiveKey(firstInstalled?.key ?? r.probes[0].key);
        }
        // 首轮自动勾选所有可导入上游，实现真正的「一键」
        if (!autoSelected.current) {
          autoSelected.current = true;
          const all = [...r.probes, ...r.extras]
            .flatMap((p) => p.providers)
            .filter((p) => p.importable)
            .map((p) => p.id);
          setSelected(all);
        }
      } catch (e) {
        pushToast('error', String(e));
      } finally {
        setLoading(false);
      }
    },
    [activeKey, extraRoots, pushToast],
  );

  useEffect(() => {
    void rescan([]);
    void loadAux();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 选择变化 → 刷新预览（本地只读调用，代价极低）
  useEffect(() => {
    if (selected.length === 0) {
      setPlan(null);
      return;
    }
    let alive = true;
    api.agents
      .preview(selected, opts, extraRoots)
      .then((p) => {
        if (alive) setPlan(p);
      })
      .catch(() => {
        if (alive) setPlan(null);
      });
    return () => {
      alive = false;
    };
  }, [selected, opts, extraRoots]);

  const installed = probes.filter((p) => p.installed);
  const providerCount = probes.reduce((n, p) => n + p.providers.filter((x) => x.importable).length, 0);
  const mcpCount = probes.reduce((n, p) => n + p.mcp_servers.length, 0);
  const conflictCount = plan?.models.filter((m) => m.conflict).length ?? 0;

  const toggle = (id: string) => {
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  };

  const runImport = async () => {
    if (selected.length === 0) {
      pushToast('error', '请先勾选要导入的上游');
      return;
    }
    setBusy('import');
    try {
      const r = await withMinDelay(api.agents.apply(selected, opts, extraRoots), 400);
      setReport(r);
      await loadAux();
      // 导入成功后重扫（可导入项会变成「已存在」）
      await rescan();
      pushToast(
        r.models_added + r.keys_added + r.mcp_added > 0 ? 'success' : 'error',
        `导入完成：模型 +${r.models_added}（跳过 ${r.models_skipped}）· Key +${r.keys_added} · MCP +${r.mcp_added}`,
      );
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const runConnect = async () => {
    if (!active) return;
    setBusy(`connect:${active.key}`);
    try {
      const r = await withMinDelay(api.agents.connect(active.key), 400);
      setConnectResult(r);
      pushToast(
        r.written.length > 0 ? 'success' : 'error',
        r.written.length > 0
          ? `${active.name} 已接入：写入 ${r.written.length} 个文件（已备份）`
          : `${active.name} 需手动粘贴片段`,
      );
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const addRoot = () => {
    const v = rootInput.trim();
    if (!v) return;
    if (extraRoots.includes(v)) {
      pushToast('error', '该目录已在列表中');
      return;
    }
    const next = [...extraRoots, v];
    setExtraRoots(next);
    setRootInput('');
    void rescan(next);
  };

  const removeRoot = (dir: string) => {
    const next = extraRoots.filter((r) => r !== dir);
    setExtraRoots(next);
    void rescan(next);
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast('success', '已复制到剪贴板');
    } catch {
      pushToast('error', '复制失败，请手动选择文本');
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title="智能体接入中心"
        desc="扫描本机其他 AI 智能体 / 兄弟网关的配置，一键并入资源池，或把本网关写回它们"
        actions={
          <>
            <button className="btn-ghost" onClick={() => void rescan()} disabled={loading || !!busy}>
              <RefreshCw size={15} className={cn(loading && 'animate-spin')} />
              重新扫描
            </button>
            <button className="btn-ghost" onClick={() => void loadAux()}>
              <History size={15} />
              刷新记录
            </button>
            <button
              className="btn-ghost"
              title="导出脱敏接入包（只含上游地址与 Key 掩码）"
              onClick={async () => {
                try {
                  const text = await api.agents.exportBundle();
                  await copy(text);
                } catch (e) {
                  pushToast('error', String(e));
                }
              }}
            >
              <Copy size={15} />
              导出接入包
            </button>
            <button className="btn-primary" onClick={() => void runImport()} disabled={!!busy || selected.length === 0}>
              {busy === 'import' ? <Spinner className="mr-1 h-3.5 w-3.5" /> : <DownloadCloud size={15} />}
              一键导入（{selected.length}）
            </button>
          </>
        }
      />

      {/* KPI */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard label="已检测智能体" value={installed.length} hint={`注册表共 ${probes.length} 个`} tone="brand" />
        <StatCard label="可导入上游" value={providerCount} hint="OpenAI / Anthropic 兼容端点" tone="blue" />
        <StatCard label="MCP server" value={mcpCount} hint={`已收录 ${mcp.length} 个`} tone="violet" />
        <StatCard
          label="待处理冲突"
          value={conflictCount}
          hint={conflictCount > 0 ? '同名模型已存在，可在选项里允许覆盖' : '无同名冲突'}
          tone={conflictCount > 0 ? 'amber' : 'green'}
        />
      </div>

      {/* 额外目录 + 导入选项 */}
      <div className="card space-y-3 p-4">
        <div className="flex flex-wrap items-center gap-2">
          <FolderPlus size={16} className="text-slate-400" />
          <span className="text-sm font-medium text-slate-700 dark:text-zinc-200">额外目录</span>
          <span className="text-xs text-slate-400">
            用于识别兄弟网关（qoder2api-hub / workbuddy2api-hub 等）：自动读端口与模型目录
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <input
            className="input min-w-[22rem] flex-1"
            placeholder="例如 E:\study\gitproject\aiwork\qoder2api-hub"
            value={rootInput}
            onChange={(e) => setRootInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') addRoot();
            }}
          />
          <button className="btn-outline" onClick={addRoot}>
            添加并扫描
          </button>
          <button className="btn-ghost" onClick={() => setShowOpts(true)}>
            <ShieldCheck size={15} />
            导入选项
          </button>
        </div>
        {extraRoots.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {extraRoots.map((r) => (
              <span key={r} className="chip bg-slate-100 text-slate-600 dark:bg-zinc-800 dark:text-zinc-300">
                {r}
                <button className="ml-1 text-slate-400 hover:text-rose-500" onClick={() => removeRoot(r)} title="移除">
                  ×
                </button>
              </span>
            ))}
          </div>
        )}
        <div className="flex flex-wrap gap-4 text-xs text-slate-500 dark:text-zinc-400">
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={opts.create_models}
              onChange={(e) => setOpts({ ...opts, create_models: e.target.checked })}
            />
            建自定义模型
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={opts.create_keys}
              onChange={(e) => setOpts({ ...opts, create_keys: e.target.checked })}
            />
            建专用 ck_ Key
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={opts.enable_models}
              onChange={(e) => setOpts({ ...opts, enable_models: e.target.checked })}
            />
            新模型默认启用
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={opts.overwrite}
              onChange={(e) => setOpts({ ...opts, overwrite: e.target.checked })}
            />
            同名模型覆盖
          </label>
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={opts.register_mcp}
              onChange={(e) => setOpts({ ...opts, register_mcp: e.target.checked })}
            />
            合并 MCP 注册表
          </label>
        </div>
      </div>

      {/* 主体：左列智能体 / 右列详情 */}
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-[22rem_minmax(0,1fr)]">
        <div className="card max-h-[38rem] overflow-auto p-2">
          {loading && !scan ? (
            <div className="flex items-center justify-center py-12 text-slate-400">
              <Spinner className="mr-2" /> 正在扫描本机智能体配置…
            </div>
          ) : probes.length === 0 ? (
            <EmptyState icon={<Plug size={22} />} title="未发现任何智能体" hint="可在上方添加额外目录" />
          ) : (
            <ul className="space-y-1">
              {probes.map((p) => {
                const importable = p.providers.filter((x) => x.importable).length;
                return (
                  <li key={p.key}>
                    <button
                      onClick={() => setActiveKey(p.key)}
                      className={cn(
                        'w-full rounded-lg px-3 py-2 text-left transition',
                        active?.key === p.key
                          ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900'
                          : 'hover:bg-slate-100 dark:hover:bg-zinc-800',
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate text-sm font-medium">{p.name}</span>
                        {p.installed ? (
                          <CheckCircle2 size={14} className="shrink-0 text-emerald-500" />
                        ) : (
                          <Circle size={14} className="shrink-0 opacity-40" />
                        )}
                      </div>
                      <div
                        className={cn(
                          'mt-0.5 flex items-center gap-2 text-[11px]',
                          active?.key === p.key ? 'opacity-80' : 'text-slate-400',
                        )}
                      >
                        <span>{p.vendor}</span>
                        <span>·</span>
                        <span>{importable} 个上游</span>
                        {p.mcp_servers.length > 0 && <span>· MCP {p.mcp_servers.length}</span>}
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="space-y-4">
          {!active ? (
            <EmptyState icon={<Server size={22} />} title="选择左侧智能体查看详情" />
          ) : (
            <>
              <div className="card p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <div className="flex items-center gap-2">
                      <h2 className="text-base font-semibold text-slate-800 dark:text-zinc-100">{active.name}</h2>
                      <Badge tone={active.installed ? 'green' : 'slate'}>{active.installed ? '已安装' : '未检测到'}</Badge>
                      <Badge tone="blue">{active.protocol}</Badge>
                      {active.connect_supported && <Badge tone="violet">可自动写回</Badge>}
                    </div>
                    <div className="mt-1 text-xs text-slate-400">
                      {active.config_paths.length > 0 ? active.config_paths.join('  ·  ') : '未找到配置文件'}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      className="btn-outline"
                      onClick={() => void runConnect()}
                      disabled={!!busy}
                      title="把本网关写入该智能体配置（写前自动备份）"
                    >
                      {busy === `connect:${active.key}` ? (
                        <Spinner className="mr-1 h-3.5 w-3.5" />
                      ) : (
                        <ArrowUpRight size={15} />
                      )}
                      一键接入本网关
                    </button>
                  </div>
                </div>
                {active.warnings.length > 0 && (
                  <ul className="mt-3 space-y-1 rounded-lg bg-amber-50 p-2 text-[11px] text-amber-700 dark:bg-amber-500/10 dark:text-amber-300">
                    {active.warnings.map((w, i) => (
                      <li key={i} className="flex gap-1.5">
                        <AlertTriangle size={12} className="mt-0.5 shrink-0" />
                        <span>{w}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </div>

              {/* 上游清单 */}
              <div className="card overflow-hidden">
                <div className="flex items-center justify-between border-b border-slate-200 px-4 py-2.5 dark:border-zinc-800">
                  <span className="text-sm font-medium text-slate-700 dark:text-zinc-200">
                    可导入上游（{active.providers.length}）
                  </span>
                  <button
                    className="text-xs text-slate-500 hover:text-slate-800 dark:text-zinc-400 dark:hover:text-zinc-100"
                    onClick={() =>
                      setSelected((prev) => {
                        const ids = active.providers.filter((p) => p.importable).map((p) => p.id);
                        const allOn = ids.every((id) => prev.includes(id));
                        return allOn ? prev.filter((id) => !ids.includes(id)) : [...new Set([...prev, ...ids])];
                      })
                    }
                  >
                    全选 / 取消
                  </button>
                </div>
                {active.providers.length === 0 ? (
                  <div className="px-4 py-8 text-center text-xs text-slate-400">该智能体没有可用上游</div>
                ) : (
                  <table className="w-full text-left text-xs">
                    <thead className="bg-slate-50 text-[11px] uppercase tracking-wide text-slate-400 dark:bg-zinc-900">
                      <tr>
                        <th className="w-8 px-3 py-2" />
                        <th className="px-2 py-2">来源 / 名称</th>
                        <th className="px-2 py-2">Base URL</th>
                        <th className="px-2 py-2">Key</th>
                        <th className="px-2 py-2">模型</th>
                        <th className="px-2 py-2">状态</th>
                      </tr>
                    </thead>
                    <tbody>
                      {active.providers.map((p) => {
                        const on = selected.includes(p.id);
                        return (
                          <tr
                            key={p.id}
                            className={cn(
                              'border-t border-slate-100 dark:border-zinc-800/70',
                              on && 'bg-slate-50/70 dark:bg-zinc-900/40',
                            )}
                          >
                            <td className="px-3 py-2 align-top">
                              <input
                                type="checkbox"
                                checked={on}
                                disabled={!p.importable}
                                onChange={() => toggle(p.id)}
                              />
                            </td>
                            <td className="px-2 py-2 align-top">
                              <div className="font-medium text-slate-700 dark:text-zinc-200">{p.label}</div>
                              <div className="text-[11px] text-slate-400">{p.source}</div>
                            </td>
                            <td className="max-w-[16rem] px-2 py-2 align-top break-all text-slate-600 dark:text-zinc-300">
                              {p.base_url || '-'}
                            </td>
                            <td className="px-2 py-2 align-top font-mono text-[11px] text-slate-500">
                              {p.key_masked || '-'}
                            </td>
                            <td className="px-2 py-2 align-top text-slate-600 dark:text-zinc-300">
                              {p.model || p.models[0] || '-'}
                              {p.models.length > 1 && (
                                <span className="ml-1 text-[11px] text-slate-400">+{p.models.length - 1}</span>
                              )}
                            </td>
                            <td className="px-2 py-2 align-top">
                              {p.importable ? (
                                <Badge tone="green">可导入</Badge>
                              ) : (
                                <span title={p.note}>
                                  <Badge tone="amber">跳过</Badge>
                                </span>
                              )}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </div>

              {/* MCP */}
              {active.mcp_servers.length > 0 && (
                <div className="card p-4">
                  <div className="mb-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
                    MCP server（{active.mcp_servers.length}）
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {active.mcp_servers.map((m) => (
                      <span
                        key={`${m.name}-${m.command}-${m.url}`}
                        className="chip bg-slate-100 text-slate-600 dark:bg-zinc-800 dark:text-zinc-300"
                        title={m.command ? `${m.command} ${m.args.join(' ')}` : m.url}
                      >
                        {m.name}
                        <span className="ml-1 opacity-60">{m.transport}</span>
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {/* 导入计划预览 */}
      {plan && plan.models.length > 0 && (
        <div className="card p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-sm font-medium text-slate-700 dark:text-zinc-200">
              导入预览（{plan.models.length} 个模型 · {plan.keys.length} 个 Key）
            </span>
            <span className="text-xs text-slate-400">实际以「一键导入」结果为准</span>
          </div>
          <div className="max-h-52 overflow-auto">
            <table className="w-full text-left text-xs">
              <thead className="text-[11px] uppercase tracking-wide text-slate-400">
                <tr>
                  <th className="py-1">模型</th>
                  <th className="py-1">上游</th>
                  <th className="py-1">供应商</th>
                  <th className="py-1">判定</th>
                </tr>
              </thead>
              <tbody>
                {plan.models.map((m) => (
                  <tr key={m.provider_id} className="border-t border-slate-100 dark:border-zinc-800/70">
                    <td className="py-1.5 font-mono text-[11px] text-slate-700 dark:text-zinc-200">{m.name}</td>
                    <td className="max-w-[18rem] truncate py-1.5 text-slate-500">{m.base_url}</td>
                    <td className="py-1.5 text-slate-500">{m.vendor}</td>
                    <td className="py-1.5">
                      {m.conflict ? <Badge tone="amber">已存在</Badge> : <Badge tone="green">新增</Badge>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {plan.skipped.length > 0 && (
            <ul className="mt-2 space-y-0.5 text-[11px] text-slate-400">
              {plan.skipped.slice(0, 6).map((s) => (
                <li key={s.provider_id}>
                  跳过 {s.agent_name} · {s.label}：{s.reason}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* 网关接入信息 + 历史 + MCP 注册表 */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <div className="card p-4">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
            <Server size={15} /> 本网关地址
          </div>
          {gateway ? (
            <div className="space-y-1.5 text-xs">
              <div className="flex items-center justify-between gap-2">
                <span className="text-slate-400">Base URL</span>
                <button className="font-mono text-slate-700 hover:underline dark:text-zinc-200" onClick={() => void copy(gateway.base_url_v1)}>
                  {gateway.base_url_v1} <Copy size={11} className="inline" />
                </button>
              </div>
              <div className="flex items-center justify-between gap-2">
                <span className="text-slate-400">默认模型</span>
                <span className="font-mono text-slate-700 dark:text-zinc-200">{gateway.default_model}</span>
              </div>
              <div className="flex items-center justify-between gap-2">
                <span className="text-slate-400">接入 Key 名</span>
                <span className="text-slate-700 dark:text-zinc-200">{gateway.key_name}</span>
              </div>
              <div className="flex items-center justify-between gap-2">
                <span className="text-slate-400">provider id</span>
                <span className="font-mono text-slate-700 dark:text-zinc-200">{gateway.provider_id}</span>
              </div>
            </div>
          ) : (
            <div className="text-xs text-slate-400">读取中…</div>
          )}
        </div>

        <div className="card p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
              <History size={15} /> 导入历史
            </span>
            <span className="text-xs text-slate-400">{history.length} 条</span>
          </div>
          {history.length === 0 ? (
            <div className="text-xs text-slate-400">还没有导入记录</div>
          ) : (
            <ul className="max-h-40 space-y-1 overflow-auto text-xs">
              {history.slice(0, 20).map((h, i) => (
                <li key={`${h.at}-${i}`} className="flex items-center justify-between gap-2">
                  <span className="text-slate-500">{fmtTime(h.at)}</span>
                  <span className="text-slate-700 dark:text-zinc-300">
                    模型 +{h.models_added} / 跳过 {h.models_skipped} / Key +{h.keys_added} / MCP +{h.mcp_added}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="card p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
              <Plug size={15} /> 统一 MCP 注册表
            </span>
            {mcp.length > 0 && (
              <button
                className="text-xs text-slate-400 hover:text-rose-500"
                onClick={async () => {
                  try {
                    const n = await api.agents.mcpClear();
                    setMcp([]);
                    pushToast('success', `已清空 ${n} 个 MCP 条目`);
                  } catch (e) {
                    pushToast('error', String(e));
                  }
                }}
                title="清空注册表"
              >
                <Trash2 size={13} />
              </button>
            )}
          </div>
          {mcp.length === 0 ? (
            <div className="text-xs text-slate-400">导入时会自动合并各智能体的 MCP server</div>
          ) : (
            <ul className="max-h-40 space-y-1 overflow-auto text-xs">
              {mcp.slice(0, 30).map((m) => (
                <li key={`${m.name}-${m.command}-${m.url}`} className="flex items-center justify-between gap-2">
                  <span className="truncate text-slate-700 dark:text-zinc-300">{m.name}</span>
                  <span className="shrink-0 text-slate-400">{m.agent}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {/* 导入结果 */}
      <Modal
        open={!!report}
        onClose={() => setReport(null)}
        title="导入结果"
        size="lg"
        footer={
          <button className="btn-primary" onClick={() => setReport(null)}>
            知道了
          </button>
        }
      >
        {report && (
          <div className="space-y-3">
            <div className="grid grid-cols-4 gap-2 text-center">
              <div className="rounded-lg bg-emerald-50 p-2 dark:bg-emerald-500/10">
                <div className="text-lg font-semibold text-emerald-600 dark:text-emerald-400">{report.models_added}</div>
                <div className="text-[11px] text-slate-500">新增模型</div>
              </div>
              <div className="rounded-lg bg-amber-50 p-2 dark:bg-amber-500/10">
                <div className="text-lg font-semibold text-amber-600 dark:text-amber-400">{report.models_skipped}</div>
                <div className="text-[11px] text-slate-500">跳过模型</div>
              </div>
              <div className="rounded-lg bg-sky-50 p-2 dark:bg-sky-500/10">
                <div className="text-lg font-semibold text-sky-600 dark:text-sky-400">{report.keys_added}</div>
                <div className="text-[11px] text-slate-500">新建 Key</div>
              </div>
              <div className="rounded-lg bg-violet-50 p-2 dark:bg-violet-500/10">
                <div className="text-lg font-semibold text-violet-600 dark:text-violet-400">{report.mcp_added}</div>
                <div className="text-[11px] text-slate-500">MCP 新增</div>
              </div>
            </div>
            <ul className="max-h-64 space-y-1 overflow-auto text-xs text-slate-600 dark:text-zinc-300">
              {report.details.map((d, i) => (
                <li key={i}>· {d}</li>
              ))}
            </ul>
            {report.warnings.length > 0 && (
              <ul className="space-y-1 text-[11px] text-amber-600 dark:text-amber-400">
                {report.warnings.map((w, i) => (
                  <li key={i}>⚠ {w}</li>
                ))}
              </ul>
            )}
          </div>
        )}
      </Modal>

      {/* 接入结果 */}
      <Modal
        open={!!connectResult}
        onClose={() => setConnectResult(null)}
        title={`接入结果 · ${connectResult?.target_name ?? ''}`}
        size="xl"
        bodyClass="max-h-[70vh] overflow-y-auto"
        footer={
          <>
            <button
              className="btn-ghost"
              onClick={() => connectResult && void copy(connectResult.snippet)}
            >
              <Copy size={14} /> 复制片段
            </button>
            <button className="btn-primary" onClick={() => setConnectResult(null)}>
              关闭
            </button>
          </>
        }
      >
        {connectResult && (
          <div className="space-y-3">
            {connectResult.written.length > 0 && (
              <div>
                <div className="mb-1 text-xs font-medium text-emerald-600 dark:text-emerald-400">已写入文件</div>
                <ul className="space-y-0.5 font-mono text-[11px] text-slate-600 dark:text-zinc-300">
                  {connectResult.written.map((w) => (
                    <li key={w}>{w}</li>
                  ))}
                </ul>
              </div>
            )}
            {connectResult.backups.length > 0 && (
              <div>
                <div className="mb-1 text-xs font-medium text-slate-500">备份</div>
                <ul className="space-y-0.5 font-mono text-[11px] text-slate-400">
                  {connectResult.backups.map((b) => (
                    <li key={b}>{b}</li>
                  ))}
                </ul>
              </div>
            )}
            <div>
              <div className="mb-1 text-xs font-medium text-slate-500">配置片段（可直接粘贴）</div>
              <pre className="max-h-72 overflow-auto rounded-lg bg-slate-900 p-3 text-[11px] leading-relaxed text-slate-100">
                {connectResult.snippet}
              </pre>
            </div>
            {connectResult.notes.length > 0 && (
              <ul className="space-y-1 text-[11px] text-slate-500 dark:text-zinc-400">
                {connectResult.notes.map((n, i) => (
                  <li key={i}>· {n}</li>
                ))}
              </ul>
            )}
          </div>
        )}
      </Modal>

      {/* 导入选项说明 */}
      <Modal open={showOpts} onClose={() => setShowOpts(false)} title="导入选项说明" size="lg">
        <div className="space-y-2 text-xs leading-relaxed text-slate-600 dark:text-zinc-300">
          <p>
            <b>建自定义模型</b>：把每个上游写成一条「自定义模型」（名字取配置里的默认模型），
            网关收到同名请求时直达该上游；与既有同名条目冲突时默认跳过。
          </p>
          <p>
            <b>建专用 ck_ Key</b>：为每个来源智能体生成一个「&lt;智能体&gt; 接入」子 Key，
            便于分辨调用来源与单独限流/限额。
          </p>
          <p>
            <b>新模型默认启用</b>：关闭后导入的条目为「停用」，可在资源调度页手动启用（便于先核对再放量）。
          </p>
          <p>
            <b>同名模型覆盖</b>：仅在你确认要换掉现有上游时开启；覆盖前会先删除旧条目再写新条目。
          </p>
          <p>
            <b>合并 MCP 注册表</b>：把各智能体的 <code>mcpServers</code> 去重合并进统一注册表
            （kv <code>agent_mcp_registry</code>），供后续工具编排与导出使用。
          </p>
        </div>
      </Modal>
    </div>
  );
}
