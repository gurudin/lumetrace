import test from 'node:test';
import assert from 'node:assert/strict';
import { originalImageSource } from '../src/pages/file-space/originalImagePreview.ts';

test('original action requires a distinct explicit original source, not a scaled display', () => {
  assert.equal(originalImageSource('local-original'), null);
  assert.equal(originalImageSource('local-original', 'local-original'), null);
  assert.equal(originalImageSource('preview-800', 'original'), 'original');
  assert.equal(originalImageSource('preview-800', ''), null);
});
