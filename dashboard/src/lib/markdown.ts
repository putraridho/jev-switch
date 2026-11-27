export type Inline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong"; children: Inline[] }
  | { kind: "em"; children: Inline[] }
  | { kind: "link"; children: Inline[]; href: string };

export type Block =
  | { kind: "paragraph"; children: Inline[] }
  | { kind: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; children: Inline[] }
  | { kind: "code"; lang: string; text: string }
  | { kind: "list"; ordered: boolean; start: number; items: Inline[][] }
  | { kind: "quote"; children: Block[] }
  | { kind: "rule" };

type CodeSpan = { contentStart: number; closeStart: number; end: number };
type LinkSpan = {
  labelStart: number;
  labelEnd: number;
  hrefStart: number;
  hrefEnd: number;
  end: number;
};
type InlineFrame = {
  marker: string;
  parent: InlineFrame | null;
  children: Inline[];
  pendingText: string;
};
type ListMarker = {
  ordered: boolean;
  start: number;
  contentStart: number;
};
type Fence = { character: string; length: number; lang: string };

function isPunctuation(character: string | undefined): boolean {
  if (character === undefined) return false;
  const code = character.charCodeAt(0);
  return (
    (code >= 33 && code <= 47) ||
    (code >= 58 && code <= 64) ||
    (code >= 91 && code <= 96) ||
    (code >= 123 && code <= 126)
  );
}

function isWordCharacter(character: string | undefined): boolean {
  if (character === undefined) return false;
  const code = character.charCodeAt(0);
  return (
    (code >= 48 && code <= 57) ||
    (code >= 65 && code <= 90) ||
    (code >= 97 && code <= 122)
  );
}

function findCodeSpans(source: string): Map<number, CodeSpan> {
  const runsByLength = new Map<number, number[]>();
  const runAt = new Map<number, number>();
  const escapedRuns = new Set<number>();
  let backslashes = 0;
  let index = 0;

  while (index < source.length) {
    const character = source[index];
    if (character === "\\") {
      backslashes += 1;
      index += 1;
      continue;
    }
    if (character === "`") {
      const start = index;
      while (source[index] === "`") index += 1;
      const length = index - start;
      const starts = runsByLength.get(length) ?? [];
      starts.push(start);
      runsByLength.set(length, starts);
      runAt.set(start, length);
      if (backslashes % 2 === 1) escapedRuns.add(start);
      backslashes = 0;
      continue;
    }
    backslashes = 0;
    index += 1;
  }

  const nextRun = new Map<number, number>();
  for (const starts of runsByLength.values()) {
    for (let position = 0; position + 1 < starts.length; position += 1) {
      const start = starts[position];
      const next = starts[position + 1];
      if (start !== undefined && next !== undefined) nextRun.set(start, next);
    }
  }

  const spans = new Map<number, CodeSpan>();
  index = 0;
  while (index < source.length) {
    const length = runAt.get(index);
    if (length === undefined) {
      index += 1;
      continue;
    }
    if (escapedRuns.has(index)) {
      index += length;
      continue;
    }
    const close = nextRun.get(index);
    if (close === undefined) {
      index += length;
      continue;
    }
    spans.set(index, {
      contentStart: index + length,
      closeStart: close,
      end: close + length,
    });
    index = close + length;
  }
  return spans;
}

function findLinkClose(source: string, start: number): number | undefined {
  let index = start;
  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (character === "\\" && isPunctuation(next)) {
      index += 2;
      continue;
    }
    if (character === ")") return index;
    index += 1;
  }
  return undefined;
}

function findLinkSpans(
  source: string,
  codeSpans: Map<number, CodeSpan>,
): Map<number, LinkSpan> {
  const openings: number[] = [];
  const spans = new Map<number, LinkSpan>();
  let index = 0;

  while (index < source.length) {
    const code = codeSpans.get(index);
    if (code !== undefined) {
      index = code.end;
      continue;
    }
    const character = source[index];
    const next = source[index + 1];
    if (character === "\\" && isPunctuation(next)) {
      index += 2;
      continue;
    }
    if (character === "[") {
      openings.push(index);
      index += 1;
      continue;
    }
    if (character === "]" && next === "(" && openings.length > 0) {
      const opening = openings.pop();
      if (opening !== undefined) {
        const close = findLinkClose(source, index + 2);
        if (close === undefined) break;
        spans.set(opening, {
          labelStart: opening + 1,
          labelEnd: index,
          hrefStart: index + 2,
          hrefEnd: close,
          end: close + 1,
        });
        index = close + 1;
        continue;
      }
    }
    index += 1;
  }
  return spans;
}

function appendInline(frame: InlineFrame, node: Inline): void {
  const previous = frame.children[frame.children.length - 1];
  if (node.kind === "text" && previous?.kind === "text") {
    previous.text += node.text;
  } else {
    frame.children.push(node);
  }
}

