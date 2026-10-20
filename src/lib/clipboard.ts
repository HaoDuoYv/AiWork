/**
 * 剪贴板写入工具：Clipboard API 带 1.5s 超时，失败回退 execCommand。
 * WebView2 下 navigator.clipboard.writeText 可能被拒绝或 promise 永不落定
 * （表现为点击复制无任何反应），因此超时/异常均走兜底，保证结果可判定。
 */

/** 复制文本到剪贴板，返回是否成功（不抛异常） */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await Promise.race([
        navigator.clipboard.writeText(text),
        new Promise((_, reject) => setTimeout(() => reject(new Error('clipboard timeout')), 1500)),
      ]);
      return true;
    }
  } catch {
    /* 超时或被拒绝，走 execCommand 兜底 */
  }
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
