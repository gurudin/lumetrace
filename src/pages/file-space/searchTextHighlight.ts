export interface SearchTextPart {
  text: string;
  highlighted: boolean;
}

export function searchHighlightTerms(query: string) {
  const seen = new Set<string>();
  return query
    .trim()
    .split(/\s+/u)
    .map((term) => term.toLocaleLowerCase())
    .filter((term) => term && !seen.has(term) && seen.add(term))
    .sort((left, right) => right.length - left.length)
    .slice(0, 16);
}

export function splitSearchText(text: string, query: string): SearchTextPart[] {
  const terms = searchHighlightTerms(query);
  if (!text || terms.length === 0) return text ? [{ text, highlighted: false }] : [];
  const folded = text.toLocaleLowerCase();
  const parts: SearchTextPart[] = [];
  let plainStart = 0;
  let cursor = 0;
  while (cursor < text.length) {
    const term = terms.find((candidate) => folded.startsWith(candidate, cursor));
    if (!term) {
      cursor += 1;
      continue;
    }
    if (plainStart < cursor) {
      parts.push({ text: text.slice(plainStart, cursor), highlighted: false });
    }
    parts.push({ text: text.slice(cursor, cursor + term.length), highlighted: true });
    cursor += term.length;
    plainStart = cursor;
  }
  if (plainStart < text.length) {
    parts.push({ text: text.slice(plainStart), highlighted: false });
  }
  return parts.length > 0 ? parts : [{ text, highlighted: false }];
}
