import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const app = read('src-tauri/src/lib.rs');
const clipboard = read('src-tauri/src/clipboard.rs');
const main = read('src/main.ts');
const popup = read('src/popup.ts');
const mainHtml = read('index.html');
const popupHtml = read('popup.html');

for (const command of [
  'load_settings', 'login', 'change_password', 'logout', 'save_preferences', 'sync_now',
]) {
  assert.match(app, new RegExp(`async fn ${command}\\(`), `${command} must not block the UI thread`);
}
for (const command of ['clipboard_history', 'clipboard_thumbnail', 'clipboard_select']) {
  assert.match(clipboard, new RegExp(`pub async fn ${command}\\(`), `${command} must be async`);
}
assert.match(app, /spawn_blocking\(task\)/);
const settingsStart = app.indexOf('fn load_settings_blocking(');
const settingsEnd = app.indexOf('\n#[tauri::command]\nasync fn load_settings', settingsStart);
assert.ok(settingsStart >= 0 && settingsEnd > settingsStart);
assert.doesNotMatch(app.slice(settingsStart, settingsEnd), /"\/me"|\.send\(\)/);

assert.match(mainHtml, /id="startupPanel"/);
assert.match(mainHtml, /id="loginPanel"[^>]*hidden/);
assert.match(main, /startupPanel\.hidden = true/);
assert.match(main, /kind === 'file' \? '📄'/);
assert.match(popup, /item\.kind === 'file' \? '📄'/);

assert.match(popup, /new IntersectionObserver/);
assert.match(popup, /thumbnailObserver\.observe/);
assert.match(popup, /if \(selecting \|\| refreshRunning\)/);
assert.match(popup, /nextSignature !== renderedSignature/);
assert.match(popup, /row\.addEventListener\('dblclick'/);
const clickStart = popup.indexOf("row.addEventListener('click'");
const doubleClickStart = popup.indexOf("row.addEventListener('dblclick'", clickStart);
assert.ok(clickStart >= 0 && doubleClickStart > clickStart);
assert.doesNotMatch(popup.slice(clickStart, doubleClickStart), /select\(/);
assert.match(popup, /await nextPaint\(\)/);
assert.match(popup, /Esc로 취소/);
assert.match(popupHtml, /Enter·더블클릭 붙여넣기/);
assert.match(popupHtml, /id="retryHistory"/);
assert.match(clipboard, /downloads_dir\(&app\)\?\.join\(hex\(&Sha256::digest\(item\.id\.as_bytes\(\)\)\)\)/);
assert.match(clipboard, /let path = directory\.join\(filename\)/);

console.log('clipboard UX regression: ok');
