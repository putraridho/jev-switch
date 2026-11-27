<!-- Minimal standard Markdown for chat replies: paragraphs, headings, fenced code, lists,
     quotes, rules, inline code, bold, italic and links. Rendered from a typed tree, never
     from HTML (the CSP and the lint rules forbid {@html}). A link shows its address and is
     not clickable: a click would navigate the app's own webview. -->
<script lang="ts">
  import { parseMarkdown, type Block, type Inline } from "../../markdown";

  let { text }: { text: string } = $props();

  const blocks = $derived(parseMarkdown(text));

  function plain(nodes: readonly Inline[]): string {
    return nodes
      .map((n) =>
        n.kind === "text" || n.kind === "code" ? n.text : plain(n.children),
      )
      .join("");
  }
</script>

{#snippet inlines(
  nodes: readonly Inline[],
)}{#each nodes as node, i (i)}{#if node.kind === "text"}{node.text}{:else if node.kind === "code"}<code
        class="md-inline-code">{node.text}</code
      >{:else if node.kind === "strong"}<strong
        >{@render inlines(node.children)}</strong
      >{:else if node.kind === "em"}<em>{@render inlines(node.children)}</em
      >{:else}<span class="md-link">{@render inlines(node.children)}</span
      >{#if plain(node.children) !== node.href}<span class="md-href"
          >{` (${node.href})`}</span
        >{/if}{/if}{/each}{/snippet}

{#snippet blockList(items: readonly Block[])}
  {#each items as block, i (i)}
    {#if block.kind === "paragraph"}
      <p class="md-p">{@render inlines(block.children)}</p>
    {:else if block.kind === "heading"}
      <p class="md-heading" data-level={block.level}>
        {@render inlines(block.children)}
      </p>
    {:else if block.kind === "code"}
      <pre class="md-code" data-lang={block.lang}><code>{block.text}</code
        ></pre>
    {:else if block.kind === "list"}
      {#if block.ordered}
        <ol class="md-list" start={block.start}>
          {#each block.items as item, j (j)}
            <li>{@render inlines(item)}</li>
          {/each}
        </ol>
      {:else}
        <ul class="md-list">
          {#each block.items as item, j (j)}
            <li>{@render inlines(item)}</li>
          {/each}
        </ul>
      {/if}
    {:else if block.kind === "quote"}
      <blockquote class="md-quote">
        {@render blockList(block.children)}
      </blockquote>
    {:else}
      <hr class="md-rule" />
    {/if}
  {/each}
{/snippet}

<div class="md-root">{@render blockList(blocks)}</div>

<style>
  .md-root {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .md-p,
  .md-heading,
  .md-list li {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .md-heading {
    font-weight: var(--text-md-weight-strong);
  }
  .md-heading[data-level="1"],
  .md-heading[data-level="2"] {
    font-size: var(--text-title);
  }
  .md-list {
    margin: 0;
    padding-inline-start: var(--space-6);
  }
  .md-inline-code,
  .md-code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--color-chip);
    border-radius: var(--radius-small);
  }
  .md-inline-code {
    padding: 0 var(--space-1);
  }
  .md-code {
    margin: 0;
    padding: var(--space-2);
    overflow-x: auto;
    white-space: pre;
  }
  .md-quote {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-inline-start: var(--space-3);
    border-left: var(--focus-ring) solid var(--color-rule);
    color: var(--color-muted);
  }
  .md-link {
    text-decoration: underline;
  }
  .md-href {
    color: var(--color-muted);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }
  .md-rule {
    width: 100%;
    border: 0;
    border-top: var(--rule) solid var(--color-rule);
  }
</style>
