import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';

function fail(message) {
  console.error(message);
  process.exit(1);
}

function run(command, args, options = {}) {
  console.log(`\n> ${path.basename(command)} ${args.join(' ')}`);
  const result = spawnSync(command, args, {
    cwd: repo,
    env,
    stdio: 'inherit',
    windowsHide: true,
    ...options,
  });
  if (result.error) fail(result.error.message);
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.platform !== 'win32') {
  fail('이 실행기는 Windows용 Node.js로 실행해야 합니다.');
}

const args = process.argv.slice(2);
const repoIndex = args.indexOf('--repo');
const repo = repoIndex >= 0 ? args[repoIndex + 1] : '';
const shouldInstall = args.includes('--install');

if (!repo || !existsSync(path.join(repo, 'package.json'))) {
  fail('Windows 빌드용 저장소 경로가 올바르지 않습니다.');
}

const nodeDirectory = path.dirname(process.execPath);
const npmCli = path.join(nodeDirectory, 'node_modules', 'npm', 'bin', 'npm-cli.js');
const cargo = path.join(process.env.USERPROFILE ?? '', '.cargo', 'bin', 'cargo.exe');
const systemRoot = process.env.SystemRoot ?? 'C:\\Windows';
const env = {
  ...process.env,
  PATH: [path.dirname(cargo), nodeDirectory, path.join(systemRoot, 'System32'), systemRoot].join(';'),
};

if (!existsSync(npmCli)) fail(`npm 실행 파일을 찾지 못했습니다: ${npmCli}`);
if (!existsSync(cargo)) fail(`Windows용 Cargo를 찾지 못했습니다: ${cargo}`);

run(process.execPath, [npmCli, 'ci']);
run(process.execPath, [npmCli, 'run', 'typecheck']);
run(process.execPath, [npmCli, 'run', 'build']);
run(process.execPath, [npmCli, 'run', 'test:windows-idle']);
run(cargo, ['test', '--manifest-path', path.join(repo, 'src-tauri', 'Cargo.toml')]);

const tauriCli = path.join(repo, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
run(process.execPath, [tauriCli, 'build', '--config', path.join('src-tauri', 'tauri.windows.conf.json')]);

const packageJson = JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8'));
const tauriConfig = JSON.parse(readFileSync(path.join(repo, 'src-tauri', 'tauri.conf.json'), 'utf8'));
const installerDirectory = path.join(repo, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
const installers = existsSync(installerDirectory)
  ? readdirSync(installerDirectory)
      .filter((name) => name.includes(`_${packageJson.version}_`) && name.endsWith('-setup.exe'))
      .map((name) => path.join(installerDirectory, name))
  : [];

if (installers.length !== 1) {
  fail(`NSIS 설치 파일을 하나로 확정하지 못했습니다: ${installerDirectory}`);
}

const installer = installers[0];
console.log(`\nWindows 설치 파일을 만들었습니다:\n${installer}`);

if (!shouldInstall) process.exit(0);

const taskkill = path.join(systemRoot, 'System32', 'taskkill.exe');
spawnSync(taskkill, ['/IM', 'mymemo-clipboard.exe', '/T', '/F'], {
  env,
  stdio: 'ignore',
  windowsHide: true,
});
run(installer, ['/S']);

const installedApp = path.join(
  process.env.LOCALAPPDATA ?? '',
  tauriConfig.productName ?? 'MyMemo Clipboard',
  'mymemo-clipboard.exe',
);
if (!existsSync(installedApp)) fail(`설치된 앱을 찾지 못했습니다: ${installedApp}`);

const child = spawn(installedApp, [], {
  detached: true,
  env,
  stdio: 'ignore',
  windowsHide: true,
});
child.unref();
console.log(`설치 후 앱을 실행했습니다:\n${installedApp}`);
