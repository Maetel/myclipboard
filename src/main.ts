import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';

interface PublicSettings {
  server_url: string; username: string; display_name: string; logged_in: boolean;
  enabled: boolean; shortcut: string; device_name: string;
  account_migration_required: boolean;
}
interface ClipboardItem { id: string; kind: 'text'|'url'|'file'|'image'; text: string; filename?: string; created_at: string; size_bytes?: number }

const input = <T extends HTMLInputElement>(id: string) => document.getElementById(id) as T;
const element = (id: string) => document.getElementById(id) as HTMLElement;
const loginPanel = element('loginPanel');
const startupPanel = element('startupPanel');
const appShell = element('appShell');
const accountPanel = element('accountPanel');
const settingsPanel = element('settingsPanel');
const securityPanel = element('securityPanel');
const recentPanel = element('recentPanel');
const message = element('message');
const connection = element('connection');
let settings: PublicSettings;
let loadGeneration = 0;
let historyLoading = false;
let historyRefreshQueued = false;
let historySignature = '';
const defaultShortcut = navigator.userAgent.includes('Mac OS') ? 'Command+Shift+V' : 'Ctrl+Shift+V';

function setMessage(value: string, error = false) {
  message.textContent = value;
  message.className = error ? 'message error' : 'message';
}

function relativeTime(value: string): string {
  const seconds = Math.max(0, Math.floor((Date.now() - Date.parse(value)) / 1000));
  if (seconds < 60) return '방금 전';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}분 전`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}시간 전`;
  return `${Math.floor(seconds / 86400)}일 전`;
}

function kindIcon(kind: ClipboardItem['kind']) {
  const icon = document.createElement('span');
  icon.className = `kind-icon kind-${kind}`;
  icon.ariaHidden = 'true';
  icon.textContent = kind === 'file' ? '📄' : kind === 'image' ? '▧' : kind === 'url' ? '↗' : 'T';
  return icon;
}

async function refreshHistory() {
  if (historyLoading) {
    historyRefreshQueued = true;
    return;
  }
  historyLoading = true;
  recentPanel.setAttribute('aria-busy', 'true');
  try {
    const items = await invoke<ClipboardItem[]>('clipboard_history');
    const signature = JSON.stringify(items.slice(0, 8).map((item) => [item.id, item.kind, item.text, item.filename, item.created_at]));
    element('historyCount').textContent = `${items.length}개`;
    if (signature === historySignature) return;
    historySignature = signature;
    const list = element('recentItems');
    list.replaceChildren();
    for (const item of items.slice(0, 8)) {
      const row = document.createElement('li');
      const content = document.createElement('div');
      content.className = 'recent-content';
      const text = document.createElement('span');
      text.className = 'item-text'; text.textContent = item.filename ?? item.text;
      const meta = document.createElement('small');
      const label = ({ url: '링크', text: '텍스트', file: '파일', image: '이미지' } as const)[item.kind];
      meta.textContent = `${label} · ${relativeTime(item.created_at)}`;
      content.append(text, meta);
      row.append(kindIcon(item.kind), content); list.append(row);
    }
    if (!items.length) list.innerHTML = '<li class="empty">아직 동기화된 기록이 없습니다.</li>';
  } catch (error) {
    setMessage(`최근 기록을 불러오지 못했습니다. ${String(error)}`, true);
  } finally {
    historyLoading = false;
    recentPanel.removeAttribute('aria-busy');
    if (historyRefreshQueued) {
      historyRefreshQueued = false;
      void refreshHistory();
    }
  }
}

