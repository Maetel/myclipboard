import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const runner = readFileSync(new URL('./dev-self-update.mjs', import.meta.url), 'utf8');
const packageJson = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
const macLauncher = readFileSync(new URL('../MyMemo Clipboard Dev.command', import.meta.url), 'utf8');
const windowsLauncher = readFileSync(new URL('../MyMemo Clipboard Dev.cmd', import.meta.url), 'utf8');
const viteConfig = readFileSync(new URL('../vite.config.ts', import.meta.url), 'utf8');
const gitAttributes = readFileSync(new URL('../.gitattributes', import.meta.url), 'utf8');

assert.equal(packageJson.scripts['dev:self-update'], 'node scripts/dev-self-update.mjs');
assert.match(macLauncher, /npm run dev:self-update/);
assert.match(windowsLauncher, /npm run dev:self-update/);
assert.match(runner, /30_000/);
assert.match(runner, /git\(\['pull', '--ff-only', 'origin', 'main'\]\)/);
assert.match(runner, /diff', '--quiet', '--ignore-submodules', '--'/);
assert.match(runner, /diff', '--cached', '--quiet', '--ignore-submodules', '--'/);
assert.match(runner, /ls-files', '--others', '--exclude-standard/);
assert.match(runner, /branch !== 'main'/);
assert.match(runner, /await stopApp\(\);[\s\S]*if \(update\.dependencies\) installDependencies\(\);[\s\S]*startApp\(\)/);
assert.match(runner, /taskkill\.exe/);
assert.match(runner, /process\.kill\(-current\.pid, 'SIGTERM'\)/);
assert.match(viteConfig, /ignored: \['\*\*\/src-tauri\/target\/\*\*'\]/);
assert.match(gitAttributes, /^\/src-tauri\/Cargo\.toml text eol=lf$/m);

console.log('self-updating development launcher regression: ok');
