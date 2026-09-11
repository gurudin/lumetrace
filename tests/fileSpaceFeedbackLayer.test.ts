import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('feedback stays above mounted file dialogs without raising the workbench', () => {
  const page = readFileSync(new URL('../src/pages/file-space/FileSpacePage.tsx', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../src/styles.css', import.meta.url), 'utf8');
  assert.match(page, /file-space-feedback-stack\$\{isFileSpaceDialogMounted \? " is-above-dialog"/);
  const backdrop = Number(css.match(/\.file-space-dialog-backdrop\s*\{[^}]*z-index:\s*(\d+)/)?.[1]);
  const feedback = Number(css.match(/\.file-space-feedback-stack\.is-above-dialog\s*\{[^}]*z-index:\s*(\d+)/)?.[1]);
  assert.ok(feedback > backdrop);
  assert.match(page, /is-failed" role="alert" aria-live="assertive"/);
});
