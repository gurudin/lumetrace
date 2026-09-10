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
  const sizedSource = { imagePreviewUrl: (_id: string, _revision: number, purpose: string) => purpose };
  assert.equal(resolve(file, sizedSource), 'thumbnail');
  assert.equal(resolve(file, sizedSource, 'detail'), 'detail');
  assert.equal(resolve(file, {}), null);
  assert.deepEqual(native, []);
  assert.equal(resolve({ ...file, currentVersion: 1 }, null), 'local-image');
  assert.equal(resolve(file, null), null);
});

test('image loading settles on load/error and resets for a new file, revision or source', () => {
  const text = readFileSync(new URL('../src/pages/file-space/FileSpacePage.tsx', import.meta.url), 'utf8');
  const fn = text.slice(text.indexOf('function FileArtwork('), text.indexOf('\nfunction matchesSearch('));
  const code = ts.transpileModule(fn, { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React, jsxFactory: 'element', jsxFragmentFactory: 'fragment' } }).outputText;
  let state: any;
  let pendingRender = false;
  let source: string | null = 'image-one';
  const dimensions: any[] = [];
  const bindings = {
    element: (type: any, props: any, ...children: any[]) => ({ type, props: { ...props, children } }),
    fragment: 'fragment',
    useState: (initial: any) => [state ??= initial, (next: any) => { state = next; pendingRender = true; }],
    useTranslation: () => ({ t: () => '' }),
    fileIcon: () => 'icon', fileArtworkFormat: () => null,
    fileArchiveExtension: () => null, fileArtworkTitle: () => '',
    useWorkspaceExtension: () => null, filePreviewSource: () => source,
    fileCategory: () => source ? 'image' : 'document', shouldShowFileVersionBadge: () => false,
  };
  const artwork = new Function(...Object.keys(bindings), `${code};return FileArtwork;`)(...Object.values(bindings));
  const file = { id: 'synthetic-image', name: 'preview.png', updatedAt: 1 };
  const render = () => {
    let result: any;
    do { pendingRender = false; result = artwork({ file, onImageDimensions: (...args: any[]) => dimensions.push(args) }); } while (pendingRender);
    return result;
  };
  const isLoading = (node: any) => node.props.className.includes('is-preview-loading');
  let node = render();
  assert.equal(isLoading(node), true);
  assert.equal(node.props['aria-busy'], true);
  assert.equal(node.props.children[0].props.loading, 'lazy');
  node.props.children[0].props.onLoad({ currentTarget: { naturalWidth: 800, naturalHeight: 500 } });
  assert.equal(isLoading(render()), false);
  assert.deepEqual(dimensions, [['synthetic-image', 1, 800, 500]]);

  file.updatedAt = 2;
  node = render();
  assert.equal(isLoading(node), true);
  node.props.children[0].props.onError();
  node = render();
  assert.equal(isLoading(node), false);
  assert.equal(node.props.className.includes('has-preview'), false);
  assert.equal(node.props.children[0].type, 'fragment');

  source = 'image-two';
  node = render();
  assert.equal(isLoading(node), true);
  node.props.children[0].props.onLoad({ currentTarget: { naturalWidth: 800, naturalHeight: 500 } });
  assert.equal(isLoading(render()), false);
  file.id = 'another-image';
  assert.equal(isLoading(render()), true);
  source = null;
  assert.equal(isLoading(render()), false);
});
