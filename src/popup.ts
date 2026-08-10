import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './popup.css';

interface ClipboardItem {
  id: string;
  kind: 'text' | 'url' | 'file' | 'image';
  text: string;
  filename?: string;
  size_bytes?: number;
  thumbnail_available: boolean;
  created_at: string;
}

const search = document.getElementById('clipboardSearch') as HTMLInputElement;
const list = document.getElementById('clipboardItems') as HTMLOListElement;
const status = document.getElementById('clipboardStatus') as HTMLDivElement;
let items: ClipboardItem[] = [];
let selectedId = '';
let dismissing = false;
let selecting = false;
const thumbnails = new Map<string, Promise<string | null>>();

const filtered = () => {
  const query = search.value.trim().toLocaleLowerCase();
  return query
    ? items.filter((item) => (item.filename ?? item.text).toLocaleLowerCase().includes(query))
    : items;
};

function relative(value: string) {
  const seconds = Math.max(0, Math.floor((Date.now() - Date.parse(value)) / 1000));
  if (seconds < 60) return `${seconds}초`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}분`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}시간`;
  return `${Math.floor(seconds / 86400)}일`;
}

function size(value?: number) {
  if (!value) return '';
  return value >= 1024 * 1024
    ? `${(value / 1024 / 1024).toFixed(1)}MB`
    : `${Math.ceil(value / 1024)}KB`;
}

function optionId(id: string) {
  return `clip-option-${id}`;
}

function thumbnail(id: string) {
  let request = thumbnails.get(id);
  if (!request) {
    request = invoke<string | null>('clipboard_thumbnail', { id }).catch(() => null);
    thumbnails.set(id, request);
  }
  return request;
}

function updateSelection(scroll = false) {
  for (const row of list.querySelectorAll<HTMLElement>('li[data-id]')) {
    const selected = row.dataset.id === selectedId;
    row.classList.toggle('selected', selected);
    row.ariaSelected = String(selected);
  }
  if (selectedId) search.setAttribute('aria-activedescendant', optionId(selectedId));
  else search.removeAttribute('aria-activedescendant');
  if (scroll) list.querySelector('.selected')?.scrollIntoView({ block: 'nearest' });
}

function render() {
  const visible = filtered();
  if (!visible.some((item) => item.id === selectedId)) selectedId = visible[0]?.id ?? '';
  list.replaceChildren();
  for (const item of visible) {
    const row = document.createElement('li');
    row.id = optionId(item.id);
    row.dataset.id = item.id;
    row.role = 'option';
    const content = document.createElement('div');
    content.className = 'clip-content';
    const text = document.createElement('span');
    text.className = 'clip-text';
    text.textContent = item.filename ?? item.text;
    const meta = document.createElement('span');
    meta.className = 'clip-meta';
    const label = ({ url: '링크', text: '텍스트', file: '파일', image: '이미지' } as const)[item.kind];
    meta.textContent = `${label}${item.size_bytes ? ` · ${size(item.size_bytes)}` : ''} · ${relative(item.created_at)} 전`;
    content.append(text, meta);
    row.append(content);
    if (item.thumbnail_available) {
      const preview = document.createElement('img');
      preview.className = 'clip-thumbnail';
      preview.alt = '';
      row.prepend(preview);
      void thumbnail(item.id).then((value) => {
        if (value && row.isConnected) preview.src = value;
        else if (!value) preview.remove();
      });
    }
    row.addEventListener('mouseenter', () => {
      if (selecting) return;
      selectedId = item.id;
      updateSelection();
    });
    row.addEventListener('click', () => void select(item.id));
    list.append(row);
  }
  updateSelection(true);
  status.textContent = visible.length ? `${visible.length}개 기록` : '기록이 없습니다.';
}

async function refresh() {
  const previous = selectedId;
  items = await invoke<ClipboardItem[]>('clipboard_history');
  selectedId = items.some((item) => item.id === previous) ? previous : (items[0]?.id ?? '');
  render();
}

async function select(id: string) {
  if (selecting) return;
  const item = items.find((value) => value.id === id);
  if (!item) return;
  selecting = true;
  list.setAttribute('aria-busy', 'true');
  status.textContent = ['file', 'image'].includes(item.kind)
    ? '원본 기기에서 파일을 받는 중… Esc를 누르면 취소합니다.'
    : '붙여넣는 중…';
  try {
    await invoke('clipboard_select', { id });
  } catch (error) {
    selecting = false;
    list.removeAttribute('aria-busy');
    status.textContent = String(error);
    search.focus();
  }
}

function move(delta: number) {
  const visible = filtered();
  if (!visible.length) return;
  const current = Math.max(0, visible.findIndex((item) => item.id === selectedId));
  selectedId = visible[(current + delta + visible.length) % visible.length].id;
  updateSelection(true);
}

async function dismiss() {
  if (dismissing) return;
  dismissing = true;
  try {
    await invoke('clipboard_dismiss');
  } catch (error) {
    dismissing = false;
    status.textContent = String(error);
  }
}

search.addEventListener('input', render);
window.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' || event.key === 'Esc') {
    event.preventDefault();
    event.stopImmediatePropagation();
    void dismiss();
    return;
  }
  if (selecting) {
    event.preventDefault();
    return;
  }
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    move(event.key === 'ArrowDown' ? 1 : -1);
  } else if (event.key === 'Enter' && selectedId) {
    event.preventDefault();
    void select(selectedId);
  }
}, { capture: true });

void listen('clipboard-open', () => {
  dismissing = false;
  selecting = false;
  list.removeAttribute('aria-busy');
  search.value = '';
  void refresh().then(() => search.focus());
});
void listen('clipboard-updated', () => void refresh());
void refresh();
