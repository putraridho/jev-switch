import { describe, expect, it } from "vitest";
import { parseInline, parseMarkdown } from "./markdown";

describe("parseMarkdown", () => {
  it("parses paragraphs and all heading levels", () => {
    expect(
      parseMarkdown(
        "first line\nsecond line\n\n# one\n## two\n### three\n#### four\n##### five\n###### six",
      ),
    ).toEqual([
      {
        kind: "paragraph",
        children: [{ kind: "text", text: "first line\nsecond line" }],
      },
      { kind: "heading", level: 1, children: [{ kind: "text", text: "one" }] },
      { kind: "heading", level: 2, children: [{ kind: "text", text: "two" }] },
      {
        kind: "heading",
        level: 3,
        children: [{ kind: "text", text: "three" }],
      },
      { kind: "heading", level: 4, children: [{ kind: "text", text: "four" }] },
      { kind: "heading", level: 5, children: [{ kind: "text", text: "five" }] },
      { kind: "heading", level: 6, children: [{ kind: "text", text: "six" }] },
    ]);
  });

  it("parses backtick and tilde fences, including an unclosed streaming fence", () => {
    expect(
      parseMarkdown(
        "```ts extra\nconst x = 1;\n```\n~~~\nplain\n~~~\n```js\npartial",
      ),
    ).toEqual([
      { kind: "code", lang: "ts extra", text: "const x = 1;" },
      { kind: "code", lang: "", text: "plain" },
      { kind: "code", lang: "js", text: "partial" },
    ]);
  });

  it("groups list items and appends indented continuations", () => {
    expect(
      parseMarkdown(
        "- first\n  continued\n* **bold** item\n+ last\n\n3) start\n4. next",
      ),
    ).toEqual([
      {
        kind: "list",
        ordered: false,
        start: 1,
        items: [
          [{ kind: "text", text: "first\ncontinued" }],
          [
            { kind: "strong", children: [{ kind: "text", text: "bold" }] },
            { kind: "text", text: " item" },
          ],
          [{ kind: "text", text: "last" }],
        ],
      },
      {
        kind: "list",
        ordered: true,
        start: 3,
        items: [
          [{ kind: "text", text: "start" }],
          [{ kind: "text", text: "next" }],
        ],
      },
    ]);
  });

  it("parses quote contents recursively", () => {
    expect(parseMarkdown("> # title\n> quoted\n> > nested")).toEqual([
      {
        kind: "quote",
        children: [
          {
            kind: "heading",
            level: 1,
            children: [{ kind: "text", text: "title" }],
          },
          { kind: "paragraph", children: [{ kind: "text", text: "quoted" }] },
          {
            kind: "quote",
            children: [
              {
                kind: "paragraph",
                children: [{ kind: "text", text: "nested" }],
              },
            ],
          },
        ],
      },
    ]);
  });

  it("recognizes rules with spaces", () => {
    expect(parseMarkdown("---\n* * *\n_  _ _")).toEqual([
      { kind: "rule" },
      { kind: "rule" },
      { kind: "rule" },
    ]);
  });

  it("parses inline kinds, nested formatting, links, and code spans", () => {
    expect(
      parseInline(
        "plain **bold `code`** __strong__ *em* _also_ [*label*](https://example.test/a) `one` ``two`ticks``",
      ),
    ).toEqual([
      { kind: "text", text: "plain " },
      {
        kind: "strong",
        children: [
          { kind: "text", text: "bold " },
          { kind: "code", text: "code" },
        ],
      },
      { kind: "text", text: " " },
      { kind: "strong", children: [{ kind: "text", text: "strong" }] },
      { kind: "text", text: " " },
      { kind: "em", children: [{ kind: "text", text: "em" }] },
      { kind: "text", text: " " },
      { kind: "em", children: [{ kind: "text", text: "also" }] },
      { kind: "text", text: " " },
      {
        kind: "link",
        children: [{ kind: "em", children: [{ kind: "text", text: "label" }] }],
        href: "https://example.test/a",
      },
      { kind: "text", text: " " },
      { kind: "code", text: "one" },
      { kind: "text", text: " " },
      { kind: "code", text: "two`ticks" },
    ]);
  });

  it("leaves underscores inside words literal", () => {
    expect(parseInline("snake_case_name and _emphasis_")).toEqual([
      { kind: "text", text: "snake_case_name and " },
      { kind: "em", children: [{ kind: "text", text: "emphasis" }] },
    ]);
  });

  it("unescapes punctuation and keeps unmatched markers literal", () => {
    expect(
      parseInline("\\*literal\\* \\[label\\] \\! **open *also _open"),
    ).toEqual([
      { kind: "text", text: "*literal* [label] ! **open *also _open" },
    ]);
  });

  it("normalizes CRLF while keeping paragraph newlines", () => {
    expect(parseMarkdown("first\r\nsecond\r\n\r\n> quote\r\n> next")).toEqual([
      {
        kind: "paragraph",
        children: [{ kind: "text", text: "first\nsecond" }],
      },
      {
        kind: "quote",
        children: [
          {
            kind: "paragraph",
            children: [{ kind: "text", text: "quote\nnext" }],
          },
        ],
      },
    ]);
  });

  it("parses a 100 KB run of unmatched link openers quickly", () => {
    const source = "[".repeat(100_000);
    const started = performance.now();
    const result = parseMarkdown(source);
    expect(performance.now() - started).toBeLessThan(2_000);
    expect(result).toEqual([
      { kind: "paragraph", children: [{ kind: "text", text: source }] },
    ]);
  });
});
