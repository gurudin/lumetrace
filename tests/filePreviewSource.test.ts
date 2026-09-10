import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { fileMimeType } from '../src/shared/extensions/fileMimeType.ts';

test('catalogues without MIME metadata classify common files by extension', () => {
  for (const name of ['photo.png', 'PHOTO.JPG', '照片.jpeg', 'a.gif', 'a.webp']) assert.ok(fileMimeType(name)?.startsWith('image/'));
  assert.equal(fileMimeType('notes.md'), 'text/markdown');
  assert.equal(fileMimeType('notes.pdf'), 'application/pdf');
  assert.equal(fileMimeType('movie.mp4'), 'video/mp4');
  assert.equal(fileMimeType('photo.png.exe'), null);
});

test('external images without history use their own transport, never local preview URLs', () => {
  const text = readFileSync(new URL('../src/pages/file-space/FileSpacePage.tsx', import.meta.url), 'utf8');
  const fn = text.slice(text.indexOf('function filePreviewSource('), text.indexOf('\nfunction FileArtwork('));
  const code = ts.transpileModule(fn, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const native: string[] = [];
  const resolve = new Function('fileCategory', 'isTauri', 'convertFileSrc', `${code};return filePreviewSource;`)(
    (f: any) => (f.mimeType ?? fileMimeType(f.name))?.startsWith('image/') ? 'image' : 'other',
    () => true, (id: string) => { native.push(id); return 'local-image'; },
  );
  const file = { id: 'external-file', name: 'photo.png', mimeType: null, currentVersion: null, updatedAt: 10 };
  assert.equal(resolve(file, { imagePreviewUrl: (id: string, revision: number) => `${id}:${revision}` }), 'external-file:10');
  assert.equal(resolve(file, {}), null);
  assert.deepEqual(native, []);
  assert.equal(resolve({ ...file, currentVersion: 1 }, null), 'local-image');
  assert.equal(resolve(file, null), null);
});
