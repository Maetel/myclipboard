import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const contractUrl = new URL('../contracts/clipboard-v1.openapi.json', import.meta.url);
const source = await readFile(contractUrl, 'utf8');
const contract = JSON.parse(source);

assert.equal(contract.info.version, '1.0.0');
assert.deepEqual(contract.servers, [{ url: 'https://memos.my/api/clipboard/v1' }]);
assert.equal(
  contract.components.securitySchemes.clipboardSession.bearerFormat,
  'smc_<43 base64url characters>',
);
assert.deepEqual(Object.keys(contract.paths).sort(), [
  '/feed',
  '/file-requests/pending',
  '/file-requests/{requestId}',
  '/file-requests/{requestId}/content',
  '/items',
  '/items/{itemId}',
  '/items/{itemId}/file-requests',
  '/items/{itemId}/thumbnail',
  '/login',
  '/logout',
  '/me',
  '/password',
  '/spaces',
]);
assert.doesNotMatch(source, /jamserver|admin\.memos\.my|jmc_/i);

console.log('Clipboard v1 contract is standalone and valid.');
