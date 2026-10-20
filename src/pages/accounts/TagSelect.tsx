import type { TagView } from '../../types';

/** 行内标签 chips（最多显示 max 个 + 「+N」溢出；点击打开编辑弹框） */
export function TagChips({
  tagIds,
  allTags,
  max = 2,
  onEdit,
}: {
  tagIds: string[] | undefined;
  allTags: TagView[];
  max?: number;
  onEdit: () => void;
}) {
  const ids = tagIds ?? [];
  const defs = ids
    .map((id) => allTags.find((t) => t.id === id))
    .filter((t): t is TagView => !!t);
  const shown = defs.slice(0, max);
  const rest = defs.length - shown.length;
  if (defs.length === 0) {
    return (
      <button
        type="button"
        onClick={onEdit}
        className="text-xs text-slate-400 hover:text-slate-600 dark:hover:text-zinc-300"
        title="编辑标签"
      >
        + 标签
      </button>
    );
  }
  return (
    <button
      type="button"
      onClick={onEdit}
      className="flex max-w-[220px] flex-wrap items-center gap-1 text-left"
      title={defs.map((t) => t.name).join('、')}
    >
      {shown.map((t) => (
        <span
          key={t.id}
          className="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium text-white"
          style={{ background: t.color }}
        >
          {t.name}
        </span>
      ))}
      {rest > 0 && (
        <span className="rounded-full bg-slate-200 px-1.5 py-0.5 text-[10px] text-slate-600 dark:bg-zinc-700 dark:text-zinc-300">
          +{rest}
        </span>
      )}
    </button>
  );
}
