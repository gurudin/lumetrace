import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read=(path:string)=>readFileSync(new URL(`../src/${path}`,import.meta.url),'utf8');
test('restore establishes a synchronous guard before IPC and always releases it',()=>{
  const page=read('pages/file-space/FileSpacePage.tsx');
  const action=page.slice(page.indexOf('const setCurrentTimelineVersion ='),page.indexOf('const swapComparedVersions ='));
  assert.match(action,/timelineBusy \|\| restoringVersionRef.current/);
  assert.ok(action.indexOf('restoringVersionRef.current = versionId')<action.indexOf('await invoke'));
  assert.match(action,/finally \{[\s\S]*restoringVersionRef.current = null/);
  assert.match(page,/aria-busy=\{restoringVersionId === version.id\}/);
  assert.match(page,/disabled=\{timelineBusy \|\| restoringVersionId !== null\}/);
});
test('restore is disclosed on hover or keyboard focus and stays visible while busy',()=>{
  const css=read('styles.css');
  assert.match(css,/> div:hover \.file-version-restore/);
  assert.match(css,/> div:focus-within \.file-version-restore/);
  assert.match(css,/\.file-version-restore\[aria-busy="true"\][\s\S]*opacity: 1/);
  assert.match(css,/@media \(hover: none\)[\s\S]*\.file-version-restore/);
  assert.match(css,/\.file-version-restore \.is-spinning \{ animation: none/);
});
test('all history surfaces use the optional author name and retain a legacy fallback',()=>{
  for(const file of ['FileSpacePage','FileSpaceInspector','MarkdownPreviewOverlay','TextPreviewOverlay','TaskVersionTimelineRail']){
    const source=read(`pages/file-space/${file}.tsx`);
    assert.match(source,/<VersionAuthor name=\{version.authorName\}/);
    assert.match(source,/authorName\?: string \| null/);
  }
  assert.match(read('pages/file-space/VersionAuthor.tsx'),/name\?\.trim\(\)/);
  assert.match(read('pages/file-space/VersionAuthor.tsx'),/fileSpace.timeline.userEdit/);
});
