/**
 * 全局 API 管理 · 资源总览与调度策略中心（unified-api-gateway-design §5.2/§5.3）
 * 「调度策略中心」卡集中三块（任务7，从资源角度整体设计）：
 *  ① 最佳组合预设：5 组专家组合（池间 + Trae / Buddy / Qoder 池内策略）一键应用，
 *     当前命中组合高亮（Qoder 与 Trae/Buddy 同等参与池间调度，默认序尾部可上移）；
 *  ② 池间调度策略：smart / priority + 优先级序编辑 + 跨池回退（dispatch_policy_get/set 即时生效）；
 *  ③ 各资源池池内调度：Trae（strategy）/ Buddy（wb_strategy，空 = 跟随 Trae 池）/
 *     Qoder（qoder_strategy，空 = 跟随 Trae 池）下拉编辑，自定义模型命中即直达；
 *     上游开关与池成员在各应用「资源调度」页维护。
 * 下方为 Trae / Buddy / Qoder / 自定义 四池摘要卡；前三者详情引导至各应用「资源调度」页，
 * 自定义池详情引导至「自定义模型」Tab。
 * 数据源：pool_list / accounts_list（Trae 池）、pool_list.wb_enabled / workbuddy_accounts_list
 * （Buddy 池）、pool_list.qoder_enabled / qoder_accounts_list（Qoder 池）、custom_models_list
 * （自定义模型池）。
 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ArrowDown, ArrowUp, Blocks, Bot, Boxes, Layers, Server, SlidersHorizontal, Star, X } from 'lucide-react';
import { Badge } from '../ui';
import { api } from '../../lib/tauri';
import { useAppStore } from '../../store';
import { fmtCredits } from '../../lib/format';
import { DISPATCH_PRESETS, matchPreset } from './dispatchPresets';
import { computePoolMetrics } from './poolMetrics';
import type { AccountView, ApiPoolFile, CustomModel, DispatchPolicy, QoderAccountView, WorkBuddyAccountView } from '../../types';

function Row({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 text-xs">
      <span className="text-slate-400 dark:text-zinc-500">{label}</span>
      <span className="tabular-nums text-slate-700 dark:text-zinc-200">{value}</span>
    </div>
  );
}

/** 池内调度策略取值（Trae / Buddy 同一套，pool.rs PoolStrategy） */
const STRATEGIES = ['expire_first', 'credit_first', 'random', 'weighted', 'p2c'] as const;

/** 池内调度策略文案（与账号池选择下拉一致） */
const STRATEGY_LABELS: Record<string, string> = {
  expire_first: '积分先过期优先',
  credit_first: '剩余积分多优先',
  random: '随机',
  weighted: '三因子加权随机',
  p2c: 'P2C 随机二选一',
};

const POOL_LABELS: Record<string, string> = {
  buddy: 'Buddy 池',
  trae: 'Trae 池',
  qoder: 'Qoder 池',
};

const selectCls =
  'h-7 rounded-md border border-slate-200 bg-white px-2 text-xs text-slate-700 outline-none focus:border-brand-400 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-200';

const inputCls =
  'h-7 w-44 rounded-md border border-slate-200 bg-white px-2 text-xs text-slate-700 outline-none placeholder:text-slate-300 focus:border-brand-400 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-200';

/** per_model 候选序选项：三池全排列（6 种，覆盖全部合法顺序） */
const POOL_ORDERS = [
  ['buddy', 'trae', 'qoder'],
  ['buddy', 'qoder', 'trae'],
  ['trae', 'buddy', 'qoder'],
  ['trae', 'qoder', 'buddy'],
  ['qoder', 'buddy', 'trae'],
  ['qoder', 'trae', 'buddy'],
].map((arr) => arr.join('>'));

const orderKey = (arr: string[]) => arr.join('>');

const orderLabel = (k: string) =>
  k
    .split('>')
    .map((p) => POOL_LABELS[p] ?? p)
    .join(' → ');

/**
 * 调度策略中心卡：预设 + 池间 + 池内三块集中管理。
 * pool / onPoolChanged 由父级持有（三池摘要卡共用同一份数据）。
 */
