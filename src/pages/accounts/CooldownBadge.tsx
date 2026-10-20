import { Snowflake } from 'lucide-react';
import { Badge } from '../../components/ui';

const COOLDOWN_LABELS: Record<string, string> = {
  PlanLimit: '套餐限额',
  SoftRate: '限流',
  SessionDead: '登录已失效',
  NotFound: '接口异常',
  Server: '服务端错误',
  Client: '客户端错误',
  BusinessError: '业务错误',
  DeviceSigned: '设备已签',
};

/** SessionDead（JWT 被服务端吊销）的恢复指引：重登 → 保存登录态 → 续期捕获新凭证 */
const SESSION_DEAD_GUIDE =
  '该账号的登录已失效（JWT 被服务端吊销，常见于账号在别处重新登录或触发风控）。' +
  '恢复步骤：在 TRAE 中重新登录该账号 → 点击「保存当前登录态」保留新登录 → ' +
  '开启代理后点击右侧「续期 JWT」重新捕获凭证（新凭证捕获成功会自动解除失效标记），签到即可恢复。';

export function CooldownBadge({ type, until }: { type: string; until: number | null }) {
  const label = COOLDOWN_LABELS[type] ?? type;
  const isPermanent = type === 'SessionDead';
  let remaining = '';
  if (!isPermanent && until) {
    const secs = until - Math.floor(Date.now() / 1000);
    if (secs > 0) {
      const h = Math.floor(secs / 3600);
      const m = Math.floor((secs % 3600) / 60);
      remaining = h > 0 ? `${h}h${m}m` : `${m}m`;
    }
  }
  return (
    <Badge tone={isPermanent ? 'red' : 'amber'} title={isPermanent ? SESSION_DEAD_GUIDE : undefined}>
      <Snowflake size={12} /> {label}{remaining && ` ${remaining}`}
    </Badge>
  );
}
