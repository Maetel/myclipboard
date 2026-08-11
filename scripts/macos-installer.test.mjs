import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';

const scriptUrl = new URL('./install-macos.sh', import.meta.url);
const script = await readFile(scriptUrl, 'utf8');
const bootstrapUrl = new URL('../install.sh', import.meta.url);
const bootstrap = await readFile(bootstrapUrl, 'utf8');
const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8');

const syntax = spawnSync('bash', ['-n', scriptUrl.pathname], { encoding: 'utf8' });
assert.equal(syntax.status, 0, syntax.stderr);
const bootstrapSyntax = spawnSync('bash', ['-n', bootstrapUrl.pathname], { encoding: 'utf8' });
assert.equal(bootstrapSyntax.status, 0, bootstrapSyntax.stderr);
assert.match(script, /\[\[ "\$\(uname -s\)" == Darwin \]\]/);
assert.match(script, /install_root="\$HOME\/Applications"/);
assert.match(script, /bundle_id.*my\.memos\.clipboard/s);
assert.match(script, /\/usr\/bin\/ditto/);
assert.match(script, /previous_app/);
assert.match(script, /--app/);
assert.match(script, /--no-launch/);
assert.doesNotMatch(script, /\bsudo\b/);
assert.match(bootstrap, /https:\/\/codeload\.github\.com\/Maetel\/myclipboard\/tar\.gz\/refs\/heads\/main/);
assert.match(bootstrap, /--proto '=https'/);
assert.match(bootstrap, /--tlsv1\.2/);
assert.match(bootstrap, /--strip-components=1/);
assert.match(bootstrap, /bash "\$installer" "\$@"/);
assert.doesNotMatch(bootstrap, /\bsudo\b/);
assert.equal(packageJson.scripts['desktop:install:macos'], 'bash scripts/install-macos.sh');
assert.equal(packageJson.scripts['test:macos-installer'], 'node scripts/macos-installer.test.mjs');
assert.match(readme, /npm run desktop:install:macos/);
assert.match(readme, /raw\.githubusercontent\.com\/Maetel\/myclipboard\/main\/install\.sh/);
assert.match(readme, /~\/Applications/);

console.log('macOS CLI installer regression: ok');
