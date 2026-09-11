import assert from "node:assert/strict";
import test from "node:test";
import { enHelpArticles, zhHelpArticles, helpArticles, searchHelp } from "../src/shared/help/helpContent.ts";

test("both guide editions cover the same complete local workflows", () => {
  const required = ["start", "workspaces", "import", "organize", "preview", "search", "cloud-ai", "local-ai", "agent-cli", "lumie", "vectors", "history", "trash-backup", "background", "appearance"];
  for (const articles of [enHelpArticles, zhHelpArticles]) {
    assert.deepEqual(articles.map(a => a.id), required);
    for (const article of articles) {
      assert.ok(article.title && article.summary && article.sections.length >= 2);
      assert.ok(article.sections.every(section => section.steps.length >= 1));
    }
  }
});
test("help searches actual instructions, tolerates spaces and never fabricates empty results", () => {
  assert.equal(searchHelp(zhHelpArticles, "向量")[0].id, "vectors");
  assert.ok(searchHelp(zhHelpArticles, "API Key").some(a => a.id === "cloud-ai"));
  assert.ok(searchHelp(enHelpArticles, "  OLLAMA   11434 ").some(a => a.id === "local-ai"));
  assert.equal(searchHelp(enHelpArticles, "nonexistent-xyz-123").length, 0);
  assert.equal(searchHelp(enHelpArticles, "   ").length, enHelpArticles.length);
  assert.equal(helpArticles("zh-CN"), zhHelpArticles);
  assert.equal(helpArticles("ja-JP"), enHelpArticles);
});
