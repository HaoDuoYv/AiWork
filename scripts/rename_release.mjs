#!/usr/bin/env node
/** 打包产物统一命名：把 `npm run tauri build` 产出的安装包复制到 release/，
 * 统一使用中文产品名命名（APP 名称标识）。
 *
 * 用法：node scripts/rename_release.mjs [--strict]
 *   --strict  任一产物缺失时以非零码退出（默认仅告警）
 *   src-tauri/target/release/bundle/nsis/AI Work 助手_<ver>_x64-setup.exe
 *       → release/AI Work 助手_<ver>_x64-setup.exe
 *   src-tauri/target/release/bundle/msi/AI Work 助手_<ver>_x64_zh-CN.msi
 *       → release/AI Work 助手_<ver>_x64_zh-CN.msi
 *   portable zip 由 package_portable.mjs 直接生成同名（无需重命名）。
 */
import { createHash } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SRC_TAURI = resolve(ROOT, 'src-tauri');

// 发布校验清单文件名（作为 Release 资产上传，更新器下载安装包后校验完整性，
// updater.rs fail-closed：清单缺失/损坏/版本不符/未收录资产均阻止自动更新）
const MANIFEST_NAME = 'latest.json';

const die = (msg) => {
  console.error(msg);
  process.exit(1);
};

function readVersion(conf) {
  // 版本单源 = Cargo.toml；tauri.conf.json 里的 version 字段已移除（自动回读 Cargo.toml）
  if (conf.version) return conf.version;
  const m = readFileSync(resolve(SRC_TAURI, 'Cargo.toml'), 'utf8').match(
    /^version\s*=\s*"(\d+\.\d+\.\d+)"/m,
  );
  if (!m) die('ERROR: 无法从 Cargo.toml 读取版本号');
  return m[1];
}

const conf = JSON.parse(readFileSync(resolve(SRC_TAURI, 'tauri.conf.json'), 'utf8'));
const version = readVersion(conf);
const product = conf.productName;
const outDir = resolve(ROOT, 'release');
mkdirSync(outDir, { recursive: true });

const jobs = [
  [
    resolve(SRC_TAURI, 'target/release/bundle/nsis', `${product}_${version}_x64-setup.exe`),
    resolve(outDir, `${product}_${version}_x64-setup.exe`),
  ],
  [
    resolve(SRC_TAURI, 'target/release/bundle/msi', `${product}_${version}_x64_zh-CN.msi`),
    resolve(outDir, `${product}_${version}_x64_zh-CN.msi`),
  ],
];

let moved = 0;
const missing = [];
for (const [src, dst] of jobs) {
  if (existsSync(src)) {
    copyFileSync(src, dst);
    console.log('OK:', dst);
    moved++;
  } else {
    console.error('SKIP（不存在）:', src);
    missing.push(src);
  }
}
if (moved === 0) die('未找到任何安装包产物，请先执行 npm run tauri build');

// 生成发布校验清单 latest.json：版本号 + 各资产 SHA-256。
// 更新器（updater.rs）下载安装包后与清单比对，不匹配即拒绝安装（更新包完整性校验）。
const assets = {};
for (const name of [
  `${product}_${version}_x64-setup.exe`,
  `${product}_${version}_x64_zh-CN.msi`,
  `${product}_${version}_x64_portable.zip`,
]) {
  const p = join(outDir, name);
  if (!existsSync(p)) {
    // portable zip 未打包时警告但不阻塞（更新器只安装 setup/msi）
    if (name.endsWith('_portable.zip')) {
      console.error('WARN（清单跳过，文件不存在）:', name);
      continue;
    }
    die(`清单生成失败：产物缺失 ${name}`);
  }
  const h = createHash('sha256').update(readFileSync(p)).digest('hex');
  assets[name] = h;
  console.log(`SHA256 ${h}  ${name}`);
}
// 与原 Python 版输出字节对齐（updater.rs 消费契约）：2 空格缩进、中文原样、无末尾换行
writeFileSync(join(outDir, MANIFEST_NAME), JSON.stringify({ version, assets }, null, 2));
console.log('OK:', join(outDir, MANIFEST_NAME));
console.error('提示：发布时请将', MANIFEST_NAME, '作为 Release 资产一并上传');

if (missing.length) {
  console.error(`WARNING: 有 ${missing.length} 个产物缺失，上传 release 前请核对清单：`);
  for (const m of missing) console.error('  -', m);
  if (process.argv.includes('--strict')) {
    console.error('--strict：缺失产物视为失败');
    process.exit(1);
  }
}
