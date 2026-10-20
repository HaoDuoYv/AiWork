/**
 * 枢纽增强中心（三项目合并的「增强」落地页）
 *
 * 把两个 Python 兄弟项目里最值钱、且桌面端确实缺的能力搬进来：
 * - **上游池**（wb_upstream_pool.py / qoder_accounts.py）：一个模型挂多个上游，
 *   SWRR 平滑加权 + soft/breaker/degrade 三族退避熔断 + 402 余额停放，
 *   预流式失败自动换端（网关侧 custom_route 已接入）；
 * - **成本核算**（wb_pricing.py）：内容寻址计价策略 + 分档按「覆盖用量的最小上限档」 +
 *   按日×模型聚合，未定价模型显式列示；
 * - **活动时间线**（wb_activity.py）：白名单投影 + 脱敏 + 环形存储；
 * - **出口探测 / 模型目录**（wb_ipintel.py / wb_egress.py / wb_modelsdev.py）：
 *   出口 IP 与延迟、四级能力查找与本地上游补全；
 * - **信封**（wb_atrest.py）：AES-256-GCM + 逐字段 AAD 的导出加密。
 */
import { useCallback, useEffect, useState } from 'react';
import {
  Activity,
  Coins,
  Globe2,
  Layers,
  Lock,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  Zap,
  AlertTriangle,
  CheckCircle2,
  DownloadCloud,
} from 'lucide-react';
import PageHeader from '../../components/PageHeader';
import { Badge, EmptyState, Modal, Spinner, StatCard } from '../../components/ui';
import { useAppStore } from '../../store';
import { api } from '../../lib/tauri';
import { withMinDelay } from '../../lib/delay';
import { cn } from '../../lib/cn';
import type {
  ActivityEvent,
  CatalogStatus,
  CostPolicy,
  CostRecord,
  CostSummary,
  EgressInfo,
  ModelCapability,
  SealedEnvelope,
  UpstreamGroupInput,
  UpstreamGroupView,
} from '../../types';

type TabKey = 'upstream' | 'cost' | 'activity' | 'probe';

const TABS: { key: TabKey; label: string; icon: typeof Layers }[] = [
  { key: 'upstream', label: '上游池', icon: Layers },
  { key: 'cost', label: '成本核算', icon: Coins },
  { key: 'activity', label: '活动时间线', icon: Activity },
  { key: 'probe', label: '探测 / 目录', icon: Globe2 },
];

