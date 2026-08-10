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
const retry = document.getElementById('retryHistory') as HTMLButtonElement;
let items: ClipboardItem[] = [];
let selectedId = '';
let dismissing = false;
let selecting = false;
let selectionId = '';
let selectionStartedAt = 0;
let selectionTimer: number | undefined;
let refreshRunning = false;
let refreshQueued = false;
let viewEpoch = 0;
let renderedSignature = '';
const thumbnails = new Map<string, Promise<string | null>>();

const thumbnailObserver = new IntersectionObserver((entries) => {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue;
    const preview = entry.target as HTMLImageElement;
    thumbnailObserver.unobserve(preview);
    const id = preview.dataset.thumbnailId;
    if (!id) continue;
    void thumbnail(id).then((value) => {
      if (value && preview.isConnected) preview.src = value;
      else if (!value) preview.remove();
    });
  }
}, { root: list, rootMargin: '80px 0px' });

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
    row.classList.toggle('loading', selecting && row.dataset.id === selectionId);
    row.ariaSelected = String(selected);
  }
  if (selectedId) search.setAttribute('aria-activedescendant', optionId(selectedId));
  else search.removeAttribute('aria-activedescendant');
  if (scroll) list.querySelector('.selected')?.scrollIntoView({ block: 'nearest' });
}

function kindIcon(item: ClipboardItem) {
  if (item.thumbnail_available) {
    const preview = document.createElement('img');
    preview.className = 'clip-thumbnail';
    preview.alt = '';
    preview.dataset.thumbnailId = item.id;
    thumbnailObserver.observe(preview);
    return preview;
  }
  const icon = document.createElement('span');
  icon.className = `clip-icon kind-${item.kind}`;
  icon.setAttribute('aria-hidden', 'true');
  icon.textContent = item.kind === 'file' ? '📄' : item.kind === 'image' ? '▧' : item.kind === 'url' ? '↗' : 'T';
  return icon;
}

function render() {
  const visible = filtered();
  if (!visible.some((item) => item.id === selectedId)) selectedId = visible[0]?.id ?? '';
  thumbnailObserver.disconnect();
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
    row.append(kindIcon(item), content);
    row.addEventListener('mouseenter', () => {
      if (selecting) return;
      selectedId = item.id;
      updateSelection();
    });
    row.addEventListener('click', () => {
      if (selecting) return;
      selectedId = item.id;
      updateSelection();
    });
    row.addEventListener('dblclick', (event) => {
      event.preventDefault();
      void select(item.id);
    });
    list.append(row);
  }
  updateSelection(true);
  if (!selecting) status.textContent = visible.length ? `${visible.length}개 기록` : '기록이 없습니다.';
}

function signature(values: ClipboardItem[]) {
  return JSON.stringify(values.map((item) => [
    item.id, item.kind, item.text, item.filename, item.size_bytes,
    item.thumbnail_available, item.created_at,
  ]));
}

async function refresh() {
  if (selecting || refreshRunning) {
    refreshQueued = true;
    return;
  }
  refreshRunning = true;
  refreshQueued = false;
  const epoch = viewEpoch;
  list.setAttribute('aria-busy', 'true');
  retry.hidden = true;
  if (!items.length) status.textContent = '클립보드 기록을 불러오는 중…';
  try {
    const next = await invoke<ClipboardItem[]>('clipboard_history');
    if (epoch !== viewEpoch) return;
    const nextSignature = signature(next);
    const previous = selectedId;
    items = next;
    selectedId = items.some((item) => item.id === previous) ? previous : (items[0]?.id ?? '');
    if (nextSignature !== renderedSignature) {
      renderedSignature = nextSignature;
      render();
    } else if (!selecting) {
      status.textContent = filtered().length ? `${filtered().length}개 기록` : '기록이 없습니다.';
    }
  } catch (error) {
    if (epoch !== viewEpoch) return;
    status.textContent = `기록을 불러오지 못했습니다. ${String(error)}`;
    retry.hidden = false;
  } finally {
    refreshRunning = false;
    list.removeAttribute('aria-busy');
    if (refreshQueued && !selecting) {
      refreshQueued = false;
      void refresh();
    }
  }
}

function updateTransferStatus(item: ClipboardItem) {
  const elapsed = Math.max(0, Math.floor((Date.now() - selectionStartedAt) / 1000));
  status.textContent = `${item.kind === 'file' ? '파일' : '이미지'}을 받는 중 · ${elapsed}초 — Esc로 취소`;
}

function nextPaint() {
  return new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}

async function select(id: string) {
  if (selecting) return;
  const item = items.find((value) => value.id === id);
  if (!item) return;
  selecting = true;
  selectionId = id;
  selectionStartedAt = Date.now();
  list.setAttribute('aria-busy', 'true');
  updateSelection();
  if (['file', 'image'].includes(item.kind)) {
    updateTransferStatus(item);
    selectionTimer = window.setInterval(() => updateTransferStatus(item), 1000);
  } else {
    status.textContent = '붙여넣는 중…';
  }
  await nextPaint();
  try {
    await invoke('clipboard_select', { id });
  } catch (error) {
    status.textContent = String(error);
    search.focus();
  } finally {
    if (selectionTimer !== undefined) window.clearInterval(selectionTimer);
    selectionTimer = undefined;
    selecting = false;
    selectionId = '';
    list.removeAttribute('aria-busy');
    updateSelection();
    if (refreshQueued) {
      refreshQueued = false;
      void refresh();
    }
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
retry.addEventListener('click', () => void refresh());
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
  viewEpoch += 1;
  dismissing = false;
  selecting = false;
  selectionId = '';
  list.removeAttribute('aria-busy');
  search.value = '';
  refreshQueued = true;
  void refresh();
  search.focus();
});
void listen('clipboard-updated', () => {
  refreshQueued = true;
  void refresh();
});
void refresh();