function flushText(frame: InlineFrame): void {
  if (frame.pendingText.length === 0) return;
  appendInline(frame, { kind: "text", text: frame.pendingText });
  frame.pendingText = "";
}

function appendLiteralFrame(
  frame: InlineFrame,
  parent: InlineFrame,
  active: Map<string, InlineFrame>,
): void {
  flushText(frame);
  if (active.get(frame.marker) === frame) active.delete(frame.marker);
  appendInline(parent, { kind: "text", text: frame.marker });
  for (const child of frame.children) appendInline(parent, child);
}

function markerAt(source: string, index: number): string | undefined {
  const character = source[index];
  if (character !== "*" && character !== "_") return undefined;
  return source[index + 1] === character ? character + character : character;
}

function isWordInternalUnderscore(source: string, index: number): boolean {
  return (
    isWordCharacter(source[index - 1]) && isWordCharacter(source[index + 1])
  );
}

function unescapePunctuation(source: string): string {
  let result = "";
  let index = 0;
  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (character === "\\" && isPunctuation(next)) {
      result += next ?? "";
      index += 2;
    } else {
      result += character ?? "";
      index += 1;
    }
  }
  return result;
}

function parseInlineInternal(source: string): Inline[] {
  const codeSpans = findCodeSpans(source);
  const linkSpans = findLinkSpans(source, codeSpans);
  const root: InlineFrame = {
    marker: "",
    parent: null,
    children: [],
    pendingText: "",
  };
  const active = new Map<string, InlineFrame>();
  let current = root;
  let index = 0;

  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (character === "\\" && isPunctuation(next)) {
      current.pendingText += next ?? "";
      index += 2;
      continue;
    }

    const code = codeSpans.get(index);
    if (code !== undefined) {
      flushText(current);
      current.children.push({
        kind: "code",
        text: source.slice(code.contentStart, code.closeStart),
      });
      index = code.end;
      continue;
    }

    const link = linkSpans.get(index);
    if (link !== undefined) {
      flushText(current);
      current.children.push({
        kind: "link",
        children: parseInlineInternal(
          source.slice(link.labelStart, link.labelEnd),
        ),
        href: unescapePunctuation(source.slice(link.hrefStart, link.hrefEnd)),
      });
      index = link.end;
      continue;
    }

    const marker = markerAt(source, index);
    if (
      marker !== undefined &&
      !(marker === "_" && isWordInternalUnderscore(source, index))
    ) {
      const matching = active.get(marker);
      if (matching !== undefined) {
        while (current !== matching) {
          const parent = current.parent;
          if (parent === null) break;
          appendLiteralFrame(current, parent, active);
          current = parent;
        }
        if (current === matching) {
          flushText(current);
          const parent = current.parent;
          if (parent !== null) {
            active.delete(marker);
            const node: Inline =
              marker === "**" || marker === "__"
                ? { kind: "strong", children: current.children }
                : { kind: "em", children: current.children };
            appendInline(parent, node);
            current = parent;
          }
        } else {
          current.pendingText += marker;
        }
      } else {
        flushText(current);
        current = {
          marker,
          parent: current,
          children: [],
          pendingText: "",
        };
        active.set(marker, current);
      }
      index += marker.length;
      continue;
    }

    current.pendingText += character ?? "";
    index += 1;
  }

  while (current !== root) {
    const parent = current.parent;
    if (parent === null) break;
    appendLiteralFrame(current, parent, active);
    current = parent;
  }
  flushText(root);
  return root.children;
}

export function parseInline(source: string): Inline[] {
  return parseInlineInternal(source);
}

function isBlank(line: string): boolean {
  for (const character of line) {
    if (character !== " " && character !== "\t") return false;
  }
  return true;
}

function readHeading(
  line: string,
): { level: 1 | 2 | 3 | 4 | 5 | 6; content: string } | undefined {
  let markerLength = 0;
  while (line[markerLength] === "#") markerLength += 1;
  const levels = [1, 2, 3, 4, 5, 6] as const;
  const level = levels[markerLength - 1];
  if (
    level === undefined ||
    (line[markerLength] !== " " && line[markerLength] !== "\t")
  ) {
    return undefined;
  }
  let contentStart = markerLength;
  while (line[contentStart] === " " || line[contentStart] === "\t") {
    contentStart += 1;
  }
  return { level, content: line.slice(contentStart) };
}

function readFence(line: string): Fence | undefined {
  let start = 0;
  while (start < 3 && line[start] === " ") start += 1;
  const character = line[start];
  if (character !== "`" && character !== "~") return undefined;
  let end = start;
  while (line[end] === character) end += 1;
  const length = end - start;
  if (length < 3) return undefined;
  return { character, length, lang: line.slice(end).trim() };
}

