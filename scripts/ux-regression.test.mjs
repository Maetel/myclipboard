import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const app = read('src-tauri/src/lib.rs');
const clipboard = read('src-tauri/src/clipboard.rs');
const main = read('src/main.ts');
const popup = read('src/popup.ts');
const mainHtml = read('index.html');
const popupHtml = read('popup.html');
const cargo = read('src-tauri/Cargo.toml');

for (const command of [
  'load_settings', 'login', 'change_password', 'logout', 'save_preferences', 'sync_now',
]) {
  assert.match(app, new RegExp(`async fn ${command}\\(`), `${command} must not block the UI thread`);
}
for (const command of ['clipboard_history', 'clipboard_thumbnail', 'clipboard_select']) {
  assert.match(clipboard, new RegExp(`pub async fn ${command}\\(`), `${command} must be async`);
}
assert.match(app, /spawn_blocking\(task\)/);
assert.match(app, /Builder::default\(\)[\s\S]*?plugin\(tauri_plugin_single_instance::init/);
assert.match(app, /tauri_plugin_single_instance::init\(\|app, _, _\|[\s\S]*show_main_window\(app\)/);
const setupStart = app.indexOf('.setup(|app|');
const setupEnd = app.indexOf('\n        .build(', setupStart);
assert.ok(setupStart >= 0 && setupEnd > setupStart);
const setup = app.slice(setupStart, setupEnd);
assert.match(setup, /std::thread::spawn\(move \|\|[\s\S]*apply_shortcut/);
assert.doesNotMatch(setup, /if let Ok\(settings\) = read_settings\(app\.handle\(\)\)/);
const settingsStart = app.indexOf('fn load_settings_blocking(');
const settingsEnd = app.indexOf('\n#[tauri::command]\nasync fn load_settings', settingsStart);
assert.ok(settingsStart >= 0 && settingsEnd > settingsStart);
assert.doesNotMatch(app.slice(settingsStart, settingsEnd), /"\/me"|\.send\(\)/);

assert.match(mainHtml, /id="startupPanel"/);
assert.match(mainHtml, /id="loginPanel"[^>]*hidden/);
assert.match(mainHtml, /id="historyLimit"[^>]*min="1"[^>]*max="500"[^>]*value="200"/);
assert.match(main, /history_limit: number/);
assert.match(main, /historyLimit: input\('historyLimit'\)\.valueAsNumber/);
assert.match(app, /DEFAULT_HISTORY_LIMIT: usize = 200/);
assert.match(clipboard, /state\.items\.truncate\(settings\.history_limit\)/);
assert.match(main, /startupPanel\.hidden = true/);
assert.doesNotMatch(main, /보안 저장소 확인 창|Keychain|키체인/);
assert.match(app, /MACOS_SECRET_ROOT/);
assert.match(app, /Permissions::from_mode\(0o700\)/);
assert.match(app, /\.mode\(0o600\)/);
assert.match(app, /custom_flags\(libc::O_NOFOLLOW\)/);
assert.doesNotMatch(cargo, /apple-native/);
assert.match(cargo, /\[target\.'cfg\(windows\)'\.dependencies\][\s\S]*keyring/);
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
