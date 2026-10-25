import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const version = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8')).version;
const release = path.join(root, 'release', `v${version}`);
const verifyRoot = path.join(release, `verification-${Date.now()}`);
fs.mkdirSync(verifyRoot);
const sourceCommit = JSON.parse(fs.readFileSync(path.join(release, `ema-desktop-${version}/build-info.json`), 'utf8')).SourceHead;
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', windowsHide: true, maxBuffer: 5 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`${path.basename(command)} failed: ${result.stderr || result.stdout || result.error?.message}`);
  return result.stdout.trim();
}
async function hash(file) {
  const digest = crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) digest.update(chunk);
  return digest.digest('hex');
}
function files(directory, prefix = '') {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const relative = prefix + entry.name;
    return entry.isDirectory() ? files(path.join(directory, entry.name), relative + '/') : [relative];
  }).sort();
}
function extract(file, destination) {
  run('C:/Program Files/7-Zip/7z.exe', ['x', '-y', '-bso0', '-bsp0', `-o${destination}`, file]);
}
const gitCommit = run('git', ['rev-parse', 'HEAD']);
if (gitCommit !== sourceCommit || run('git', ['status', '--porcelain=v1'])) throw new Error(`Source checkout is not the clean v${version} commit.`);
if (run('git', ['rev-parse', `v${version}^{}`]) !== sourceCommit) throw new Error('Source tag does not match.');
const portable = path.join(release, `ema-desktop-${version}`);
const portableFiles = files(portable);
if (portableFiles.some(file => /(^|\/)(\.env(?:\..*)?|storage\.json|identity\.json|settings\.json)$|\.key$|\.sqlite(?:-wal|-shm)?$|\.log$/i.test(file))) throw new Error('Unexpected local state in portable package.');
const zipExtract = path.join(verifyRoot, 'portable');
extract(path.join(release, `ema-desktop-${version}.zip`), zipExtract);
const extractedPortable = path.join(zipExtract, `ema-desktop-${version}`);
if (JSON.stringify(files(extractedPortable)) !== JSON.stringify(portableFiles)) throw new Error('ZIP file list differs from the prepared portable directory.');
const portableHashes = new Map();
for (const file of portableFiles) {
  const digest = await hash(path.join(portable, file));
  if (digest !== await hash(path.join(extractedPortable, file))) throw new Error(`ZIP content mismatch: ${file}`);
  portableHashes.set(file, digest);
}
const packageCheck = executable => JSON.parse(run('pwsh', ['-NoProfile', '-File', path.join(root, 'scripts/verify-release-package.ps1'), '-Executable', executable]));
const portableCheck = packageCheck(path.join(extractedPortable, 'traybuddy.exe'));
const installerBuildInfo = JSON.parse(fs.readFileSync(path.join(release, `ema-desktop-${version}-installer-build-info.json`), 'utf8'));
const packages = [];
for (const [filename, mode] of [[`traybuddy_${version}_x64-setup-user.exe`, 'currentUser']]) {
  const extracted = path.join(verifyRoot, mode);
  extract(path.join(release, filename), extracted);
  const extractedFiles = files(extracted);
  for (const file of portableFiles) {
    const fullPath = path.join(extracted, file);
    if (!fs.existsSync(fullPath)) throw new Error(`Missing installed resource: ${filename}/${file}`);
    const expected = file === 'build-info.json' ? await hash(path.join(release, `ema-desktop-${version}-installer-build-info.json`)) : file === 'traybuddy.exe' ? installerBuildInfo.BinarySha256.toLowerCase() : portableHashes.get(file);
    if (await hash(fullPath) !== expected) throw new Error(`Installed content mismatch: ${filename}/${file}`);
  }
  const extras = extractedFiles.filter(file => !portableFiles.includes(file) && !file.startsWith('$PLUGINSDIR/') && !/^uninstall\.exe$/i.test(file));
  if (extras.length) throw new Error(`Unexpected installed files in ${filename}: ${extras.join(', ')}`);
  const script = fs.readFileSync(path.join(root, 'src-tauri/target/release/nsis/x64/installer.nsi'), 'utf8');
  if (!script.includes(`!define INSTALLMODE "${mode}"`) || !script.includes(`!define VERSION "${version}"`) || !script.includes('!define BUNDLEID "com.traybuddy.cafel"')) throw new Error('Installer configuration mismatch.');
  packages.push({ filename, platform: 'windows-x86_64', installationMode: mode, bytes: fs.statSync(path.join(release, filename)).size, sha256: await hash(path.join(release, filename)), payloadBinarySha256: await hash(path.join(extracted, 'traybuddy.exe')), payloadFilesChecked: portableFiles.length, productionPackageCheck: packageCheck(path.join(extracted, 'traybuddy.exe')) });
}
packages.push({ filename: `ema-desktop-${version}.zip`, platform: 'windows-x86_64', installationMode: 'portable', bytes: fs.statSync(path.join(release, `ema-desktop-${version}.zip`)).size, sha256: await hash(path.join(release, `ema-desktop-${version}.zip`)), payloadBinarySha256: portableHashes.get('traybuddy.exe'), payloadFilesChecked: portableFiles.length, productionPackageCheck: portableCheck });
const originalBuildInfo = JSON.parse(fs.readFileSync(path.join(portable, 'build-info.json'), 'utf8'));
const manifest = { version: `${version}`, repository: 'ayasesakurashin-dotcom/TrayBuddy', tag: `v${version}`, sourceCommit, verifiedAt: new Date().toISOString(), sourceWorkingTreeClean: true, sourceBuildRecord: originalBuildInfo, installerBuildRecord: installerBuildInfo, binaryPackagingNote: 'Tauri stamps the NSIS bundle type into the installer payload binary; the portable binary remains unchanged.', packages };
const manifestFilename = `ema-desktop-${version}-release-manifest.json`;
fs.writeFileSync(path.join(release, manifestFilename), JSON.stringify(manifest, null, 2) + '\n');
const assetNames = [...packages.map(p => p.filename), `ema-desktop-${version}-release-notes.md`, manifestFilename];
const checksumLines = [];
for (const filename of assetNames) checksumLines.push(`${await hash(path.join(release, filename))}  ${filename}`);
fs.writeFileSync(path.join(release, `SHA256SUMS-${version}.txt`), checksumLines.join('\n') + '\n');
console.log(JSON.stringify({ verified: true, sourceCommit, filesPerPackage: portableFiles.length, packages: packages.map(p => ({ filename: p.filename, bytes: p.bytes, sha256: p.sha256, production: p.productionPackageCheck.ok })), assets: [...assetNames, `SHA256SUMS-${version}.txt`] }, null, 2));
