<!-- A dialog (S39): the action named in the primary, focus on Cancel. With
     `cancelLabel={null}` it is the info dialog: one primary button. Mount it to open it;
     the caller removes it and restores focus. -->
<script lang="ts">
  import { pushEscape } from "../../escape";

  interface Props {
    title: string;
    body: string;
    /** The primary button: it names the action. */
    confirmLabel: string;
    cancelLabel?: string | null;
    onconfirm: () => void;
    oncancel: () => void;
  }

  const {
    title,
    body,
    confirmLabel,
    cancelLabel = "Cancel",
    onconfirm,
    oncancel,
  }: Props = $props();

  const uid = $props.id();
  let dialog = $state<HTMLDialogElement | null>(null);
  let cancel = $state<HTMLButtonElement | null>(null);
  let primary = $state<HTMLButtonElement | null>(null);

  $effect(() => {
    if (dialog === null) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
    (cancel ?? primary)?.focus();
    // `Esc` goes through the app's one order of precedence.
    return pushEscape("dialog", () => {
      oncancel();
      return true;
    });
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="{uid}-title"
  aria-describedby="{uid}-body"
  oncancel={(event) => {
    event.preventDefault();
  }}
>
  <h2 id="{uid}-title">{title}</h2>
  <p id="{uid}-body">{body}</p>
  <div class="actions">
    {#if cancelLabel !== null}<button
        type="button"
        bind:this={cancel}
        onclick={oncancel}>{cancelLabel}</button
      >{/if}
    <button
      type="button"
      class="primary"
      bind:this={primary}
      onclick={onconfirm}>{confirmLabel}</button
    >
  </div>
</dialog>

<style>
  dialog {
    width: min(var(--dialog-w), calc(100vw - 2 * var(--space-4)));
    padding: var(--space-6);
    color: var(--color-text);
    background: var(--color-paper);
    border: var(--outline) solid var(--color-border-strong);
    border-radius: var(--radius-dialog);
  }

  dialog::backdrop {
    background: var(--color-scrim);
  }

  h2 {
    margin: 0 0 var(--space-3);
    font-family: var(--font-serif);
    font-size: var(--text-title);
    font-weight: var(--text-title-weight);
    line-height: var(--text-title-line);
  }

  p {
    font-size: var(--text-md);
    line-height: var(--text-md-line);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-6);
  }
</style>
