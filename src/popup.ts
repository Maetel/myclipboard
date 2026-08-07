import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './popup.css';

interface ClipboardItem { id:string; origin_device_id:string; kind:'text'|'url'; text:string; created_at:string }
const search = document.getElementById('clipboardSearch') as HTMLInputElement;
const list = document.getElementById('clipboardItems') as HTMLOListElement;
const status = document.getElementById('clipboardStatus') as HTMLDivElement;
let items: ClipboardItem[] = [], selectedId = '', dismissing = false;
const filtered = () => { const q=search.value.trim().toLocaleLowerCase(); return q ? items.filter(item=>item.text.toLocaleLowerCase().includes(q)) : items };
function relative(value:string){const s=Math.max(0,Math.floor((Date.now()-Date.parse(value))/1000));if(s<60)return `${s}초`;if(s<3600)return `${Math.floor(s/60)}분`;if(s<86400)return `${Math.floor(s/3600)}시간`;return `${Math.floor(s/86400)}일`}
function render(){const visible=filtered();if(!visible.some(item=>item.id===selectedId))selectedId=visible[0]?.id||'';list.replaceChildren();for(const item of visible){const row=document.createElement('li');row.dataset.id=item.id;row.role='option';row.ariaSelected=String(item.id===selectedId);row.className=item.id===selectedId?'selected':'';const text=document.createElement('span');text.className='clip-text';text.textContent=item.text;const meta=document.createElement('span');meta.className='clip-meta';meta.textContent=`${item.kind==='url'?'링크':'텍스트'} · ${relative(item.created_at)} 전 · ${item.origin_device_id.slice(-8)}`;row.append(text,meta);row.addEventListener('mouseenter',()=>{selectedId=item.id;render()});row.addEventListener('click',()=>void select(item.id));list.append(row)}status.textContent=visible.length?`${visible.length}개 기록`:'기록이 없습니다.';list.querySelector('.selected')?.scrollIntoView({block:'nearest'})}
async function refresh(){const previous=selectedId;items=await invoke<ClipboardItem[]>('clipboard_history');selectedId=items.some(item=>item.id===previous)?previous:items[0]?.id||'';render()}
async function select(id:string){status.textContent='붙여넣는 중…';try{await invoke('clipboard_select',{id})}catch(error){status.textContent=String(error)}}
function move(delta:number){const visible=filtered();if(!visible.length)return;const current=Math.max(0,visible.findIndex(item=>item.id===selectedId));selectedId=visible[(current+delta+visible.length)%visible.length].id;render()}
async function dismiss(){if(dismissing)return;dismissing=true;try{await invoke('clipboard_dismiss')}catch(error){dismissing=false;status.textContent=String(error)}}
search.addEventListener('input',render);window.addEventListener('keydown',(event)=>{if(event.key==='Escape'||event.key==='Esc'){event.preventDefault();event.stopImmediatePropagation();void dismiss();return}if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();move(event.key==='ArrowDown'?1:-1)}else if(event.key==='Enter'&&selectedId){event.preventDefault();void select(selectedId)}},{capture:true});
void listen('clipboard-open',()=>{dismissing=false;search.value='';void refresh().then(()=>search.focus())});void listen('clipboard-updated',()=>void refresh());void refresh();

