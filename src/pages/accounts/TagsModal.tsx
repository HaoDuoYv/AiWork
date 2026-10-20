import { useEffect, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { Modal } from '../../components/ui';
import type { TagView } from '../../types';

const PRESET_COLORS = [
  '#14b8a6', '#6366f1', '#22c55e', '#f59e0b', '#ef4444', '#0ea5e9', '#a855f7',
];

export function TagsModal({
  open,
  onClose,
  tags,
  onCreate,
  onRename,
  onRecolor,
  onDelete,
}: {
  open: boolean;
  onClose: () => void;
  tags: TagView[];
  onCreate: (name: string, color: string) => Promise<void>;
  onRename: (id: string, name: string) => Promise<void>;
  onRecolor: (id: string, color: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [color, setColor] = useState(PRESET_COLORS[0]);
  const [editingNames, setEditingNames] = useState<Record<string, string>>({});
  const [createBusy, setCreateBusy] = useState(false);
  // 删除标签确认弹框（禁 window.confirm，红线）
  const [deleteTagTarget, setDeleteTagTarget] = useState<TagView | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

  useEffect(() => {
    setEditingNames((prev) => {
      const next = { ...prev };
      tags.forEach((t) => {
        if (!(t.id in next)) next[t.id] = t.name;
      });
      // 清理已删除标签的残留输入值（防 state 无限累积）
      const live = new Set(tags.map((t) => t.id));
      for (const id of Object.keys(next)) {
        if (!live.has(id)) delete next[id];
      }
      return next;
    });
  }, [tags]);

  // 审查 No.13：弹框复用同一组件实例，打开时重置新建表单，避免残留上次的名称/颜色
  useEffect(() => {
    if (open) {
      setName('');
      setColor(PRESET_COLORS[0]);
    }
  }, [open]);

  return (
    <>
      <Modal
      open={open}
      onClose={onClose}
      title="标签管理"
      footer={
        <>
          <button onClick={onClose} className="btn-ghost">关闭</button>
          <button
            onClick={async () => {
              const trimmed = name.trim();
              if (!trimmed || createBusy) return;
              setCreateBusy(true);
              try {
                await onCreate(trimmed, color);
                setName('');
              } catch {
                /* 失败提示由调用方 toast 发出 */
              } finally {
                setCreateBusy(false);
              }
            }}
            disabled={createBusy}
            className="btn-primary"
          >
            <Plus size={14} /> 新建
          </button>
        </>
      }
    >
      <p className="mb-3 text-xs text-slate-400">
        标签为多选标注（分组为单选归属，两者独立）。删除标签后所有账号的该标签绑定与调度/Key 引用会被一并清理。
      </p>
      <div className="mb-4 space-y-3">
        <div>
          <label className="label">新标签名称</label>
          <input value={name} onChange={(e) => setName(e.target.value)} className="input" placeholder="例如：高积分 / 待续期" />
        </div>
        <div>
          <label className="label">颜色</label>
          <div className="flex flex-wrap gap-2">
            {PRESET_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                onClick={() => setColor(c)}
                className={`h-6 w-6 rounded-full border-2 ${color === c ? 'border-slate-900 dark:border-white' : 'border-transparent'}`}
                style={{ background: c }}
                aria-label={c}
              />
            ))}
          </div>
        </div>
      </div>
      <div className="max-h-64 space-y-2 overflow-auto">
        {tags.length === 0 && <div className="text-xs text-slate-400">暂无标签</div>}
        {tags.map((t) => (
          <div key={t.id} className="flex items-center gap-2 rounded-lg border border-slate-200 p-2 dark:border-zinc-700">
            <span className="inline-block h-4 w-4 rounded-full" style={{ background: t.color }} />
            <input
              value={editingNames[t.id] ?? t.name}
              onChange={(e) =>
                setEditingNames((prev) => ({ ...prev, [t.id]: e.target.value }))
              }
              onBlur={(e) => {
                const val = e.target.value.trim();
                if (val && val !== t.name) {
                  onRename(t.id, val)
                    .then(() => {
                      setEditingNames((prev) => {
                        const next = { ...prev };
                        delete next[t.id];
                        return next;
                      });
                    })
                    .catch(() => {
                      // F5（审查修复清单 v1.4.9）：保存失败回退输入框为持久化旧名
                      //（调用方已 toast 失败），不残留未持久化新值误导用户
                      setEditingNames((prev) => ({ ...prev, [t.id]: t.name }));
                    });
                }
              }}
              className="input !py-1 flex-1 !text-xs"
            />
            <select
              value={t.color}
              onChange={(e) => onRecolor(t.id, e.target.value).catch(() => {})}
              className="input !py-1 !text-xs w-24"
            >
              {PRESET_COLORS.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
            <span className="text-xs text-slate-400">{t.count}</span>
            <button
              onClick={() => setDeleteTagTarget(t)}
              className="btn-ghost !p-2 text-rose-500"
              aria-label="删除"
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
    </Modal>
      {/* 删除标签确认弹框（禁 window.confirm，红线） */}
      <Modal
        open={deleteTagTarget != null}
        onClose={() => {
          if (!deleteBusy) setDeleteTagTarget(null);
        }}
        title="删除标签"
        footer={
          <>
            <button className="btn-outline" onClick={() => setDeleteTagTarget(null)} disabled={deleteBusy}>取消</button>
            <button
              className="btn-primary !bg-rose-600 hover:!bg-rose-500"
              onClick={async () => {
                const target = deleteTagTarget;
                if (!target || deleteBusy) return;
                setDeleteBusy(true);
                try {
                  await onDelete(target.id);
                  setDeleteTagTarget(null);
                } catch {
                  /* 保留确认弹框便于重试 */
                } finally {
                  setDeleteBusy(false);
                }
              }}
              disabled={deleteBusy}
            >
              {deleteBusy ? '删除中…' : '确认删除'}
            </button>
          </>
        }
      >
        <div className="text-sm">
          删除标签「{deleteTagTarget?.name}」？{deleteTagTarget?.count ? `当前 ${deleteTagTarget.count} 个账号绑定了该标签，` : ''}
          绑定与调度/API Key 中的引用将被一并清理，该操作不可撤销。
        </div>
      </Modal>
    </>
  );
}

/** 账号标签多选编辑弹框（全量覆盖式提交） */
export function AccountTagsEditModal({
  open,
  onClose,
  accountName,
  tags,
  selected,
  onSave,
}: {
  open: boolean;
  onClose: () => void;
  accountName: string;
  tags: TagView[];
  selected: string[];
  onSave: (tagIds: string[]) => Promise<void>;
}) {
  const [picked, setPicked] = useState<string[]>(selected);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) setPicked(selected);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`标签 — ${accountName}`}
      footer={
        <>
          <button className="btn-outline" onClick={onClose} disabled={busy}>取消</button>
          <button
            className="btn-primary"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onSave(picked);
                onClose();
              } catch {
                /* 失败提示由调用方 toast 发出，保留弹框重试 */
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? '保存中…' : '保存'}
          </button>
        </>
      }
    >
      {tags.length === 0 ? (
        <div className="text-xs text-slate-400">暂无标签，请先在「标签管理」中创建。</div>
      ) : (
        <div className="flex flex-wrap gap-2">
          {tags.map((t) => {
            const on = picked.includes(t.id);
            return (
              <button
                key={t.id}
                type="button"
                onClick={() =>
                  setPicked((prev) => (on ? prev.filter((x) => x !== t.id) : [...prev, t.id]))
                }
                className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs transition-colors ${
                  on
                    ? 'border-transparent text-white'
                    : 'border-slate-200 text-slate-600 dark:border-zinc-700 dark:text-zinc-300'
                }`}
                style={on ? { background: t.color } : undefined}
              >
                <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: on ? '#fff' : t.color }} />
                {t.name}
              </button>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
