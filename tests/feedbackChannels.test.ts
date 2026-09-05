import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("feedback channels use the approved project destinations without attached data", () => {
  const channels = JSON.parse(readFileSync(new URL("../src/shared/feedbackChannels.json", import.meta.url), "utf8"));
  assert.deepEqual(channels, {
    github: "https://github.com/gurudin/lumetrace/issues",
    discord: "https://discord.gg/6pJVMTJ5UG",
  });
});

test("feedback brand marks are bundled SVG shapes without remote content or scripts", () => {
  for (const name of ["github", "discord"]) {
    const svg = readFileSync(new URL(`../src/assets/brands/${name}-mark.svg`, import.meta.url), "utf8");
    assert.match(svg, /<svg[^>]*viewBox=/);
    assert.match(svg, /<path d=/);
    assert.doesNotMatch(svg, /<(?:script|image|foreignObject)\b|\bhref=|\bon\w+=/i);
  }
});