function isClosingFence(line: string, fence: Fence): boolean {
  let start = 0;
  while (start < 4 && line[start] === " ") start += 1;
  if (start > 3 || line[start] !== fence.character) return false;
  let end = start;
  while (line[end] === fence.character) end += 1;
  if (end - start < fence.length) return false;
  while (line[end] === " " || line[end] === "\t") end += 1;
  return end === line.length;
}

function readListMarker(line: string): ListMarker | undefined {
  const first = line[0];
  if ((first === "-" || first === "*" || first === "+") && line[1] === " ") {
    return { ordered: false, start: 1, contentStart: 2 };
  }
  if (first === undefined || first < "0" || first > "9") return undefined;
  let end = 0;
  while (line.charAt(end) >= "0" && line.charAt(end) <= "9") {
    end += 1;
  }
  const delimiter = line[end];
  if ((delimiter !== "." && delimiter !== ")") || line[end + 1] !== " ") {
    return undefined;
  }
  return {
    ordered: true,
    start: Number(line.slice(0, end)),
    contentStart: end + 2,
  };
}

function continuationText(line: string): string | undefined {
  let indent = 0;
  while (line[indent] === " ") indent += 1;
  return indent < 2 ? undefined : line.slice(2);
}

function isRule(line: string): boolean {
  let index = 0;
  while (line[index] === " " || line[index] === "\t") index += 1;
  const marker = line[index];
  if (marker !== "-" && marker !== "*" && marker !== "_") return false;
  let count = 0;
  for (; index < line.length; index += 1) {
    const character = line[index];
    if (character === marker) count += 1;
    else if (character !== " " && character !== "\t") return false;
  }
  return count >= 3;
}

function isBlockStart(line: string): boolean {
  return (
    readFence(line) !== undefined ||
    readHeading(line) !== undefined ||
    readListMarker(line) !== undefined ||
    line.startsWith(">") ||
    isRule(line)
  );
}

export function parseMarkdown(source: string): Block[] {
  const lines = source.split(/\r\n|\n|\r/);
  const blocks: Block[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (line === undefined) break;
    if (isBlank(line)) {
      index += 1;
      continue;
    }

    const fence = readFence(line);
    if (fence !== undefined) {
      index += 1;
      const body: string[] = [];
      while (index < lines.length) {
        const codeLine = lines[index];
        if (codeLine === undefined || isClosingFence(codeLine, fence)) break;
        body.push(codeLine);
        index += 1;
      }
      if (index < lines.length && lines[index] !== undefined) index += 1;
      blocks.push({ kind: "code", lang: fence.lang, text: body.join("\n") });
      continue;
    }

    const heading = readHeading(line);
    if (heading !== undefined) {
      blocks.push({
        kind: "heading",
        level: heading.level,
        children: parseInline(heading.content),
      });
      index += 1;
      continue;
    }

    if (isRule(line)) {
      blocks.push({ kind: "rule" });
      index += 1;
      continue;
    }

    const marker = readListMarker(line);
    if (marker !== undefined) {
      const ordered = marker.ordered;
      const start = marker.start;
      let currentItem = line.slice(marker.contentStart);
      const items: Inline[][] = [];
      index += 1;
      while (index < lines.length) {
        const nextLine = lines[index];
        if (nextLine === undefined || isBlank(nextLine)) break;
        const nextMarker = readListMarker(nextLine);
        if (nextMarker?.ordered === ordered) {
          items.push(parseInline(currentItem));
          currentItem = nextLine.slice(nextMarker.contentStart);
          index += 1;
          continue;
        }
        const continuation = continuationText(nextLine);
        if (continuation !== undefined) {
          currentItem += `\n${continuation}`;
          index += 1;
          continue;
        }
        break;
      }
      items.push(parseInline(currentItem));
      blocks.push({ kind: "list", ordered, start, items });
      continue;
    }

    if (line.startsWith(">")) {
      const quoteLines: string[] = [];
      while (index < lines.length) {
        const quoteLine = lines[index];
        if (!quoteLine?.startsWith(">")) break;
        quoteLines.push(
          quoteLine[1] === " " ? quoteLine.slice(2) : quoteLine.slice(1),
        );
        index += 1;
      }
      blocks.push({
        kind: "quote",
        children: parseMarkdown(quoteLines.join("\n")),
      });
      continue;
    }

    const paragraph = [line];
    index += 1;
    while (index < lines.length) {
      const nextLine = lines[index];
      if (nextLine === undefined || isBlank(nextLine) || isBlockStart(nextLine))
        break;
      paragraph.push(nextLine);
      index += 1;
    }
    blocks.push({
      kind: "paragraph",
      children: parseInline(paragraph.join("\n")),
    });
  }
  return blocks;
}