function fmtTs(sec: number): string {
  if (!sec) return '-';
  const d = new Date(sec * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function fmtCost(v: number): string {
  if (!Number.isFinite(v)) return '0.00';
  return v >= 100 ? v.toFixed(0) : v.toFixed(4).replace(/0+$/, '').replace(/\.$/, '.00');
}

const EMPTY_GROUP: UpstreamGroupInput = {
  model: '',
  strategy: 'swrr',
  endpoints: [
    { id: '', label: '', base_url: '', api_key: '', weight: 1, enabled: true, note: '' },
  ],
  updated_at: 0,
};

const EMPTY_POLICY: CostPolicy = {
  id: '',
  model: '',
  currency: 'CNY',
  flat_in: 0,
  flat_out: 0,
  bands: [],
  usd_cny: 7.2,
  note: '',
  created_at: 0,
};

export default function HubCenter() {
  const pushToast = useAppStore((s) => s.pushToast);
  const [tab, setTab] = useState<TabKey>('upstream');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState('');

  // 上游池
  const [groups, setGroups] = useState<UpstreamGroupView[]>([]);
  const [groupDraft, setGroupDraft] = useState<UpstreamGroupInput | null>(null);
  const [probeResult, setProbeResult] = useState<Record<string, string>>({});

  // 成本
  const [summary, setSummary] = useState<CostSummary | null>(null);
  const [policies, setPolicies] = useState<CostPolicy[]>([]);
  const [records, setRecords] = useState<CostRecord[]>([]);
  const [policyDraft, setPolicyDraft] = useState<CostPolicy | null>(null);
  const [calcIn, setCalcIn] = useState(1_000_000);
  const [calcOut, setCalcOut] = useState(500_000);
  const [calcResult, setCalcResult] = useState<string>('');

  // 活动
  const [events, setEvents] = useState<ActivityEvent[]>([]);
  const [kind, setKind] = useState('');

  // 探测
  const [egress, setEgress] = useState<EgressInfo | null>(null);
  const [catalog, setCatalog] = useState<CatalogStatus | null>(null);
  const [lookupModel, setLookupModel] = useState('glm-5.3');
  const [capability, setCapability] = useState<ModelCapability | null>(null);

  // 信封
  const [seed, setSeed] = useState('');
  const [envelope, setEnvelope] = useState<SealedEnvelope | null>(null);
  const [plain, setPlain] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [g, s, p, r, e, c, a] = await Promise.all([
        api.hub.upstreamGroups().catch(() => [] as UpstreamGroupView[]),
        api.hub.costSummary(30).catch(() => null),
        api.hub.costPolicies().catch(() => [] as CostPolicy[]),
        api.hub.costRecords(60).catch(() => [] as CostRecord[]),
        api.hub.egress(false).catch(() => null),
        api.hub.catalogStatus().catch(() => null),
        api.hub.activityRecent(80, kind || undefined).catch(() => [] as ActivityEvent[]),
      ]);
      setGroups(g);
      setSummary(s);
      setPolicies(p);
      setRecords(r);
      setEgress(e);
      setCatalog(c);
      setEvents(a);
    } finally {
      setLoading(false);
    }
  }, [kind]);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const saveGroup = async () => {
    if (!groupDraft) return;
    setBusy('group');
    try {
      const next = await withMinDelay(api.hub.upstreamSave(groupDraft), 300);
      setGroups(next);
      setGroupDraft(null);
      pushToast('success', `上游组已保存：${groupDraft.model}`);
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const removeGroup = async (model: string) => {
    try {
      await api.hub.upstreamRemove(model);
      setGroups(await api.hub.upstreamGroups());
      pushToast('success', `已删除上游组：${model}`);
    } catch (e) {
      pushToast('error', String(e));
    }
  };

  const testGroup = async (model: string) => {
    setBusy(`test:${model}`);
    try {
      const r = await api.hub.upstreamTest(model);
      setProbeResult((prev) => ({
        ...prev,
        [model]: `${r.ok ? '通过' : '失败'} · ${r.status ?? 'ERR'} · ${r.latency_ms}ms · ${r.detail.slice(0, 120)}`,
      }));
      setGroups(await api.hub.upstreamGroups());
      pushToast(r.ok ? 'success' : 'error', `${model} 探测${r.ok ? '通过' : '失败'}（${r.latency_ms}ms）`);
    } catch (e) {
      setProbeResult((prev) => ({ ...prev, [model]: String(e) }));
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const savePolicy = async () => {
    if (!policyDraft) return;
    setBusy('policy');
    try {
      const saved = await withMinDelay(api.hub.costSave(policyDraft), 300);
      setPolicies(await api.hub.costPolicies());
      setSummary(await api.hub.costSummary(30));
      setPolicyDraft(null);
      pushToast('success', `策略已保存：${saved.model}（${saved.id}）`);
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const removePolicy = async (id: string) => {
    try {
      await api.hub.costRemove(id);
      setPolicies(await api.hub.costPolicies());
      setSummary(await api.hub.costSummary(30));
      pushToast('success', '策略已删除');
    } catch (e) {
      pushToast('error', String(e));
    }
  };

  const calc = async () => {
    if (!policyDraft) return;
    try {
      const r = await api.hub.costPreview(policyDraft, calcIn, calcOut);
      setCalcResult(
        `${fmtCost(r.cost)} ${r.currency}（输入 ${r.unit_in}/1M，输出 ${r.unit_out}/1M）`,
      );
    } catch (e) {
      setCalcResult(String(e));
    }
  };

  const syncCatalog = async () => {
    setBusy('catalog');
    try {
      const n = await withMinDelay(api.hub.catalogSync(), 400);
      setCatalog(await api.hub.catalogStatus());
      pushToast('success', `模型目录已同步：${n} 条`);
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const doLookup = async () => {
    try {
      setCapability(await api.hub.catalogLookup(lookupModel));
    } catch (e) {
      pushToast('error', String(e));
    }
  };

  const sealBundle = async () => {
    if (seed.trim().length < 8) {
      pushToast('error', '口令至少 8 位（建议 16 位以上）');
      return;
    }
    setBusy('seal');
    try {
      const env = await withMinDelay(api.hub.exportBundleSealed(seed), 300);
      setEnvelope(env);
      setPlain('');
      pushToast('success', `接入包已加密导出（key_id=${env.key_id}）`);
    } catch (e) {
      pushToast('error', String(e));
    } finally {
      setBusy('');
    }
  };

  const openBundle = async () => {
    if (!envelope) return;
    try {
      setPlain(await api.hub.open(envelope, seed));
      pushToast('success', '解密成功');
    } catch (e) {
      pushToast('error', String(e));
    }
  };

  return (
    <div className="space-y-5">
      <PageHeader
        title="枢纽增强中心"
        desc="上游池熔断 / 成本核算 / 活动时间线 / 出口探测 / 模型目录 / 加密信封"
        actions={
          <button className="btn-ghost" onClick={() => void load()} disabled={loading}>
            <RefreshCw size={15} className={cn(loading && 'animate-spin')} />
            刷新
          </button>
        }
      />

      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard
          label="上游组"
          value={groups.length}
          hint={`健康端点 ${groups.reduce((n, g) => n + g.healthy_count, 0)}`}
          tone={groups.some((g) => g.healthy_count === 0) ? 'amber' : 'green'}
        />
        <StatCard
          label="30 天成本"
          value={`${fmtCost(summary?.total_cost ?? 0)}`}
          hint={`${summary?.currency ?? 'CNY'} · 策略 ${summary?.policy_count ?? 0} 条`}
          tone="brand"
        />
        <StatCard
          label="请求 / Token"
          value={summary?.total_requests ?? 0}
          hint={`${(summary?.total_in_tokens ?? 0).toLocaleString()} in / ${(summary?.total_out_tokens ?? 0).toLocaleString()} out`}
          tone="blue"
        />
        <StatCard
          label="出口 IP"
          value={egress?.ip || '-'}
          hint={egress?.ok ? `${egress.region || '未知地区'} · ${egress.latency_ms}ms` : egress?.error || '未探测'}
          tone={egress?.ok ? 'violet' : 'amber'}
        />
      </div>

      {/* Tab 头 */}
      <div className="flex flex-wrap items-center gap-1 rounded-lg bg-slate-100 p-1 dark:bg-zinc-900">
        {TABS.map((t) => {
          const Icon = t.icon;
          return (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              className={cn(
                'flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition',
                tab === t.key
                  ? 'bg-white text-zinc-900 shadow-sm dark:bg-zinc-800 dark:text-zinc-100'
                  : 'text-slate-500 hover:text-slate-700 dark:text-zinc-400 dark:hover:text-zinc-200',
              )}
            >
              <Icon size={14} />
              {t.label}
            </button>
          );
        })}
      </div>

      {tab === 'upstream' && (
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-xs text-slate-500 dark:text-zinc-400">
              一个模型可挂多个上游：SWRR 加权轮询选点，失败按 soft/breaker/degrade 三族退避并换端
            </span>
            <button className="btn-primary" onClick={() => setGroupDraft({ ...EMPTY_GROUP, endpoints: [...EMPTY_GROUP.endpoints.map((e) => ({ ...e }))] })}>
              <Plus size={15} /> 新建上游组
            </button>
          </div>
          {groups.length === 0 ? (
            <EmptyState icon={<Layers size={22} />} title="还没有上游组" hint="没有上游组时，自定义模型走单源语义（行为与旧版一致）" />
          ) : (
            groups.map((g) => (
              <div key={g.model} className="card p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm font-semibold text-slate-800 dark:text-zinc-100">{g.model}</span>
                    <Badge tone={g.strategy === 'priority' ? 'blue' : 'violet'}>{g.strategy}</Badge>
                    <Badge tone={g.healthy_count > 0 ? 'green' : 'red'}>
                      {g.healthy_count}/{g.endpoints.length} 健康
                    </Badge>
                  </div>
                  <div className="flex items-center gap-2">
                    <button className="btn-ghost !px-2" onClick={() => void testGroup(g.model)} disabled={!!busy}>
                      {busy === `test:${g.model}` ? <Spinner className="h-3.5 w-3.5" /> : <Zap size={14} />}
                      探测
                    </button>
                    <button
                      className="btn-ghost !px-2"
                      onClick={() =>
                        setGroupDraft({
                          model: g.model,
                          strategy: g.strategy,
                          updated_at: g.updated_at,
                          endpoints: g.endpoints.map((e) => ({
                            id: e.id,
                            label: e.label,
                            base_url: e.base_url,
                            api_key: '',
                            weight: e.weight,
                            enabled: e.enabled,
                            note: e.last_error,
                          })),
                        })
                      }
                    >
                      编辑
                    </button>
                    <button className="btn-ghost !px-2 text-rose-500" onClick={() => void removeGroup(g.model)}>
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
                <div className="mt-2 grid gap-1.5">
                  {g.endpoints.map((e) => (
                    <div
                      key={e.id}
                      className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-slate-50 px-3 py-1.5 text-xs dark:bg-zinc-900/60"
                    >
                      <div className="flex items-center gap-2">
                        {e.healthy ? (
                          <CheckCircle2 size={13} className="text-emerald-500" />
                        ) : (
                          <AlertTriangle size={13} className="text-amber-500" />
                        )}
                        <span className="text-slate-700 dark:text-zinc-200">{e.label}</span>
                        <span className="font-mono text-[11px] text-slate-400">{e.base_url}</span>
                      </div>
                      <div className="flex items-center gap-2 text-[11px] text-slate-400">
                        <span>w={e.weight}</span>
                        {e.fails > 0 && <span className="text-amber-500">fail×{e.fails}</span>}
                        {e.cooling_until > 0 && (
                          <span className="text-rose-500">
                            {e.cooling_reason} · 至 {fmtTs(e.cooling_until)}
                          </span>
                        )}
                        {!e.enabled && <Badge tone="slate">停用</Badge>}
                      </div>
                    </div>
                  ))}
                </div>
                {probeResult[g.model] && (
                  <div className="mt-2 rounded-lg bg-slate-900 px-3 py-1.5 font-mono text-[11px] text-slate-100">
                    {probeResult[g.model]}
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      )}

      {tab === 'cost' && (
        <div className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs text-slate-500 dark:text-zinc-400">
              内容寻址策略：调价产生新 id，历史账目只存引用，不会被改价追溯
            </span>
            <div className="flex gap-2">
              <button className="btn-outline" onClick={() => setPolicyDraft({ ...EMPTY_POLICY, bands: [] })}>
                <Plus size={15} /> 新建策略
              </button>
            </div>
          </div>

          {summary && summary.unpriced.length > 0 && (
            <div className="card border-amber-200 p-3 text-xs text-amber-700 dark:border-amber-500/30 dark:text-amber-300">
              <div className="mb-1 flex items-center gap-1.5 font-medium">
                <AlertTriangle size={13} /> 未定价模型（有请求但没有策略，成本记 0）
              </div>
              <div className="flex flex-wrap gap-1.5">
                {summary.unpriced.map((m) => (
                  <span key={m} className="chip bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300">
                    {m}
                  </span>
                ))}
              </div>
            </div>
          )}

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            <div className="card overflow-hidden">
              <div className="border-b border-slate-200 px-4 py-2 text-sm font-medium text-slate-700 dark:border-zinc-800 dark:text-zinc-200">
                计价策略（{policies.length}）
              </div>
              {policies.length === 0 ? (
                <div className="px-4 py-8 text-center text-xs text-slate-400">还没有策略</div>
              ) : (
                <table className="w-full text-left text-xs">
                  <thead className="bg-slate-50 text-[11px] uppercase tracking-wide text-slate-400 dark:bg-zinc-900">
                    <tr>
                      <th className="px-3 py-2">模型</th>
                      <th className="px-2 py-2">单价（in/out）</th>
                      <th className="px-2 py-2">分档</th>
                      <th className="px-2 py-2">id</th>
                      <th className="px-2 py-2" />
                    </tr>
                  </thead>
                  <tbody>
                    {policies.map((p) => (
                      <tr key={p.id} className="border-t border-slate-100 dark:border-zinc-800/70">
                        <td className="px-3 py-1.5 font-mono text-[11px] text-slate-700 dark:text-zinc-200">{p.model}</td>
                        <td className="px-2 py-1.5 text-slate-600 dark:text-zinc-300">
                          {p.flat_in}/{p.flat_out} {p.currency}
                        </td>
                        <td className="px-2 py-1.5 text-slate-500">{p.bands.length || '-'}</td>
                        <td className="px-2 py-1.5 font-mono text-[11px] text-slate-400">{p.id}</td>
                        <td className="px-2 py-1.5 text-right">
                          <button className="text-slate-400 hover:text-slate-700" onClick={() => setPolicyDraft(p)}>
                            <Save size={13} />
                          </button>
                          <button className="ml-2 text-slate-400 hover:text-rose-500" onClick={() => void removePolicy(p.id)}>
                            <Trash2 size={13} />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>

            <div className="card p-4">
              <div className="mb-2 text-sm font-medium text-slate-700 dark:text-zinc-200">按日成本（近 30 天）</div>
              {!summary || summary.days.length === 0 ? (
                <div className="py-6 text-center text-xs text-slate-400">
                  还没有成本记录（仅有「自定义模型 / 上游组」路径会计价，账号池按积分计费）
                </div>
              ) : (
                <div className="max-h-56 overflow-auto">
                  <table className="w-full text-left text-xs">
                    <thead className="text-[11px] uppercase tracking-wide text-slate-400">
                      <tr>
                        <th className="py-1">日期</th>
                        <th className="py-1">模型</th>
                        <th className="py-1 text-right">请求</th>
                        <th className="py-1 text-right">成本</th>
                      </tr>
                    </thead>
                    <tbody>
                      {summary.days
                        .slice()
                        .reverse()
                        .map((d) => (
                          <tr key={`${d.date}-${d.model}`} className="border-t border-slate-100 dark:border-zinc-800/70">
                            <td className="py-1 text-slate-500">{d.date}</td>
                            <td className="py-1 font-mono text-[11px] text-slate-700 dark:text-zinc-200">{d.model}</td>
                            <td className="py-1 text-right text-slate-500">{d.requests}</td>
                            <td className="py-1 text-right text-slate-700 dark:text-zinc-200">{fmtCost(d.cost)}</td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )}
              {records.length > 0 && (
                <div className="mt-3 max-h-32 overflow-auto border-t border-slate-100 pt-2 dark:border-zinc-800">
                  <table className="w-full text-left text-[11px]">
                    <tbody>
                      {records.slice(0, 12).map((r, i) => (
                        <tr key={`${r.at}-${i}`}>
                          <td className="py-0.5 text-slate-400">{fmtTs(r.at)}</td>
                          <td className="py-0.5 font-mono text-slate-600 dark:text-zinc-300">{r.model}</td>
                          <td className="py-0.5 text-right text-slate-500">
                            {r.in_tokens}/{r.out_tokens}
                          </td>
                          <td className="py-0.5 text-right text-slate-700 dark:text-zinc-200">{fmtCost(r.cost)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {tab === 'activity' && (
        <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              {['', 'gateway', 'upstream', 'agent', 'system'].map((k) => (
                <button
                  key={k || 'all'}
                  onClick={async () => {
                    setKind(k);
                    setEvents(await api.hub.activityRecent(80, k || undefined));
                  }}
                  className={cn(
                    'rounded-md px-2 py-1 text-xs transition',
                    kind === k
                      ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900'
                      : 'text-slate-500 hover:bg-slate-100 dark:hover:bg-zinc-800',
                  )}
                >
                  {k || '全部'}
                </button>
              ))}
            </div>
            <button
              className="btn-ghost text-rose-500"
              onClick={async () => {
                try {
                  const n = await api.hub.activityClear();
                  setEvents([]);
                  pushToast('success', `已清空 ${n} 条活动（JSONL 保留一代）`);
                } catch (e) {
                  pushToast('error', String(e));
                }
              }}
            >
              <Trash2 size={14} /> 清空
            </button>
          </div>
          <div className="card divide-y divide-slate-100 dark:divide-zinc-800">
            {events.length === 0 ? (
              <div className="px-4 py-10 text-center text-xs text-slate-400">暂无活动</div>
            ) : (
              events.map((e, i) => (
                <div key={`${e.at}-${i}`} className="flex items-start gap-3 px-4 py-2 text-xs">
                  <span className="w-24 shrink-0 text-slate-400">{fmtTs(e.at)}</span>
                  <Badge
                    tone={
                      e.kind === 'agent' ? 'violet' : e.kind === 'upstream' ? 'blue' : e.kind === 'system' ? 'slate' : 'green'
                    }
                  >
                    {e.kind}
                  </Badge>
                  <span className="min-w-0 flex-1 text-slate-700 dark:text-zinc-200">{e.summary}</span>
                  <span className="shrink-0 text-[11px] text-slate-400">{e.actor}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {tab === 'probe' && (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          <div className="card p-4">
            <div className="mb-2 flex items-center justify-between">
              <span className="flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
                <Globe2 size={15} /> 出口 IP 情报
              </span>
              <button
                className="btn-ghost !px-2"
                onClick={async () => setEgress(await api.hub.egress(true))}
              >
                <RefreshCw size={13} /> 重新探测
              </button>
            </div>
            {egress ? (
              <div className="space-y-1.5 text-xs">
                <div className="flex justify-between">
                  <span className="text-slate-400">公网 IP</span>
                  <span className="font-mono text-slate-700 dark:text-zinc-200">{egress.ip || '-'}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">地区</span>
                  <span className="text-slate-700 dark:text-zinc-200">{egress.region || '-'}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">运营商</span>
                  <span className="text-slate-700 dark:text-zinc-200">{egress.isp || '-'}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">延迟</span>
                  <span className="text-slate-700 dark:text-zinc-200">{egress.latency_ms} ms</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">探测时间</span>
                  <span className="text-slate-500">{fmtTs(egress.probed_at)}</span>
                </div>
                {egress.error && <div className="text-amber-600 dark:text-amber-400">⚠ {egress.error}</div>}
              </div>
            ) : (
              <div className="text-xs text-slate-400">探测中…</div>
            )}
          </div>

          <div className="card p-4">
            <div className="mb-2 flex items-center justify-between">
              <span className="flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
                <Layers size={15} /> 模型能力目录
              </span>
              <button className="btn-ghost !px-2" onClick={() => void syncCatalog()} disabled={!!busy}>
                {busy === 'catalog' ? <Spinner className="h-3.5 w-3.5" /> : <DownloadCloud size={13} />} 同步远程
              </button>
            </div>
            <div className="space-y-1.5 text-xs">
              <div className="flex justify-between">
                <span className="text-slate-400">缓存条目</span>
                <span className="text-slate-700 dark:text-zinc-200">{catalog?.entries ?? 0}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-400">上次同步</span>
                <span className="text-slate-500">{catalog?.synced_at ? fmtTs(catalog.synced_at) : '未同步'}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-400">状态</span>
                <Badge tone={catalog?.fresh ? 'green' : 'amber'}>{catalog?.fresh ? '新鲜' : '过期/未同步'}</Badge>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-400">来源</span>
                <span className="font-mono text-[11px] text-slate-500">{catalog?.remote_url ?? '-'}</span>
              </div>
            </div>
            <div className="mt-3 flex items-center gap-2">
              <input className="input flex-1" value={lookupModel} onChange={(e) => setLookupModel(e.target.value)} placeholder="模型名" />
              <button className="btn-outline" onClick={() => void doLookup()}>
                查找
              </button>
            </div>
            {capability && (
              <div className="mt-2 rounded-lg bg-slate-50 p-2 text-xs dark:bg-zinc-900/60">
                <div className="flex justify-between">
                  <span className="text-slate-400">上下文</span>
                  <span className="text-slate-700 dark:text-zinc-200">{capability.context_length.toLocaleString()}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">输出上限</span>
                  <span className="text-slate-700 dark:text-zinc-200">
                    {capability.max_output ? capability.max_output.toLocaleString() : '未收录（不编造）'}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-slate-400">来源</span>
                  <Badge tone="blue">{capability.source}</Badge>
                </div>
              </div>
            )}
          </div>

          <div className="card p-4 xl:col-span-2">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium text-slate-700 dark:text-zinc-200">
              <Lock size={15} /> 加密信封（导出接入包）
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <input
                className="input min-w-[16rem] flex-1"
                type="password"
                placeholder="导出口令（≥8 位，建议 16 位以上）"
                value={seed}
                onChange={(e) => setSeed(e.target.value)}
              />
              <button className="btn-primary" onClick={() => void sealBundle()} disabled={!!busy}>
                {busy === 'seal' ? <Spinner className="h-3.5 w-3.5" /> : <Lock size={14} />} 加密导出
              </button>
              <button className="btn-outline" onClick={() => void openBundle()} disabled={!envelope}>
                解密验证
              </button>
            </div>
            <p className="mt-2 text-[11px] text-slate-400">
              AES-256-GCM · key=sha256(口令) · AAD 绑定信封类型/代数/key_id/业务码；口令不落盘，忘记无法恢复。
            </p>
            {envelope && (
              <div className="mt-2 rounded-lg bg-slate-900 p-3 font-mono text-[11px] text-slate-100">
                <div>key_id: {envelope.key_id}</div>
                <div>code: {envelope.code}</div>
                <div>ct: {envelope.ct.slice(0, 48)}…（{envelope.ct.length} chars）</div>
              </div>
            )}
            {plain && (
              <pre className="mt-2 max-h-40 overflow-auto rounded-lg bg-slate-50 p-2 text-[11px] text-slate-600 dark:bg-zinc-900/60 dark:text-zinc-300">
                {plain}
              </pre>
            )}
          </div>
        </div>
      )}

      {/* 上游组编辑 */}
      <Modal
        open={!!groupDraft}
        onClose={() => setGroupDraft(null)}
        title="上游组"
        size="2xl"
        bodyClass="max-h-[70vh] overflow-y-auto"
        footer={
          <>
            <button className="btn-ghost" onClick={() => setGroupDraft(null)}>
              取消
            </button>
            <button className="btn-primary" onClick={() => void saveGroup()} disabled={!!busy}>
              {busy === 'group' ? <Spinner className="h-3.5 w-3.5" /> : <Save size={14} />} 保存
            </button>
          </>
        }
      >
        {groupDraft && (
          <div className="space-y-3">
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <div>
                <div className="label">模型名（路由键）</div>
                <input
                  className="input w-full"
                  value={groupDraft.model}
                  onChange={(e) => setGroupDraft({ ...groupDraft, model: e.target.value })}
                  placeholder="与自定义模型同名，如 glm-5.3"
                />
              </div>
              <div>
                <div className="label">调度策略</div>
                <select
                  className="input w-full"
                  value={groupDraft.strategy}
                  onChange={(e) => setGroupDraft({ ...groupDraft, strategy: e.target.value })}
                >
                  <option value="swrr">swrr（平滑加权轮询）</option>
                  <option value="priority">priority（按顺序取首个健康）</option>
                </select>
              </div>
              <div className="flex items-end">
                <button
                  className="btn-outline w-full"
                  onClick={() =>
                    setGroupDraft({
                      ...groupDraft,
                      endpoints: [
                        ...groupDraft.endpoints,
                        { id: '', label: '', base_url: '', api_key: '', weight: 1, enabled: true, note: '' },
                      ],
                    })
                  }
                >
                  <Plus size={14} /> 添加端点
                </button>
              </div>
            </div>
            <div className="space-y-2">
              {groupDraft.endpoints.map((ep, i) => (
                <div key={i} className="grid grid-cols-1 gap-2 rounded-lg bg-slate-50 p-2 md:grid-cols-12 dark:bg-zinc-900/60">
                  <input
                    className="input md:col-span-3"
                    placeholder="名称（如 主站 / 香港中转）"
                    value={ep.label}
                    onChange={(e) => {
                      const eps = [...groupDraft.endpoints];
                      eps[i] = { ...ep, label: e.target.value };
                      setGroupDraft({ ...groupDraft, endpoints: eps });
                    }}
                  />
                  <input
                    className="input md:col-span-5"
                    placeholder="https://…/v1"
                    value={ep.base_url}
                    onChange={(e) => {
                      const eps = [...groupDraft.endpoints];
                      eps[i] = { ...ep, base_url: e.target.value };
                      setGroupDraft({ ...groupDraft, endpoints: eps });
                    }}
                  />
                  <input
                    className="input md:col-span-2"
                    placeholder="Key（空=用模型条目的）"
                    value={ep.api_key}
                    onChange={(e) => {
                      const eps = [...groupDraft.endpoints];
                      eps[i] = { ...ep, api_key: e.target.value };
                      setGroupDraft({ ...groupDraft, endpoints: eps });
                    }}
                  />
                  <input
                    className="input md:col-span-1"
                    type="number"
                    min={1}
                    value={ep.weight}
                    onChange={(e) => {
                      const eps = [...groupDraft.endpoints];
                      eps[i] = { ...ep, weight: Number(e.target.value) || 1 };
                      setGroupDraft({ ...groupDraft, endpoints: eps });
                    }}
                  />
                  <div className="flex items-center justify-between gap-1 md:col-span-1">
                    <label className="flex items-center gap-1 text-[11px] text-slate-500">
                      <input
                        type="checkbox"
                        checked={ep.enabled}
                        onChange={(e) => {
                          const eps = [...groupDraft.endpoints];
                          eps[i] = { ...ep, enabled: e.target.checked };
                          setGroupDraft({ ...groupDraft, endpoints: eps });
                        }}
                      />
                      启用
                    </label>
                    <button
                      className="text-slate-400 hover:text-rose-500"
                      onClick={() =>
                        setGroupDraft({
                          ...groupDraft,
                          endpoints: groupDraft.endpoints.filter((_, idx) => idx !== i),
                        })
                      }
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>
                </div>
              ))}
            </div>
            <p className="text-[11px] text-slate-400">
              提示：编辑既有组时 Key 显示为空表示「沿用原值/模型条目 Key」；填写则覆盖该端点 Key。
            </p>
          </div>
        )}
      </Modal>

      {/* 计价策略编辑 */}
      <Modal
        open={!!policyDraft}
        onClose={() => setPolicyDraft(null)}
        title="计价策略"
        size="xl"
        bodyClass="max-h-[70vh] overflow-y-auto"
        footer={
          <>
            <button className="btn-ghost" onClick={() => setPolicyDraft(null)}>
              取消
            </button>
            <button className="btn-primary" onClick={() => void savePolicy()} disabled={!!busy}>
              {busy === 'policy' ? <Spinner className="h-3.5 w-3.5" /> : <Save size={14} />} 保存
            </button>
          </>
        }
      >
        {policyDraft && (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
              <div>
                <div className="label">模型名</div>
                <input
                  className="input w-full"
                  value={policyDraft.model}
                  onChange={(e) => setPolicyDraft({ ...policyDraft, model: e.target.value })}
                />
              </div>
              <div>
                <div className="label">输入单价 / 1M</div>
                <input
                  className="input w-full"
                  type="number"
                  step="0.01"
                  value={policyDraft.flat_in}
                  onChange={(e) => setPolicyDraft({ ...policyDraft, flat_in: Number(e.target.value) })}
                />
              </div>
              <div>
                <div className="label">输出单价 / 1M</div>
                <input
                  className="input w-full"
                  type="number"
                  step="0.01"
                  value={policyDraft.flat_out}
                  onChange={(e) => setPolicyDraft({ ...policyDraft, flat_out: Number(e.target.value) })}
                />
              </div>
              <div>
                <div className="label">币种 / USD→CNY</div>
                <div className="flex gap-1">
                  <input
                    className="input w-20"
                    value={policyDraft.currency}
                    onChange={(e) => setPolicyDraft({ ...policyDraft, currency: e.target.value })}
                  />
                  <input
                    className="input flex-1"
                    type="number"
                    step="0.1"
                    value={policyDraft.usd_cny}
                    onChange={(e) => setPolicyDraft({ ...policyDraft, usd_cny: Number(e.target.value) })}
                  />
                </div>
              </div>
            </div>

            <div>
              <div className="mb-1 flex items-center justify-between">
                <span className="label !mb-0">分档（按输入 token；最后一行 upto_input=0 表示无上限）</span>
                <button
                  className="btn-ghost !px-2 text-xs"
                  onClick={() =>
                    setPolicyDraft({
                      ...policyDraft,
                      bands: [...policyDraft.bands, { upto_input: 0, price_in: 0, price_out: 0 }],
                    })
                  }
                >
                  <Plus size={13} /> 添加档位
                </button>
              </div>
              {policyDraft.bands.length === 0 ? (
                <div className="text-[11px] text-slate-400">为空则使用上方 flat 单价</div>
              ) : (
                <div className="space-y-1.5">
                  {policyDraft.bands.map((b, i) => (
                    <div key={i} className="flex items-center gap-2">
                      <input
                        className="input w-28"
                        type="number"
                        value={b.upto_input}
                        onChange={(e) => {
                          const bands = [...policyDraft.bands];
                          bands[i] = { ...b, upto_input: Number(e.target.value) };
                          setPolicyDraft({ ...policyDraft, bands });
                        }}
                      />
                      <input
                        className="input w-24"
                        type="number"
                        step="0.01"
                        value={b.price_in}
                        onChange={(e) => {
                          const bands = [...policyDraft.bands];
                          bands[i] = { ...b, price_in: Number(e.target.value) };
                          setPolicyDraft({ ...policyDraft, bands });
                        }}
                      />
                      <input
                        className="input w-24"
                        type="number"
                        step="0.01"
                        value={b.price_out}
                        onChange={(e) => {
                          const bands = [...policyDraft.bands];
                          bands[i] = { ...b, price_out: Number(e.target.value) };
                          setPolicyDraft({ ...policyDraft, bands });
                        }}
                      />
                      <button
                        className="text-slate-400 hover:text-rose-500"
                        onClick={() =>
                          setPolicyDraft({
                            ...policyDraft,
                            bands: policyDraft.bands.filter((_, idx) => idx !== i),
                          })
                        }
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="rounded-lg bg-slate-50 p-2 dark:bg-zinc-900/60">
              <div className="mb-1 text-xs font-medium text-slate-600 dark:text-zinc-300">试算</div>
              <div className="flex flex-wrap items-center gap-2 text-xs">
                <input
                  className="input w-28"
                  type="number"
                  value={calcIn}
                  onChange={(e) => setCalcIn(Number(e.target.value))}
                />
                <span className="text-slate-400">in</span>
                <input
                  className="input w-28"
                  type="number"
                  value={calcOut}
                  onChange={(e) => setCalcOut(Number(e.target.value))}
                />
                <span className="text-slate-400">out</span>
                <button className="btn-outline" onClick={() => void calc()}>
                  计算
                </button>
                {calcResult && <span className="font-mono text-slate-700 dark:text-zinc-200">{calcResult}</span>}
              </div>
            </div>

            <div>
              <div className="label">备注</div>
              <input
                className="input w-full"
                value={policyDraft.note}
                onChange={(e) => setPolicyDraft({ ...policyDraft, note: e.target.value })}
                placeholder="如：官方价 / 中转站折扣价"
              />
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}
