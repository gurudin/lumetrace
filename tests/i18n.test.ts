import assert from "node:assert/strict";
import test from "node:test";
import {
  getSystemLanguage,
  matchSupportedLanguage,
  resolveLanguagePreference,
  supportedLanguages,
} from "../src/shared/i18n/language.ts";
import { de } from "../src/shared/i18n/locales/de.ts";
import { en } from "../src/shared/i18n/locales/en.ts";
import { es } from "../src/shared/i18n/locales/es.ts";
import { fr } from "../src/shared/i18n/locales/fr.ts";
import { ja } from "../src/shared/i18n/locales/ja.ts";
import { ko } from "../src/shared/i18n/locales/ko.ts";
import { zh } from "../src/shared/i18n/locales/zh.ts";
import { zhTW } from "../src/shared/i18n/locales/zhTW.ts";

function flattenTranslations(value: unknown, prefix = "", flattened = new Map<string, string>()) {
  if (typeof value === "string") {
    flattened.set(prefix, value);
    return flattened;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`Translation value at ${prefix || "<root>"} must be a string or object.`);
  }
  for (const [key, child] of Object.entries(value)) {
    flattenTranslations(child, prefix ? `${prefix}.${key}` : key, flattened);
  }
  return flattened;
}

function interpolationTokens(value: string) {
  return [...value.matchAll(/{{\s*([^},\s]+)[^}]*}}/g)].map((match) => match[1]).sort();
}

const translations = {
  "en-US": en,
  "zh-CN": zh,
  "zh-TW": zhTW,
  "de-DE": de,
  "ja-JP": ja,
  "ko-KR": ko,
  "fr-FR": fr,
  "es-ES": es,
};

test("all supported languages provide the same translation keys and interpolations", () => {
  const english = flattenTranslations(en);
  const expectedKeys = [...english.keys()].sort();

  assert.deepEqual(supportedLanguages.map(({ code }) => code), Object.keys(translations));
  for (const [locale, resource] of Object.entries(translations)) {
    const flattened = flattenTranslations(resource);
    assert.deepEqual([...flattened.keys()].sort(), expectedKeys, `${locale} translation keys differ from en-US`);
    for (const [key, englishValue] of english) {
      assert.deepEqual(
        interpolationTokens(flattened.get(key) ?? ""),
        interpolationTokens(englishValue),
        `${locale} interpolation differs at ${key}`,
      );
    }
  }
});

test("system language detection maps regional variants to the supported locales", () => {
  assert.equal(getSystemLanguage(["zh-Hant-HK"]), "zh-TW");
  assert.equal(getSystemLanguage(["zh-SG"]), "zh-CN");
  assert.equal(getSystemLanguage(["de-AT"]), "de-DE");
  assert.equal(getSystemLanguage(["pt-BR", "fr-CA"]), "fr-FR");
  assert.equal(getSystemLanguage(["pt-BR"]), "en-US");
  assert.equal(matchSupportedLanguage("ko_KR"), "ko-KR");
});

test("an explicit language preference overrides the system language", () => {
  assert.equal(resolveLanguagePreference("system", ["ja-JP"]), "ja-JP");
  assert.equal(resolveLanguagePreference("es-ES", ["ja-JP"]), "es-ES");
});

test("Chinese file cards use the compact version badge copy", () => {
  assert.equal(zh.fileSpace.content.versionCount_one, "共 {{count}} 版");
  assert.equal(zh.fileSpace.content.versionCount_other, "共 {{count}} 版");
  assert.equal(zhTW.fileSpace.content.versionCount_one, "共 {{count}} 版");
  assert.equal(zhTW.fileSpace.content.versionCount_other, "共 {{count}} 版");
});

test("sidebar favorites heading uses the requested Chinese copy", () => {
  assert.equal(zh.fileSpace.sidebar.favorites, "常用");
  assert.equal(zhTW.fileSpace.sidebar.favorites, "常用");
});

test("folder menu exposes recursive expansion actions", () => {
  assert.equal(zh.fileSpace.folderMenu.expandAll, "展开全部");
  assert.equal(zh.fileSpace.folderMenu.collapseAll, "收起全部");
  assert.equal(zhTW.fileSpace.folderMenu.expandAll, "全部展開");
  assert.equal(zhTW.fileSpace.folderMenu.collapseAll, "全部收合");
});

test("file area menu and text-file creation use the requested Chinese flow", () => {
  assert.equal(zh.fileSpace.contentMenu.createFile, "新建文件");
  assert.equal(zh.fileSpace.contentMenu.layout, "布局方式");
  assert.equal(zh.fileSpace.contentMenu.layoutOptions.adaptive, "自适应");
  assert.equal(zh.fileSpace.contentMenu.layoutOptions.list, "列表");
  assert.equal(zh.fileSpace.content.listColumns.name, "名称");
  assert.equal(zh.fileSpace.content.listColumns.dimensions, "尺寸");
  assert.equal(zh.fileSpace.content.listColumns.extension, "扩展名");
  assert.equal(zh.fileSpace.content.listColumns.fileSize, "文件大小");
  assert.equal(zh.fileSpace.content.listColumns.addedAt, "添加日期");
  assert.equal(zh.fileSpace.contentMenu.sort, "排列方式");
  assert.equal(zh.fileSpace.contentMenu.automatic, "自动");
  assert.match(zh.fileSpace.contentMenu.automaticHint, /拖拽/);
  assert.equal(zh.fileSpace.toolbar.sortOptions.manual, "自动");
  assert.match(zh.fileSpace.contentMenu.ascendingHint, /最早修改/);
  assert.match(zh.fileSpace.contentMenu.descendingHint, /最近修改/);
  assert.equal(zh.fileSpace.contentMenu.showFolders, "显示文件夹");
  assert.equal(zh.fileSpace.contentMenu.hideFolders, "隐藏文件夹");
  assert.equal(zh.fileSpace.contentMenu.showInfo, "显示信息栏");
  assert.equal(zh.fileSpace.contentMenu.hideInfo, "隐藏信息栏");
  assert.equal(zh.fileSpace.createFile.dialogTitle, "新建文件");
  assert.equal(zh.fileSpace.createFile.formats.md, "Markdown");
  assert.equal(zh.fileSpace.createFile.formats.txt, "TXT");
  assert.equal(zh.fileSpace.createFile.defaultName, "未命名");
  assert.match(zh.fileSpace.createFile.fixedExtension, /{{extension}}/);
});

