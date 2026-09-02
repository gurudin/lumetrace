export type VersionDiffSegmentKind = "equal" | "added" | "removed";

export interface VersionDiffSegment {
  kind: VersionDiffSegmentKind;
  text: string;
}

export interface VersionDiffSide {
  lineNumber: number;
  text: string;
  segments: VersionDiffSegment[];
}

export interface VersionDiffContentRow {
  kind: "context" | "change";
  before: VersionDiffSide | null;
  after: VersionDiffSide | null;
}

export interface VersionDiffOmittedRow {
  kind: "omitted";
  omittedLines: number;
}

export type VersionDiffRow = VersionDiffContentRow | VersionDiffOmittedRow;

export interface VersionDiffResult {
  rows: VersionDiffRow[];
  addedLines: number;
  removedLines: number;
  identical: boolean;
}

type SequenceOperation<T> = {
  kind: VersionDiffSegmentKind;
  value: T;
};

const maxDynamicCells = 1_500_000;

function splitLines(value: string) {
  return value.replace(/\r\n?/g, "\n").split("\n");
}

function diffLargeSequence<T>(before: readonly T[], after: readonly T[]): SequenceOperation<T>[] {
  let prefixLength = 0;
  while (
    prefixLength < before.length
    && prefixLength < after.length
    && before[prefixLength] === after[prefixLength]
  ) prefixLength += 1;

  let suffixLength = 0;
  while (
    suffixLength < before.length - prefixLength
    && suffixLength < after.length - prefixLength
    && before[before.length - suffixLength - 1] === after[after.length - suffixLength - 1]
  ) suffixLength += 1;

  return [
    ...before.slice(0, prefixLength).map((value) => ({ kind: "equal" as const, value })),
    ...before.slice(prefixLength, before.length - suffixLength).map((value) => ({ kind: "removed" as const, value })),
    ...after.slice(prefixLength, after.length - suffixLength).map((value) => ({ kind: "added" as const, value })),
    ...before.slice(before.length - suffixLength).map((value) => ({ kind: "equal" as const, value })),
  ];
}

function diffSequence<T>(before: readonly T[], after: readonly T[]): SequenceOperation<T>[] {
  if (before.length * after.length > maxDynamicCells) return diffLargeSequence(before, after);

  const columns = after.length + 1;
  const matrix = new Uint32Array((before.length + 1) * columns);
  for (let beforeIndex = before.length - 1; beforeIndex >= 0; beforeIndex -= 1) {
    for (let afterIndex = after.length - 1; afterIndex >= 0; afterIndex -= 1) {
      const index = beforeIndex * columns + afterIndex;
      matrix[index] = before[beforeIndex] === after[afterIndex]
        ? matrix[(beforeIndex + 1) * columns + afterIndex + 1] + 1
        : Math.max(matrix[(beforeIndex + 1) * columns + afterIndex], matrix[index + 1]);
    }
  }

  const operations: SequenceOperation<T>[] = [];
  let beforeIndex = 0;
  let afterIndex = 0;
  while (beforeIndex < before.length || afterIndex < after.length) {
    if (
      beforeIndex < before.length
      && afterIndex < after.length
      && before[beforeIndex] === after[afterIndex]
    ) {
      operations.push({ kind: "equal", value: before[beforeIndex] });
      beforeIndex += 1;
      afterIndex += 1;
    } else if (
      beforeIndex < before.length
      && (
        afterIndex >= after.length
        || matrix[(beforeIndex + 1) * columns + afterIndex] >= matrix[beforeIndex * columns + afterIndex + 1]
      )
    ) {
      operations.push({ kind: "removed", value: before[beforeIndex] });
      beforeIndex += 1;
    } else {
      operations.push({ kind: "added", value: after[afterIndex] });
      afterIndex += 1;
    }
  }
  return operations;
}

function tokenizeLine(value: string) {
  return value.match(/\s+|[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]+/gu) ?? [];
}

function inlineSegments(before: string, after: string) {
  const operations = diffSequence(tokenizeLine(before), tokenizeLine(after));
  return {
    before: operations
      .filter((operation) => operation.kind !== "added")
      .map((operation) => ({ kind: operation.kind, text: operation.value })),
    after: operations
      .filter((operation) => operation.kind !== "removed")
      .map((operation) => ({ kind: operation.kind, text: operation.value })),
  };
}

