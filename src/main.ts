import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';

interface PublicSettings {
  server_url: string; username: string; display_name: string; logged_in: boolean;
  enabled: boolean; shortcut: string; device_name: string;
}
interface ClipboardItem { id: string; kind: 'text'|'url'|'file'|'image'; text: string; created_at: string; size_bytes?: number }

const input = <T extends HTMLInputElement>(id: string) => document.getElementById(id) as T;
const element = (id: string) => document.getElementById(id) as HTMLElement;
const loginPanel = element('loginPanel');
const accountPanel = element('accountPanel');
const settingsPanel = element('settingsPanel');
const securityPanel = element('securityPanel');
const recentPanel = element('recentPanel');
const message = element('message');
const connection = element('connection');
let settings: PublicSettings;

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

async function refreshHistory() {
  const items = await invoke<ClipboardItem[]>('clipboard_history');
  element('historyCount').textContent = `${items.length}개`;
  const list = element('recentItems');
  list.replaceChildren();
  for (const item of items.slice(0, 8)) {
    const row = document.createElement('li');
    const text = document.createElement('span');
    text.className = 'item-text'; text.textContent = item.text;
    const meta = document.createElement('small');
    const label = ({ url: '링크', text: '텍스트', file: '파일', image: '이미지' } as const)[item.kind];
    meta.textContent = `${label} · ${relativeTime(item.created_at)}`;
    row.append(text, meta); list.append(row);
  }
  if (!items.length) list.innerHTML = '<li class="empty">아직 동기화된 기록이 없습니다.</li>';
}

async function load() {
  settings = await invoke<PublicSettings>('load_settings');
  input('serverUrl').value = settings.server_url || 'https://admin.memos.my';
  input('enabled').checked = settings.enabled;
  input('shortcut').value = settings.shortcut;
  loginPanel.hidden = settings.logged_in;
  accountPanel.hidden = !settings.logged_in;
  settingsPanel.hidden = !settings.logged_in;
  securityPanel.hidden = !settings.logged_in;
  recentPanel.hidden = !settings.logged_in;
  connection.textContent = settings.logged_in ? '동기화 준비됨' : '로그인 필요';
  connection.className = settings.logged_in ? 'connection ready' : 'connection';
  element('accountName').textContent = settings.display_name;
  element('accountUsername').textContent = settings.username ? `@${settings.username} · ${settings.device_name}` : '';
  if (settings.logged_in) await refreshHistory();
}

element('loginForm').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = element('loginButton') as HTMLButtonElement;
  button.disabled = true; setMessage('로그인하고 있습니다.');
  try {
    await invoke('login', { serverUrl: input('serverUrl').value.trim(), username: input('username').value.trim(), password: input('password').value });
    input('password').value = ''; setMessage('로그인했습니다.'); await load();
  } catch (error) { setMessage(String(error), true); }
  finally { button.disabled = false; }
});

element('settingsForm').addEventListener('submit', async (event) => {
  event.preventDefault(); setMessage('설정을 저장하고 있습니다.');
  try {
    await invoke('save_preferences', { enabled: input('enabled').checked, shortcut: input('shortcut').value.trim() });
    setMessage('설정을 저장했습니다.'); await load();
  } catch (error) { setMessage(String(error), true); }
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

element('resetShortcut').addEventListener('click', () => { input('shortcut').value = 'Ctrl+Shift+V'; });
element('syncNow').addEventListener('click', async () => {
  setMessage('동기화하고 있습니다.');
  try { await invoke('sync_now'); await refreshHistory(); setMessage('동기화했습니다.'); }
  catch (error) { setMessage(String(error), true); }
});
element('logoutButton').addEventListener('click', async () => {
  if (!window.confirm('이 기기에서 로그아웃할까요? 로컬 클립보드 기록도 삭제됩니다.')) return;
  try { await invoke('logout'); setMessage('로그아웃했습니다.'); await load(); }
  catch (error) { setMessage(String(error), true); }
});

void listen<string>('sync-status', (event) => {
  connection.textContent = event.payload;
  connection.className = event.payload === '동기화됨' ? 'connection ready' : 'connection';
});
void listen('clipboard-updated', () => void refreshHistory());
void load().catch((error) => setMessage(String(error), true));