test("global file search exposes the requested platform shortcut and navigation copy", () => {
  assert.equal(zh.fileSpace.globalSearch.trigger, "搜索");
  assert.match(zh.fileSpace.globalSearch.openShortcut, /{{shortcut}}/);
  assert.equal(zh.fileSpace.globalSearch.navigateHint, "选择");
  assert.equal(zh.fileSpace.globalSearch.openHint, "打开");
  assert.equal(zh.fileSpace.globalSearch.closeHint, "关闭");
});

test("preview timeline controls have explicit Chinese actions", () => {
  assert.equal(zh.fileSpace.preview.common.showVersionHistory, "显示版本记录");
  assert.equal(zh.fileSpace.preview.common.hideVersionHistory, "隐藏版本记录");
  assert.equal(zhTW.fileSpace.preview.common.showVersionHistory, "顯示版本歷程");
  assert.equal(zhTW.fileSpace.preview.common.hideVersionHistory, "隱藏版本歷程");
});

test("trash restore confirmation uses the requested Chinese prompt", () => {
  assert.equal(zh.fileSpace.trash.confirmDescription, "将恢复到原有位置，是否恢复？");
});

test("automatic version notification uses the requested Chinese actions", () => {
  assert.equal(zh.fileSpace.versionNotification.acknowledge, "知道了");
  assert.equal(zh.fileSpace.versionNotification.view, "查看");
});

test("AI waiting state exposes a live elapsed-seconds label", () => {
  assert.equal(zh.fileSpace.ai.waiting, "已等待 {{seconds}}s");
  assert.equal(zh.fileSpace.ai.asking, "正在检索相关文件…");
  assert.equal(zh.fileSpace.ai.generating, "正在等待 AI 响应…");
  assert.equal(zh.fileSpace.ai.thinking, "正在思考…");
  assert.match(en.fileSpace.ai.waiting, /{{seconds}}/);
});

test("local model settings expose a real connection flow", () => {
  assert.equal(zh.fileSpace.settings.aiService.modelPlaceholder, "请先测试连接");
  assert.equal(zh.fileSpace.settings.aiService.baseUrlPlaceholder, "请输入服务地址");
  assert.equal(zh.fileSpace.settings.aiService.baseUrlExample, "示例：{{url}}");
  assert.match(zh.fileSpace.settings.aiService.localConnection.idle.description, /测试连接/);
  assert.match(zh.fileSpace.settings.aiService.localConnection.passed.description, /选择模型/);
  assert.doesNotMatch(zh.fileSpace.settings.aiService.localPrivacy, /未来/);
});

test("AI no-result copy exposes an inline index-status action", () => {
  assert.equal(zh.fileSpace.ai.noSourcesBefore, "没有找到足够相关的本地文件内容，请换一种问法或");
  assert.equal(zh.fileSpace.ai.confirmIndex, "确认索引");
  assert.equal(zh.fileSpace.ai.noSourcesAfter, "已经完成。");
});

test("same-name import conflict offers versioning and independent-file actions", () => {
  assert.equal(zh.fileSpace.importConflict.rename, "保留为新文件");
  assert.equal(zh.fileSpace.importConflict.latestVersion, "添加为最新版本");
  assert.match(zh.fileSpace.importConflict.description_one, /最新版本/);
});

test("identical imports explain the skip and offer direct file location", () => {
  assert.equal(zh.fileSpace.importConflict.identicalTitleSingle, "未导入重复文件");
  assert.match(zh.fileSpace.importConflict.identicalDescriptionSingle, /内容完全相同/);
  assert.equal(zh.fileSpace.importConflict.showFile, "显示文件");
});

test("trash retention and permanent deletion are explicit in Chinese", () => {
  assert.match(zh.fileSpace.trash.emptyDescription, /30 天/);
  assert.match(zh.fileSpace.trash.emptyConfirmDescription_one, /永久删除/);
  assert.match(zh.fileSpace.trash.emptyConfirmDescription_other, /永久删除/);
  assert.match(zh.fileSpace.trash.emptyConfirmDescription_other, /无法恢复/);
});

test("trash count copy distinguishes singular and plural where the language requires it", () => {
  assert.match(en.fileSpace.trash.emptyConfirmDescription_one, /\bitem\b/);
  assert.match(en.fileSpace.trash.emptyConfirmDescription_other, /\bitems\b/);
  assert.match(en.fileSpace.trash.autoDeleted_one, /\bitem\b/);
  assert.match(en.fileSpace.trash.autoDeleted_other, /\bitems\b/);
  assert.notEqual(de.fileSpace.trash.emptyConfirmDescription_one, de.fileSpace.trash.emptyConfirmDescription_other);
  assert.notEqual(fr.fileSpace.trash.autoDeleted_one, fr.fileSpace.trash.autoDeleted_other);
  assert.notEqual(es.fileSpace.trash.autoDeleted_one, es.fileSpace.trash.autoDeleted_other);
});