function collapseContext(rows: VersionDiffContentRow[], contextLines: number): VersionDiffRow[] {
  const changedIndexes = rows
    .map((row, index) => row.kind === "change" ? index : -1)
    .filter((index) => index >= 0);
  if (changedIndexes.length === 0) return [];

  const visible = new Set<number>();
  changedIndexes.forEach((index) => {
    const start = Math.max(0, index - contextLines);
    const end = Math.min(rows.length - 1, index + contextLines);
    for (let visibleIndex = start; visibleIndex <= end; visibleIndex += 1) visible.add(visibleIndex);
  });

  const result: VersionDiffRow[] = [];
  let omittedLines = 0;
  rows.forEach((row, index) => {
    if (!visible.has(index)) {
      omittedLines += 1;
      return;
    }
    if (omittedLines > 0) {
      result.push({ kind: "omitted", omittedLines });
      omittedLines = 0;
    }
    result.push(row);
  });
  if (omittedLines > 0) result.push({ kind: "omitted", omittedLines });
  return result;
}

export function diffTextVersions(beforeText: string, afterText: string, contextLines = 3): VersionDiffResult {
  const operations = diffSequence(splitLines(beforeText), splitLines(afterText));
  const rows: VersionDiffContentRow[] = [];
  let beforeLineNumber = 1;
  let afterLineNumber = 1;
  let addedLines = 0;
  let removedLines = 0;

  for (let operationIndex = 0; operationIndex < operations.length;) {
    const operation = operations[operationIndex];
    if (operation.kind === "equal") {
      const text = operation.value;
      rows.push({
        kind: "context",
        before: { lineNumber: beforeLineNumber, text, segments: [{ kind: "equal", text }] },
        after: { lineNumber: afterLineNumber, text, segments: [{ kind: "equal", text }] },
      });
      beforeLineNumber += 1;
      afterLineNumber += 1;
      operationIndex += 1;
      continue;
    }

    const removed: Array<{ lineNumber: number; text: string }> = [];
    const added: Array<{ lineNumber: number; text: string }> = [];
    while (operationIndex < operations.length && operations[operationIndex].kind !== "equal") {
      const changedOperation = operations[operationIndex];
      if (changedOperation.kind === "removed") {
        removed.push({ lineNumber: beforeLineNumber, text: changedOperation.value });
        beforeLineNumber += 1;
        removedLines += 1;
      } else {
        added.push({ lineNumber: afterLineNumber, text: changedOperation.value });
        afterLineNumber += 1;
        addedLines += 1;
      }
      operationIndex += 1;
    }

    const pairCount = Math.max(removed.length, added.length);
    for (let pairIndex = 0; pairIndex < pairCount; pairIndex += 1) {
      const before = removed[pairIndex] ?? null;
      const after = added[pairIndex] ?? null;
      const segments = before && after ? inlineSegments(before.text, after.text) : null;
      rows.push({
        kind: "change",
        before: before ? {
          ...before,
          segments: segments?.before ?? [{ kind: "removed", text: before.text }],
        } : null,
        after: after ? {
          ...after,
          segments: segments?.after ?? [{ kind: "added", text: after.text }],
        } : null,
      });
    }
  }

  return {
    rows: collapseContext(rows, Math.max(0, contextLines)),
    addedLines,
    removedLines,
    identical: addedLines === 0 && removedLines === 0,
  };
}

export function defaultVersionComparison(
  versions: readonly { id: string; versionNumber: number; isCurrent: boolean }[],
  currentVersionId: string,
) {
  if (versions.length < 2) return null;
  const current = versions.find((version) => version.id === currentVersionId)
    ?? versions.find((version) => version.isCurrent)
    ?? versions[0];
  const chronological = [...versions].sort((left, right) => left.versionNumber - right.versionNumber);
  const currentIndex = chronological.findIndex((version) => version.id === current.id);
  const previous = chronological[currentIndex - 1];
  if (previous) return { beforeVersionId: previous.id, afterVersionId: current.id };
  const next = chronological[currentIndex + 1];
  return next ? { beforeVersionId: current.id, afterVersionId: next.id } : null;
}
