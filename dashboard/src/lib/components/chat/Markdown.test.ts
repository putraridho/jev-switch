import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import Markdown from "./Markdown.svelte";

afterEach(() => {
  cleanup();
});

describe("Markdown", () => {
  it("renders standard blocks and inline marks without stray spaces", () => {
    const view = render(Markdown, {
      text: "## Plan\n\nRun **cargo** `test` now, *quickly*.\n\n- one\n- two\n\n1. first\n\n```sh\nls -la\n```\n\n> quoted\n\n---\n\nSee [docs](https://example.test).",
    });
    expect(
      view.container.querySelector(".md-heading")?.textContent.trim(),
    ).toBe("Plan");
    expect(view.container.querySelector(".md-p")?.textContent).toBe(
      "Run cargo test now, quickly.",
    );
    expect(view.container.querySelector("strong")?.textContent).toBe("cargo");
    expect(view.container.querySelector("em")?.textContent).toBe("quickly");
    expect(
      view.container.querySelector("code.md-inline-code")?.textContent,
    ).toBe("test");
    expect(view.container.querySelectorAll("ul li")).toHaveLength(2);
    expect(view.container.querySelectorAll("ol li")).toHaveLength(1);
    expect(view.container.querySelector("pre.md-code")?.textContent).toBe(
      "ls -la",
    );
    expect(view.container.querySelector("blockquote")?.textContent.trim()).toBe(
      "quoted",
    );
    expect(view.container.querySelector("hr")).not.toBeNull();
    // A link is shown with its address and is never a navigable anchor.
    expect(view.container.querySelector("a")).toBeNull();
    expect(view.container.textContent).toContain("docs (https://example.test)");
  });

  it("never turns text into markup", () => {
    const view = render(Markdown, {
      text: "<img src=x onerror=alert(1)> **<b>x</b>**",
    });
    expect(view.container.querySelector("img")).toBeNull();
    expect(view.container.querySelector("b")).toBeNull();
    expect(view.container.textContent).toContain(
      "<img src=x onerror=alert(1)>",
    );
  });
});
