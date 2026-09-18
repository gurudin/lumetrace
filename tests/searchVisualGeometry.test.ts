import assert from "node:assert/strict";
import test from "node:test";
import { mergeVisualRectangles, visualPreviewKind } from "../src/pages/file-space/searchVisualGeometry.ts";

test("visual search supports image/PDF sources without replacing text previews", () => {
  for (const extension of ["PNG", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "heic", "heif"]) assert.equal(visualPreviewKind(`file.${extension}`), "image");
  assert.equal(visualPreviewKind("scan.PDF"), "pdf");
  for (const name of ["doc.md", "doc.docx", "plain.txt"]) assert.equal(visualPreviewKind(name), null);
});
test("adjacent character boxes merge without crossing lines or mutating cached geometry", () => {
  const input = [{x:0.1,y:0.2,width:0.03,height:0.05},{x:0.13,y:0.2,width:0.04,height:0.05},{x:0.1,y:0.4,width:0.1,height:0.05}];
  const copy = structuredClone(input);
  const output = mergeVisualRectangles(input);
  assert.equal(output.length,2);
  assert(Math.abs(output[0].width-0.07)<0.00001);
  assert.deepEqual(input,copy);
  assert.deepEqual(mergeVisualRectangles([{x:NaN,y:0,width:1,height:1},{x:0.9,y:0,width:1,height:1}]),[]);
});
