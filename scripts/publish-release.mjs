import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { github, repository, uploadHeaders } from './github-release-access.mjs';

const root = path.resolve(import.meta.dirname, '../release');
const version = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../package.json'), 'utf8')).version;
const base = `/repos/${repository}`;
const tag = `v${version}`;
const expectedCommit = JSON.parse(fs.readFileSync(path.join(root, `ema-desktop-${version}/build-info.json`), 'utf8')).SourceHead;
const title = `Ema Desktop / TrayBuddy ${version}`;
const recordPath = path.join(root, `github-release-${version}-publish-record.json`);
const noteName = `ema-desktop-${version}-release-notes.md`;
const manifestName = `ema-desktop-${version}-release-manifest.json`;
const manifest = JSON.parse(fs.readFileSync(path.join(root, manifestName), 'utf8'));
if (manifest.sourceCommit !== expectedCommit || !manifest.sourceWorkingTreeClean || manifest.packages.length !== 2 || manifest.packages.some(p => !p.productionPackageCheck.ok)) throw new Error('Release verification is incomplete.');
const assetNames = [...manifest.packages.map(p => p.filename), noteName, manifestName, `SHA256SUMS-${version}.txt`];
const body = fs.readFileSync(path.join(root, noteName), 'utf8');
async function hash(file) {
  const digest = crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) digest.update(chunk);
  return digest.digest('hex');
}
const expectedAssets = [];
for (const name of assetNames) expectedAssets.push({ name, size: fs.statSync(path.join(root, name)).size, digest: `sha256:${await hash(path.join(root, name))}` });
const checksums = new Map(fs.readFileSync(path.join(root, `SHA256SUMS-${version}.txt`), 'utf8').trim().split('\n').map(line => { const [digest, name] = line.split('  '); return [name, `sha256:${digest}`]; }));
for (const asset of expectedAssets.filter(a => a.name !== `SHA256SUMS-${version}.txt`)) if (checksums.get(asset.name) !== asset.digest) throw new Error(`Local checksum mismatch: ${asset.name}`);
const repo = await github(base);
if (repo.full_name !== repository || repo.owner.login !== repository.split('/')[0] || !repo.permissions?.push) throw new Error('Repository authorization mismatch.');
let tagObject = (await github(`${base}/git/ref/tags/${tag}`)).object;
while (tagObject.type === 'tag') tagObject = (await github(`${base}/git/tags/${tagObject.sha}`)).object;
if (tagObject.type !== 'commit' || tagObject.sha !== expectedCommit) throw new Error('Remote tag points to different source.');
let record = fs.existsSync(recordPath) ? JSON.parse(fs.readFileSync(recordPath, 'utf8')) : null;
const save = () => fs.writeFileSync(recordPath, JSON.stringify(record, null, 2) + '\n');
const releases = await github(`${base}/releases?per_page=100`);
let release = releases.find(r => r.tag_name === tag);
const mode = process.argv[2] || 'upload';
if (!release) {
  if (record || mode !== 'upload') throw new Error('Expected release is missing.');
  release = await github(`${base}/releases`, { method: 'POST', body: JSON.stringify({ tag_name: tag, target_commitish: expectedCommit, name: title, body, draft: true, prerelease: false }) });
  record = { repository, tag, sourceCommit: expectedCommit, releaseId: release.id, createdByThisTask: true, createdAt: new Date().toISOString(), url: release.html_url, draft: true, assets: [] };
  save();
  console.log(`Created draft release ${release.id}`);
}
if (!record?.createdByThisTask || record.releaseId !== release.id || record.repository !== repository) throw new Error('Release already exists without this task\'s upload record; review it before modifying.');
if (release.name !== title || release.body !== body || release.prerelease) throw new Error('Unexpected release metadata.');

async function verifyAssets() {
  const remote = await github(`${base}/releases/${release.id}`);
  if (remote.assets.length !== expectedAssets.length) throw new Error('Remote attachment count differs from the verified release.');
  for (const expected of expectedAssets) {
    const asset = remote.assets.find(a => a.name === expected.name);
    if (!asset || asset.state !== 'uploaded' || asset.size !== expected.size || asset.digest?.toLowerCase() !== expected.digest) throw new Error(`Remote attachment verification failed: ${expected.name}`);
  }
  record.assets = remote.assets.map(a => ({ id: a.id, name: a.name, size: a.size, digest: a.digest, url: a.browser_download_url }));
  record.assetsVerifiedAt = new Date().toISOString();
  record.url = remote.html_url;
  record.draft = remote.draft;
  save();
  return remote;
}

if (mode === 'upload') {
  if (!release.draft) throw new Error('Release is already published.');
  const uploadUrl = new URL(release.upload_url.split('{')[0]);
  if (uploadUrl.protocol !== 'https:' || uploadUrl.hostname !== 'uploads.github.com' || uploadUrl.pathname !== `/repos/${repository}/releases/${release.id}/assets`) throw new Error('Unexpected release upload endpoint.');
  for (const expected of expectedAssets) {
    release = await github(`${base}/releases/${release.id}`);
    let existing = release.assets.find(a => a.name === expected.name);
    if (existing) {
      if (existing.state === 'uploaded' && existing.size === expected.size && existing.digest?.toLowerCase() === expected.digest) { console.log(`Verified existing ${expected.name}`); continue; }
      if (existing.state === 'starter') {
        await github(`${base}/releases/assets/${existing.id}`, { method: 'DELETE' });
      } else throw new Error(`Conflicting remote asset: ${expected.name}`);
    }
    const url = new URL(uploadUrl);
    url.searchParams.set('name', expected.name);
    const contentType = expected.name.endsWith('.exe') ? 'application/vnd.microsoft.portable-executable' : expected.name.endsWith('.zip') ? 'application/zip' : expected.name.endsWith('.json') ? 'application/json' : 'text/plain; charset=utf-8';
    console.log(`Uploading ${expected.name} (${expected.size} bytes)`);
    const response = await fetch(url, { method: 'POST', headers: { ...uploadHeaders(), 'Content-Type': contentType, 'Content-Length': String(expected.size) }, body: fs.createReadStream(path.join(root, expected.name)), duplex: 'half', redirect: 'error', signal: AbortSignal.timeout(900000) });
    const uploaded = await response.json();
    if (!response.ok) throw new Error(`Upload failed (${response.status}): ${uploaded.message}`);
    if (uploaded.state !== 'uploaded' || uploaded.size !== expected.size || uploaded.digest?.toLowerCase() !== expected.digest) throw new Error(`Upload digest mismatch: ${expected.name}`);
    record.assets.push({ id: uploaded.id, name: uploaded.name, size: uploaded.size, digest: uploaded.digest, url: uploaded.browser_download_url });
    save();
    console.log(`Uploaded and verified ${expected.name}`);
  }
  await verifyAssets();
  console.log(JSON.stringify({ draft: true, id: release.id, assetsVerified: record.assets.length, url: record.url }));
} else if (mode === 'publish') {
  await verifyAssets();
  if (release.draft) await github(`${base}/releases/${release.id}`, { method: 'PATCH', body: JSON.stringify({ draft: false, make_latest: 'true' }) });
  const published = await verifyAssets();
  if (published.draft || !published.published_at) throw new Error('Publication did not complete.');
  record.publishedAt = published.published_at;
  save();
  console.log(JSON.stringify({ published: true, url: published.html_url, tag: published.tag_name, assets: record.assets }));
} else throw new Error('Use upload or publish.');