function DispatchCenterCard({
  pool,
  onPoolChanged,
}: {
  pool: ApiPoolFile | null;
  onPoolChanged: (p: ApiPoolFile) => void;
}) {
  const toast = useAppStore((s) => s.pushToast);
  const [policy, setPolicy] = useState<DispatchPolicy | null>(null);
  const [saving, setSaving] = useState(false);
  // per_model 新增绑定行（本地输入态，添加时才落 policy）
  const [newPerModelName, setNewPerModelName] = useState('');
  const [newPerModelOrder, setNewPerModelOrder] = useState(POOL_ORDERS[0]);
  // 账号保留剩余积分（issue #91）输入草稿：null = 未编辑（展示 policy 生效值），
  // onBlur 才落盘。审查 F14 修复：改存原始字符串——旧实现 Number('')=0 使「清空
  // 重输」直接跳 0，误失焦即静默保存 0（=关闭调度保护）；现空串/非法输入失焦时
  // 还原策略值不落盘，合法整数才提交（保存侧 fail-closed，无失焦抢跑钳制）
  const [reserveDraft, setReserveDraft] = useState<string | null>(null);
  // F2：保存请求序号守卫（仅最新一次响应可写状态）
  const saveSeqRef = useRef(0);

  useEffect(() => {
    api.apiServer
      .dispatchPolicyGet()
      .then(setPolicy)
      .catch(() => {
        /* 保留空态 */
      });
  }, []);

  /** 当前组合命中的预设（匹配规则见 dispatchPresets.matchPreset） */
  const activePreset = useMemo(() => matchPreset(pool, policy), [pool, policy]);

  /** 统一保存入口：inter 走 dispatch_policy_set；trae/buddy/qoder 走 pool_set（未传维度保留原值） */
  const save = async (patch: {
    inter?: DispatchPolicy['strategy'];
    trae?: string;
    buddy?: string;
    qoder?: string;
  }) => {
    if (!pool) return;
    const trae = patch.trae ?? pool.strategy ?? '';
    const buddy = patch.buddy ?? pool.wb_strategy ?? '';
    const qoder = patch.qoder ?? pool.qoder_strategy ?? '';
    setSaving(true);
    try {
      if (patch.inter && policy) {
        setPolicy(await api.apiServer.dispatchPolicySet({ ...policy, strategy: patch.inter }));
      }
      // uids/groups 原样回传（本卡不改成员与分组）；wbFlags 仅带 Qoder 池内策略，
      // 其余未传字段 → 后端保留原值
      await api.apiServer.poolSet(
        pool.enabled_uids,
        trae,
        pool.group_ids,
        { qoderStrategy: qoder },
        buddy,
      );
      onPoolChanged({ ...pool, strategy: trae, wb_strategy: buddy, qoder_strategy: qoder });
      toast('success', '调度策略已保存，运行中网关即时生效');
    } catch (e) {
      toast('error', `调度策略保存失败：${String(e).slice(0, 120)}`);
    } finally {
      setSaving(false);
    }
  };

  /** 优先级数组内移动（priority 模式编辑） */
  const move = (i: number, dir: -1 | 1) => {
    if (!policy) return;
    const next = policy.priority.slice();
    const j = i + dir;
    if (j < 0 || j >= next.length) return;
    [next[i], next[j]] = [next[j], next[i]];
    void savePolicy({ priority: next });
  };

  const savePolicy = async (patch: Partial<DispatchPolicy>) => {
    if (!policy) return;
    // F2（审查修复清单 v1.4.9）：per_model 快速连续切换时保存响应可能乱序返回，
    // 旧响应覆盖最后选择——序号守卫：仅最新一次请求允许 setState/toast/saving 复位
    const seq = ++saveSeqRef.current;
    setSaving(true);
    try {
      const next = await api.apiServer.dispatchPolicySet({ ...policy, ...patch });
      if (seq === saveSeqRef.current) {
        setPolicy(next);
        // 审查修复（中等项）：写操作必须有可见反馈——成功也提示（对齐 L140 既有口径）
        toast('success', '调度策略已保存，运行中网关即时生效');
      }
    } catch (e) {
      if (seq === saveSeqRef.current) {
        toast('error', `调度策略保存失败：${String(e).slice(0, 120)}`);
      }
    } finally {
      if (seq === saveSeqRef.current) setSaving(false);
    }
  };

  if (!policy) return null;

  return (
    <div className="card p-4">
      {/* 头部 */}
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <SlidersHorizontal size={16} className="text-brand-500" />
        <span className="text-sm font-medium text-slate-800 dark:text-zinc-100">调度策略中心</span>
        {activePreset ? (
          <Badge tone="blue">{activePreset.name}</Badge>
        ) : (
          pool && <Badge tone="slate">自定义组合</Badge>
        )}
        {saving && <span className="text-[11px] text-slate-400">保存中…</span>}
        <span className="ml-auto text-[11px] text-slate-400">运行中网关即时生效，无需重启</span>
      </div>

      {/* ① 最佳组合预设 */}
      <div className="mb-3">
        <div className="mb-1.5 text-[11px] font-medium text-slate-500 dark:text-zinc-400">
          最佳组合预设（池间 + Trae / Buddy / Qoder 池内策略 一键应用）
        </div>
        <div className="flex flex-wrap gap-1.5">
          {DISPATCH_PRESETS.map((p) => {
            const active = activePreset?.key === p.key;
            return (
              <button
                key={p.key}
                title={p.desc}
                disabled={!pool || saving}
                onClick={() => void save({ inter: p.inter, trae: p.trae, buddy: p.buddy, qoder: p.qoder })}
                className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-xs transition-colors disabled:opacity-50 ${
                  active
                    ? 'border-brand-500 bg-brand-50 font-semibold text-brand-600 dark:bg-brand-500/10 dark:text-brand-400'
                    : 'border-slate-200 bg-white text-slate-600 hover:border-brand-300 hover:text-brand-600 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-300'
                }`}
              >
                {p.recommended && <Star size={11} className="text-amber-500" fill="currentColor" />}
                {p.name}
              </button>
            );
          })}
        </div>
        <p className="mt-1.5 text-[11px] text-slate-400 dark:text-zinc-500">
          {activePreset ? activePreset.desc : '当前为手动组合；选择预设可一键对齐推荐配置'}
        </p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-zinc-500">
          Qoder 池与 Trae / Buddy 同等地位参与策略调度（默认优先级序位于尾部，可在「固定优先级」中上移）。
        </p>
      </div>

      {/* ② 池间调度策略 */}
      <div className="border-t border-slate-100 pt-3 dark:border-zinc-800">
        <div className="mb-1.5 text-[11px] font-medium text-slate-500 dark:text-zinc-400">
          池间调度（仅作用于多源同名模型 Trae/Buddy/Qoder；自定义模型命中即直达不受影响）
        </div>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
          <label className="flex items-center gap-2">
            <span className="text-slate-400 dark:text-zinc-500">策略</span>
            <select
              className={selectCls}
              value={policy.strategy}
              onChange={(e) => void savePolicy({ strategy: e.target.value as DispatchPolicy['strategy'] })}
            >
              <option value="smart">智能调度（默认）</option>
              <option value="priority">固定优先级</option>
            </select>
          </label>
          {policy.strategy === 'priority' && (
            <div className="flex items-center gap-1.5">
              <span className="text-slate-400 dark:text-zinc-500">优先级</span>
              {policy.priority.map((p, i) => (
                <span
                  key={p}
                  className="flex items-center gap-0.5 rounded-full border border-slate-200 py-0.5 pl-2 pr-0.5 dark:border-zinc-700"
                >
                  {POOL_LABELS[p] ?? p}
                  <button
                    className="rounded p-0.5 text-slate-400 hover:text-brand-500 disabled:opacity-30"
                    disabled={i === 0}
                    onClick={() => move(i, -1)}
                    title="上移"
                  >
                    <ArrowUp size={11} />
                  </button>
                  <button
                    className="rounded p-0.5 text-slate-400 hover:text-brand-500 disabled:opacity-30"
                    disabled={i === policy.priority.length - 1}
                    onClick={() => move(i, 1)}
                    title="下移"
                  >
                    <ArrowDown size={11} />
                  </button>
                </span>
              ))}
            </div>
          )}
          <label className="flex items-center gap-1.5 text-slate-500 dark:text-zinc-400">
            <input
              type="checkbox"
              checked={policy.fallback}
              onChange={(e) => void savePolicy({ fallback: e.target.checked })}
            />
            首选池不可用时跨池回退
          </label>
          {/* issue #90 修复③a：多源同名模型来源标识（/v1/models 池前缀别名条目） */}
          <label
            className="flex items-center gap-1.5 text-slate-500 dark:text-zinc-400"
            title="/v1/models 为多源同名模型附带 trae/xxx、buddy/xxx、qoder/xxx 别名条目，客户端可按前缀点名指定池（三家同名模型倍率与效果不同）；关闭时目录保持原形态"
          >
            <input
              type="checkbox"
              checked={policy.model_source_labels}
              onChange={(e) => void savePolicy({ model_source_labels: e.target.checked })}
            />
            模型目录标注来源池
          </label>
          <label
            className="flex items-center gap-2"
            title="剩余积分低于或等于该值的账号暂停调度（切换其他账号），防止积分被耗尽后免费模型也不可用；0 = 关闭"
          >
            <span className="text-slate-400 dark:text-zinc-500">账号保留剩余积分</span>
            <input
              type="number"
              className={`${selectCls} w-20`}
              min={0}
              step={1}
              value={reserveDraft ?? String(policy.reserve_credits)}
              disabled={saving}
              onChange={(e) => setReserveDraft(e.target.value)}
              onBlur={() => {
                if (reserveDraft == null) return;
                const t = reserveDraft.trim();
                // 空串/非法数值：放弃编辑还原策略值（不静默清零/落盘）；
                // 接受小数（后端 reserve_credits 为 f64，避免既有小数值无法往返编辑）
                if (t === '' || !/^\d+(\.\d+)?$/.test(t)) {
                  setReserveDraft(null);
                  return;
                }
                const v = Math.max(0, Number(t));
                if (v !== policy.reserve_credits) {
                  void savePolicy({ reserve_credits: v });
                }
                setReserveDraft(null);
              }}
            />
            <span className="text-slate-400 dark:text-zinc-500">积分</span>
          </label>
        </div>
        <p className="mt-1.5 text-[11px] text-slate-400 dark:text-zinc-500">
          智能调度按请求模型对可用池排序：积分先到期优先 → 免费 / 倍率小优先 → 剩余积分多优先；全并列时按上方优先级序。
        </p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-zinc-500">
          账号保留剩余积分：剩余 ≤ 阈值的账号暂停调度（默认 50，0 = 关闭），防止积分被调度耗尽后免费模型也不可用。
        </p>

        {/* ②+ per_model 模型绑定池（DispatchPolicy.per_model）：显式为单个模型固定
            池候选序，优先于全局策略且不做智能重排（后端 dispatch.rs 有绑定序即最终序） */}
        <div className="mt-3">
          <div className="mb-1.5 text-[11px] font-medium text-slate-500 dark:text-zinc-400">
            模型绑定池（可选：为单个模型固定池候选序，优先于上方全局策略）
          </div>
          {Object.keys(policy.per_model).length === 0 ? (
            <p className="text-[11px] text-slate-400 dark:text-zinc-500">
              暂无绑定，全部模型按上方全局策略调度。
            </p>
          ) : (
            <div className="space-y-1.5">
              {Object.entries(policy.per_model).map(([model, order]) => (
                <div key={model} className="flex items-center gap-2 text-xs">
                  <span
                    className="w-44 truncate font-mono text-slate-700 dark:text-zinc-200"
                    title={model}
                  >
                    {model}
                  </span>
                  <select
                    className={selectCls}
                    value={orderKey(order)}
                    onChange={(e) => {
                      const next = { ...policy.per_model, [model]: e.target.value.split('>') };
                      void savePolicy({ per_model: next });
                    }}
                  >
                    {POOL_ORDERS.map((k) => (
                      <option key={k} value={k}>
                        {orderLabel(k)}
                      </option>
                    ))}
                    {!POOL_ORDERS.includes(orderKey(order)) && (
                      <option value={orderKey(order)}>{orderKey(order)}（自定义）</option>
                    )}
                  </select>
                  <button
                    className="rounded p-1 text-slate-400 hover:text-rose-500"
                    title="移除绑定"
                    disabled={saving}
                    onClick={() => {
                      const next = { ...policy.per_model };
                      delete next[model];
                      void savePolicy({ per_model: next });
                    }}
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="mt-1.5 flex items-center gap-2">
            <input
              className={inputCls}
              placeholder="模型名（如 gpt-4.1）"
              value={newPerModelName}
              onChange={(e) => setNewPerModelName(e.target.value)}
            />
            <select
              className={selectCls}
              value={newPerModelOrder}
              onChange={(e) => setNewPerModelOrder(e.target.value)}
            >
              {POOL_ORDERS.map((k) => (
                <option key={k} value={k}>
                  {orderLabel(k)}
                </option>
              ))}
            </select>
            <button
              className="h-7 rounded-md border border-slate-200 px-2.5 text-xs text-slate-600 transition-colors hover:border-brand-300 hover:text-brand-600 disabled:opacity-40 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-300"
              disabled={!newPerModelName.trim() || saving || Boolean(policy.per_model[newPerModelName.trim()])}
              title={policy.per_model[newPerModelName.trim()] ? '该模型已绑定' : '添加绑定'}
              onClick={() => {
                const model = newPerModelName.trim();
                if (!model || policy.per_model[model]) return;
                const next = { ...policy.per_model, [model]: newPerModelOrder.split('>') };
                void savePolicy({ per_model: next });
                setNewPerModelName('');
              }}
            >
              添加
            </button>
          </div>
        </div>
      </div>

      {/* ③ 各资源池池内调度 */}
      <div className="mt-3 border-t border-slate-100 pt-3 dark:border-zinc-800">
        <div className="mb-1.5 text-[11px] font-medium text-slate-500 dark:text-zinc-400">
          各资源池池内调度（账号取号顺序；成员与分组在各应用「资源调度」页维护）
        </div>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2 text-xs">
          <label className="flex items-center gap-2">
            <span className="text-slate-400 dark:text-zinc-500">Trae 池</span>
            <select
              className={selectCls}
              value={pool?.strategy || 'expire_first'}
              disabled={!pool}
              onChange={(e) => void save({ trae: e.target.value })}
            >
              {STRATEGIES.map((s) => (
                <option key={s} value={s}>
                  {STRATEGY_LABELS[s]}
                  {s === 'expire_first' ? '（默认）' : ''}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2">
            <span className="text-slate-400 dark:text-zinc-500">Buddy 池</span>
            <select
              className={selectCls}
              value={pool?.wb_strategy ?? ''}
              disabled={!pool}
              onChange={(e) => void save({ buddy: e.target.value })}
            >
              <option value="">跟随 Trae 池</option>
              {STRATEGIES.map((s) => (
                <option key={s} value={s}>
                  {STRATEGY_LABELS[s]}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2">
            <span className="text-slate-400 dark:text-zinc-500">Qoder 池</span>
            <select
              className={selectCls}
              value={pool?.qoder_strategy ?? ''}
              disabled={!pool}
              onChange={(e) => void save({ qoder: e.target.value })}
            >
              <option value="">跟随 Trae 池</option>
              {STRATEGIES.map((s) => (
                <option key={s} value={s}>
                  {STRATEGY_LABELS[s]}
                </option>
              ))}
            </select>
          </label>
          <span className="text-slate-400 dark:text-zinc-500">
            自定义模型：<span className="text-slate-700 dark:text-zinc-200">命中即直达</span>（上游自有计费，无池内调度）
          </span>
          <span className="text-slate-400 dark:text-zinc-500">
            上游开关与成员在各应用「资源调度」页配置
          </span>
        </div>
      </div>
    </div>
  );
}

export default function ResourceSummary() {
  const [pool, setPool] = useState<ApiPoolFile | null>(null);
  const [accounts, setAccounts] = useState<AccountView[]>([]);
  const [wbAccounts, setWbAccounts] = useState<WorkBuddyAccountView[]>([]);
  const [qoderAccounts, setQoderAccounts] = useState<QoderAccountView[]>([]);
  const [customModels, setCustomModels] = useState<CustomModel[]>([]);

  useEffect(() => {
    api.apiServer
      .poolList()
      .then(setPool)
      .catch(() => {
        /* 保留空摘要 */
      });
    api.accounts
      .list()
      .then(setAccounts)
      .catch(() => {
        /* 保留空摘要 */
      });
    api.workbuddy
      .accountsList()
      .then(setWbAccounts)
      .catch(() => {
        /* 保留空摘要 */
      });
    api.qoder
      .accountsList()
      .then(setQoderAccounts)
      .catch(() => {
        /* 保留空摘要 */
      });
    api.apiServer
      .customModelsList()
      .then(setCustomModels)
      .catch(() => {
        /* 保留空摘要 */
      });
  }, []);

  // 四池摘要指标（池内成员与积分口径的纯计算见 poolMetrics.ts，含单测）
  const metrics = computePoolMetrics(pool, accounts, wbAccounts, customModels, qoderAccounts);
  // Buddy 池内策略文案：空 = 跟随 Trae 池（展示 Trae 当前生效策略）
  const buddyStrategyText = pool?.wb_strategy
    ? (STRATEGY_LABELS[pool.wb_strategy] ?? pool.wb_strategy)
    : pool?.strategy
      ? `跟随 Trae 池（${STRATEGY_LABELS[pool.strategy] ?? pool.strategy}）`
      : '—';
  // Qoder 池内策略文案：空 = 跟随 Trae 池（同 Buddy 语义）
  const qoderStrategyText = pool?.qoder_strategy
    ? (STRATEGY_LABELS[pool.qoder_strategy] ?? pool.qoder_strategy)
    : pool?.strategy
      ? `跟随 Trae 池（${STRATEGY_LABELS[pool.strategy] ?? pool.strategy}）`
      : '—';

  // 自定义模型池：启用条目数 / 总条目数
  const customEnabled = customModels.filter((m) => m.enabled).length;

  return (
    <div className="space-y-4">
      {/* 调度策略中心（预设 + 池间 + 池内集中管理，整宽一行） */}
      <DispatchCenterCard pool={pool} onPoolChanged={setPool} />

      {/* 四池摘要卡（Trae / Buddy / Qoder / 自定义）同行等宽排布 */}
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
      {/* Trae 资源池摘要 */}
      <div className="card p-4">
        <div className="mb-3 flex items-center gap-2">
          <Server size={16} className="text-brand-500" />
          <span className="text-sm font-medium text-slate-800 dark:text-zinc-100">Trae 资源池</span>
          {/* 对齐 Buddy/Qoder 卡口径：上游开关状态（原「可用/空池」与池成员数语义重复） */}
          <Badge tone={(pool?.trae_enabled ?? true) ? 'green' : 'slate'}>
            {(pool?.trae_enabled ?? true) ? '上游已启用' : '上游未启用'}
          </Badge>
        </div>
        <div className="space-y-1.5">
          <Row label="池内账号数" value={metrics.traePoolCount} />
          <Row
            label="可用积分"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {fmtCredits(metrics.traePoolCredits)}
              </span>
            }
          />
          <Row label="账号总数" value={metrics.traeAccountTotal} />
          <Row
            label="积分总余额"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {fmtCredits(metrics.traeTotalCredits)}
              </span>
            }
          />
          <Row
            label="池内调度策略"
            value={pool?.strategy ? (STRATEGY_LABELS[pool.strategy] ?? pool.strategy) : '—'}
          />
          <Row label="消耗口径" value="通用积分（product_id 208）" />
        </div>
        <p className="mt-3 border-t border-slate-100 pt-2 text-[11px] text-slate-400 dark:border-zinc-800 dark:text-zinc-500">
          详情（账号池选择 / 分组筛选 / 模型目录）见 Trae「资源调度」页
        </p>
      </div>

      {/* Buddy 资源池摘要 */}
      <div className="card p-4">
        <div className="mb-3 flex items-center gap-2">
          <Bot size={16} className="text-violet-500" />
          <span className="text-sm font-medium text-slate-800 dark:text-zinc-100">Buddy 资源池</span>
          <Badge tone={metrics.buddyEnabled ? 'green' : 'slate'}>
            {metrics.buddyEnabled ? '上游已启用' : '上游未启用'}
          </Badge>
        </div>
        <div className="space-y-1.5">
          <Row label="池内账号数" value={metrics.buddyPoolCount} />
          <Row
            label="可用积分"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {metrics.buddyPoolCredits != null ? fmtCredits(metrics.buddyPoolCredits) : '未知'}
              </span>
            }
          />
          <Row label="账号总数" value={metrics.buddyAccountTotal} />
          <Row
            label="积分总余额"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {metrics.buddyTotalCredits != null ? fmtCredits(metrics.buddyTotalCredits) : '未知'}
              </span>
            }
          />
          <Row label="池内调度策略" value={buddyStrategyText} />
          <Row label="消耗口径" value="Buddy 积分（按模型倍率）" />
        </div>
        <p className="mt-3 border-t border-slate-100 pt-2 text-[11px] text-slate-400 dark:border-zinc-800 dark:text-zinc-500">
          <Layers size={11} className="mr-1 inline" />
          详情（上游开关 / 模型目录 / 账号池）见 Buddy「资源调度」页
        </p>
      </div>

      {/* Qoder 资源池摘要 */}
      <div className="card p-4">
        <div className="mb-3 flex items-center gap-2">
          <Boxes size={16} className="text-emerald-500" />
          <span className="text-sm font-medium text-slate-800 dark:text-zinc-100">Qoder 资源池</span>
          <Badge tone={metrics.qoderEnabled ? 'green' : 'slate'}>
            {metrics.qoderEnabled ? '上游已启用' : '上游未启用'}
          </Badge>
        </div>
        <div className="space-y-1.5">
          <Row label="池内账号数" value={metrics.qoderPoolCount} />
          <Row
            label="可用积分"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {metrics.qoderPoolCredits != null ? fmtCredits(metrics.qoderPoolCredits) : '未知'}
              </span>
            }
          />
          <Row label="账号总数" value={metrics.qoderAccountTotal} />
          <Row
            label="积分总余额"
            value={
              <span className="font-semibold text-amber-600 dark:text-amber-400">
                {metrics.qoderTotalCredits != null ? fmtCredits(metrics.qoderTotalCredits) : '未知'}
              </span>
            }
          />
          <Row label="池内调度策略" value={qoderStrategyText} />
          <Row label="消耗口径" value="Qoder 通用 credits" />
        </div>
        <p className="mt-3 border-t border-slate-100 pt-2 text-[11px] text-slate-400 dark:border-zinc-800 dark:text-zinc-500">
          <Layers size={11} className="mr-1 inline" />
          详情（上游开关 / 模型目录 / 账号清单）见 Qoder「资源调度」页
        </p>
      </div>

      {/* 自定义模型池摘要 */}
      <div className="card p-4">
        <div className="mb-3 flex items-center gap-2">
          <Blocks size={16} className="text-sky-500" />
          <span className="text-sm font-medium text-slate-800 dark:text-zinc-100">自定义资源池</span>
          <Badge tone={metrics.customEnabledCount > 0 ? 'green' : 'slate'}>
            {metrics.customEnabledCount > 0 ? '调度中' : '无启用条目'}
          </Badge>
        </div>
        <div className="space-y-1.5">
          <Row label="启用模型数" value={metrics.customEnabledCount} />
          <Row label="模型总数" value={metrics.customModelTotal} />
          <Row label="消耗口径" value="上游自有计费" />
          <Row label="调度语义" value="模型名命中即直达" />
        </div>
        <p className="mt-3 border-t border-slate-100 pt-2 text-[11px] text-slate-400 dark:border-zinc-800 dark:text-zinc-500">
          <Blocks size={11} className="mr-1 inline" />
          详情（新增 / 编辑 / 启停）见「自定义模型」Tab
        </p>
      </div>
      </div>
    </div>
  );
}
