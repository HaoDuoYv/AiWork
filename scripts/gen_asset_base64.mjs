#!/usr/bin/env node
/** 把图片转为 base64 data URI 的 TS 模块，用于小体积关键资产（如赞赏码）内嵌。
 *
 * 用法:
 *   node scripts/gen_asset_base64.mjs <输入图片> <输出.ts> [mime]
 *
 * 示例:
 *   node scripts/gen_asset_base64.mjs src/assets/donate-qr.jpg src/assets/donate-qr.base64.ts
 *
 * mime 缺省按扩展名推断（jpg->image/jpeg, png->image/png, svg->image/svg+xml, webp->image/webp）。
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { basename, extname } from 'node:path';

const MIME_BY_EXT = {
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
};

const [, , src, out, mimeArg] = process.argv;
if (!src || !out) {
  console.log(`用法: node scripts/gen_asset_base64.mjs <输入图片> <输出.ts> [mime]`);
  process.exit(1);
}
const ext = extname(src).toLowerCase();
const mime = mimeArg || MIME_BY_EXT[ext];
if (!mime) {
  console.error(`无法识别扩展名 ${ext}，请显式传入 mime 类型`);
  process.exit(1);
}
const data = readFileSync(src).toString('base64');
const varName = basename(out, extname(out)).replace(/-/g, '_').replace(/\./g, '_');
const content =
  `// 由 scripts/gen_asset_base64.mjs 生成：base64 内嵌，dev/生产均不依赖静态服务器。\n` +
  `// 重新生成: node scripts/gen_asset_base64.mjs ${src} ${out}\n` +
  `export const ${varName} = 'data:${mime};base64,${data}';\n`;
writeFileSync(out, content);
console.log(`written ${out} (${content.length} chars)`);
