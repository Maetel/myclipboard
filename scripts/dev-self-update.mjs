import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const intervalMs = Math.max(10_000, Number(process.env.MYMEMO_DEV_UPDATE_INTERVAL_MS) || 30_000);
const isWindows = process.platform === 'win32';
const npmCli = process.env.npm_execpath;
let child;
let stopping = false;

function run(command, args, options = {}) {
  return spawnSync(command, args, {
    cwd: repo,
    encoding: 'utf8',
    windowsHide: true,
    ...options,
  });
}

function git(args) {
  return run('git', args);
}

function npm(args, stdio = 'inherit') {
  if (npmCli) return run(process.execPath, [npmCli, ...args], { stdio });
  return run(isWindows ? 'npm.cmd' : 'npm', args, { stdio, shell: isWindows });
}

function line(result) {
  return result.status === 0 ? result.stdout.trim() : '';
}

function dependenciesChanged(before, after) {
  if (!before || !after || before === after) return false;
  const changed = git(['diff', '--name-only', before, after, '--', 'package.json', 'package-lock.json']);
  return changed.status === 0 && Boolean(changed.stdout.trim());
}

function installDependencies() {
  console.log('개발 의존성을 맞추고 있습니다…');
  const command = existsSync(path.join(repo, 'package-lock.json')) ? ['ci'] : ['install'];
  const result = npm(command);
  if (result.status !== 0) throw new Error('개발 의존성을 설치하지 못했습니다.');
}

function updateWorkingTree() {
  const branch = line(git(['branch', '--show-current']));
  if (branch !== 'main') {
    console.log(`main 브랜치가 아니어서 자동 업데이트를 건너뜁니다. 현재: ${branch || '분리된 checkout'}`);
    return { updated: false, dependencies: false };
  }
  const status = git(['status', '--porcelain', '--untracked-files=normal']);
  if (status.status !== 0 || status.stdout.trim()) {
    console.log('로컬 수정 사항이 있어 자동 업데이트를 건너뜁니다. 실행 중인 앱은 그대로 유지합니다.');
    return { updated: false, dependencies: false };
  }
  const before = line(git(['rev-parse', 'HEAD']));
  const pulled = git(['pull', '--ff-only', 'origin', 'main']);
  if (pulled.status !== 0) {
    console.log('최신 코드를 받지 못했습니다. 현재 코드로 계속 실행합니다.');
    return { updated: false, dependencies: false };
  }
  const after = line(git(['rev-parse', 'HEAD']));
  if (!before || !after || before === after) return { updated: false, dependencies: false };
  console.log(`새 코드를 받았습니다: ${before.slice(0, 8)} → ${after.slice(0, 8)}`);
  return { updated: true, dependencies: dependenciesChanged(before, after) };
}

function startApp() {
  console.log('MyMemo Clipboard 개발 앱을 시작합니다.');
  const command = npmCli ? process.execPath : isWindows ? 'npm.cmd' : 'npm';
  const args = npmCli ? [npmCli, 'run', 'desktop:dev'] : ['run', 'desktop:dev'];
  child = spawn(command, args, {
    cwd: repo,
    env: process.env,
    stdio: 'inherit',
    windowsHide: true,
    detached: !isWindows,
    shell: isWindows && !npmCli,
  });
  child.once('exit', (code, signal) => {
    if (!stopping && code !== 0) {
      console.log(`개발 앱이 종료됐습니다 (${signal ?? code}). 다음 업데이트 확인 뒤 다시 시작합니다.`);
    }
    child = undefined;
  });
}

async function stopApp() {
  const current = child;
  if (!current?.pid) return;
  if (isWindows) {
    run(path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'taskkill.exe'), [
      '/PID', String(current.pid), '/T', '/F',
    ], { stdio: 'ignore' });
  } else {
    try { process.kill(-current.pid, 'SIGTERM'); } catch {}
  }
  await new Promise((resolve) => {
    if (current.exitCode !== null) return resolve();
    const timer = setTimeout(resolve, 5_000);
    current.once('exit', () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function shutdown() {
  if (stopping) return;
  stopping = true;
  await stopApp();
  process.exit(0);
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

if (!existsSync(path.join(repo, 'node_modules'))) installDependencies();
const initialUpdate = updateWorkingTree();
if (initialUpdate.dependencies) installDependencies();
startApp();

while (!stopping) {
  await new Promise((resolve) => setTimeout(resolve, intervalMs));
  if (stopping) break;
  const update = updateWorkingTree();
  if (update.updated) {
    await stopApp();
    if (update.dependencies) installDependencies();
    startApp();
  } else if (!child) {
    startApp();
  }
}