async function load(showStartup = false) {
  const generation = ++loadGeneration;
  if (showStartup) {
    startupPanel.hidden = false;
    startupPanel.querySelector('.spinner')?.removeAttribute('hidden');
    (startupPanel.querySelector('h2') as HTMLElement).textContent = '저장된 계정을 확인하는 중…';
    (startupPanel.querySelector('p') as HTMLElement).textContent = '확인이 끝나면 바로 클립보드 동기화를 시작합니다.';
    (element('startupRetry') as HTMLButtonElement).hidden = true;
    appShell.setAttribute('aria-busy', 'true');
  }
  try {
    const loaded = await invoke<PublicSettings>('load_settings');
    if (generation !== loadGeneration) return;
    settings = loaded;
  } catch (error) {
    if (generation !== loadGeneration) return;
    startupPanel.hidden = false;
    startupPanel.querySelector('.spinner')?.setAttribute('hidden', '');
    (startupPanel.querySelector('h2') as HTMLElement).textContent = '계정 정보를 확인하지 못했습니다.';
    (startupPanel.querySelector('p') as HTMLElement).textContent = String(error);
    (element('startupRetry') as HTMLButtonElement).hidden = false;
    appShell.removeAttribute('aria-busy');
    return;
  }
  input('enabled').checked = settings.enabled;
  input('shortcut').value = settings.shortcut;
  element('shortcutHint').textContent = settings.shortcut;
  loginPanel.hidden = settings.logged_in;
  accountPanel.hidden = !settings.logged_in;
  settingsPanel.hidden = !settings.logged_in;
  securityPanel.hidden = !settings.logged_in;
  recentPanel.hidden = !settings.logged_in;
  connection.textContent = settings.logged_in ? '동기화 준비됨' : '로그인 필요';
  connection.className = settings.logged_in ? 'connection ready' : 'connection';
  element('accountName').textContent = settings.display_name;
  element('accountUsername').textContent = settings.username ? `@${settings.username} · ${settings.device_name}` : '';
  startupPanel.hidden = true;
  appShell.removeAttribute('aria-busy');
  if (settings.logged_in) void refreshHistory();
  if (settings.account_migration_required) {
    setMessage('기존 복사 기록은 마이메모로 옮겼습니다. 같은 마이메모 계정으로 다시 로그인해 주세요.');
  }
}

element('loginForm').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = element('loginButton') as HTMLButtonElement;
  button.disabled = true; setMessage('로그인하고 있습니다.');
  try {
    await invoke('login', { serverUrl: 'https://memos.my', username: input('username').value.trim(), password: input('password').value });
    input('password').value = ''; setMessage('로그인했습니다.'); await load();
  } catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});

element('settingsForm').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = element('settingsButton') as HTMLButtonElement;
  button.disabled = true; setMessage('설정을 저장하고 있습니다.');
  try {
    await invoke('save_preferences', { enabled: input('enabled').checked, shortcut: input('shortcut').value.trim() });
    setMessage('설정을 저장했습니다.'); await load();
  } catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});

element('passwordForm').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = element('passwordButton') as HTMLButtonElement;
  const newPassword = input('newPassword').value;
  if (newPassword !== input('newPasswordConfirm').value) {
    setMessage('새 비밀번호 확인이 일치하지 않습니다.', true);
    return;
  }
  button.disabled = true; setMessage('비밀번호를 변경하고 있습니다.');
  try {
    await invoke('change_password', { currentPassword: input('currentPassword').value, newPassword });
    input('currentPassword').value = '';
    input('newPassword').value = '';
    input('newPasswordConfirm').value = '';
    setMessage('비밀번호를 변경했습니다. 다른 기기에서는 다시 로그인해 주세요.');
  } catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});

element('resetShortcut').addEventListener('click', () => { input('shortcut').value = defaultShortcut; });
element('syncNow').addEventListener('click', async () => {
  const button = element('syncNow') as HTMLButtonElement;
  button.disabled = true; setMessage('동기화하고 있습니다.');
  try { await invoke('sync_now'); await refreshHistory(); setMessage('동기화했습니다.'); }
  catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});
element('logoutButton').addEventListener('click', async () => {
  if (!window.confirm('이 기기에서 로그아웃할까요? 로컬 클립보드 기록도 삭제됩니다.')) return;
  const button = element('logoutButton') as HTMLButtonElement;
  button.disabled = true;
  try { await invoke('logout'); historySignature = ''; setMessage('로그아웃했습니다.'); await load(); }
  catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});

element('startupRetry').addEventListener('click', () => void load(true));

void listen<string>('sync-status', (event) => {
  connection.textContent = event.payload;
  connection.className = event.payload === '동기화됨' ? 'connection ready' : 'connection';
});
void listen('auth-required', () => {
  setMessage('로그인이 만료되었습니다. 다시 로그인해 주세요.', true);
  void load();
});
void listen('clipboard-updated', () => void refreshHistory());
void load(true);
