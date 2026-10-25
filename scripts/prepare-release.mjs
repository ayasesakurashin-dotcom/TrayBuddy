// Run after a clean source commit, pnpm build and cargo build --release --features custom-protocol.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const version = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8')).version;
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version.');
const release = path.join(root, 'release', `v${version}`);
const portableName = `ema-desktop-${version}`;
const portable = path.join(release, portableName);
const sevenZip = process.env.SEVEN_ZIP || 'C:/Program Files/7-Zip/7z.exe';
function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', windowsHide: true, maxBuffer: 8 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`${path.basename(command)} failed: ${result.stderr || result.stdout || result.error?.message}`);
  return result.stdout.trim();
}
const sourceCommit = run('git', ['rev-parse', 'HEAD']);
if (run('git', ['status', '--porcelain=v1'])) throw new Error('Commit the release sources first.');
if (run('git', ['rev-parse', `v${version}^{}`]) !== sourceCommit) throw new Error('Release tag differs from HEAD.');
if (fs.existsSync(portable)) throw new Error('Prepared directory already exists; preserve or archive it before rebuilding.');
fs.mkdirSync(portable, { recursive: true });
function copy(source, destination) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(source, destination);
}
const resourceDirs = ['config', 'i18n', 'mod-tool', 'mods', 'other-tool', 'static', 'tools-common'];
// Copy only versioned assets, never arbitrary working-directory or user state.
const tracked = run('git', ['ls-files', '-z', ...resourceDirs, 'src-tauri/icons', 'mods_release']).split('\0').filter(Boolean);
const modStaging = path.join(release, `mod-sources-${version}`);
if (fs.existsSync(modStaging)) throw new Error('Mod staging already exists.');
for (const file of tracked) {
  if (file.startsWith('mods_release/')) {
    copy(path.join(root, file), path.join(modStaging, file.slice('mods_release/'.length)));
  } else {
    const destination = file.startsWith('src-tauri/icons/') ? file.slice('src-tauri/'.length) : file;
    copy(path.join(root, file), path.join(portable, destination));
  }
}
for (const entry of fs.readdirSync(modStaging, { withFileTypes: true })) {
  if (!entry.isDirectory() || !fs.existsSync(path.join(modStaging, entry.name, 'manifest.json'))) continue;
  run(sevenZip, ['a', '-tzip', '-mx=5', '-bso0', '-bsp0', path.join(portable, 'mods', `${entry.name}.tbuddy`), entry.name], modStaging);
}
for (const file of ['companion-0.4.0.md', 'companion-flexible-care-2026-10-23.md', 'companion-role-decoupling-2026-10-25.md', `releases/${version}.md`]) {
  copy(path.join(root, 'docs', file), path.join(portable, 'docs', file));
}
copy(path.join(root, 'README-EMA-AI.md'), path.join(portable, '使用说明.md'));
copy(path.join(root, 'CHANGELOG.md'), path.join(portable, 'CHANGELOG.md'));
copy(path.join(root, `docs/releases/${version}.md`), path.join(release, `${portableName}-release-notes.md`));
copy(path.join(root, 'src-tauri/target/release/traybuddy.exe'), path.join(portable, 'traybuddy.exe'));
const check = JSON.parse(run('pwsh', ['-NoProfile', '-File', path.join(root, 'scripts/verify-release-package.ps1'), '-Executable', path.join(portable, 'traybuddy.exe')]));
if (!check.ok || !check.production_mode) throw new Error('Production executable check failed.');
fs.writeFileSync(path.join(portable, 'package-check.json'), JSON.stringify(check, null, 2) + '\n');
const binaryHash = crypto.createHash('sha256').update(fs.readFileSync(path.join(portable, 'traybuddy.exe'))).digest('hex');
const record = { Version: version, BuiltAt: new Date().toISOString(), BinarySha256: binaryHash, ProductionMode: true, ApplicationIdentifier: 'com.traybuddy.cafel', SourceHead: sourceCommit, SourceWorkingTreeChanges: false, BuildCommand: process.env.RELEASE_BUILD_COMMAND || 'cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml --offline', FrontendCommand: 'pnpm build', UserData: 'not included' };
fs.writeFileSync(path.join(portable, 'build-info.json'), JSON.stringify(record, null, 2) + '\n');
run(sevenZip, ['a', '-tzip', '-mx=5', '-bso0', '-bsp0', path.join(release, `${portableName}.zip`), portableName], release);

const resources = Object.fromEntries([...resourceDirs, 'icons', 'docs'].map(directory => [`../release/v${version}/${portableName}/${directory}`, directory]));
for (const file of ['使用说明.md', 'CHANGELOG.md', 'package-check.json']) resources[`../release/v${version}/${portableName}/${file}`] = file;
resources[`../release/v${version}/${portableName}-installer-build-info.json`] = 'build-info.json';
// bundle-release.ps1 replaces the default map; CLI overlays would merge it.
fs.writeFileSync(path.join(release, `tauri.${version}.current-user.json`), JSON.stringify({ bundle: { resources, windows: { nsis: { installMode: 'currentUser' } } } }, null, 2) + '\n');
console.log(JSON.stringify({ version, sourceCommit, portable, production: check.ok, binaryHash }));
