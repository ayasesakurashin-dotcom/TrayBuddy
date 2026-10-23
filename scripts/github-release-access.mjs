import { spawnSync } from 'node:child_process';

const owner = 'ayasesakurashin-dotcom';
const result = spawnSync('git', ['credential', 'fill'], {
  input: `protocol=https\nhost=github.com\nusername=${owner}\n\n`,
  encoding: 'utf8',
  windowsHide: true,
  env: { ...process.env, GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'never' },
});
if (result.status !== 0) throw new Error('Existing GitHub login is unavailable.');
const entries = result.stdout.trim().split(/\r?\n/).map(line => {
  const index = line.indexOf('=');
  return [line.slice(0, index), line.slice(index + 1)];
});
const credentials = Object.fromEntries(entries);
if (!credentials.password) throw new Error('Existing GitHub credential is unavailable.');

export const repository = `${owner}/TrayBuddy`;
export async function github(endpoint, options = {}) {
  const response = await fetch(`https://api.github.com${endpoint}`, {
    ...options,
    headers: {
      Authorization: `Bearer ${credentials.password}`,
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'Ema-Desktop-Release',
      ...(options.body ? { 'Content-Type': 'application/json' } : {}),
      ...options.headers,
    },
    signal: AbortSignal.timeout(60000),
  });
  const data = response.status === 204 ? null : await response.json();
  if (!response.ok) throw new Error(`GitHub request failed (${response.status}): ${data.message}`);
  return data;
}

export function uploadHeaders() {
  return {
    Authorization: `Bearer ${credentials.password}`,
    Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28',
    'User-Agent': 'Ema-Desktop-Release',
  };
}

const account = await github('/user');
if (account.login !== owner) throw new Error('GitHub account does not match the repository owner.');
if (process.argv[1]?.endsWith('github-release-access.mjs')) {
  const repo = await github(`/repos/${repository}`);
  const releases = await github(`/repos/${repository}/releases?per_page=10`);
  console.log(JSON.stringify({ account: account.login, repository: repo.full_name, canPush: repo.permissions?.push, releases: releases.map(r => ({ id: r.id, tag: r.tag_name, draft: r.draft, url: r.html_url, assets: r.assets.map(a => ({ name: a.name, size: a.size })) })) }));
}
