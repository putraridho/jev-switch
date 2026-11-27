<script lang="ts">
  import { tick, untrack } from "svelte";
  import Header from "./lib/components/shell/Header.svelte";
  import FirstRun from "./lib/components/shell/FirstRun.svelte";
  import LiveRegion from "./lib/components/shell/LiveRegion.svelte";
  import { providerName, providerOrder } from "./lib/copy";
  import { closeTopmost } from "./lib/escape";
  import { refreshAll } from "./lib/actions";
  import { router } from "./lib/router.svelte";
  import { handleKey, zoomNext, type KeyEnv } from "./lib/shortcuts";
  import { settings } from "./lib/stores/settings.svelte";
  import { usage } from "./lib/stores/usage.svelte";
  import Overview from "./screens/Overview.svelte";
  import Chat from "./screens/Chat.svelte";
  import ProviderDetail from "./screens/ProviderDetail.svelte";
  import Settings from "./screens/Settings.svelte";

  const env: KeyEnv = {
    shortcutsOn: () => settings.value.shortcuts,
    hidden: () => settings.value.hidden_providers,
    order: () => providerOrder,
    closeTopmost,
    canGoBack: () => router.canGoBack,
    back: () => {
      router.back();
    },
    refresh: () => {
      void refreshAll();
    },
    openSettings: (target) => {
      if (router.route.name !== "settings")
        router.go({ name: "settings" }, target);
      else void focusById(target);
    },
    openProvider: (provider) => {
      router.go({ name: "provider", provider });
    },
    zoom: (step) => {
      void settings.update({
        zoom_pct: zoomNext(settings.value.zoom_pct, step),
      });
    },
  };

  function onkeydown(event: KeyboardEvent): void {
    if (handleKey(event, env)) event.preventDefault();
  }

  async function focusById(id: string | undefined): Promise<void> {
    await tick();
    if (id === undefined) return;
    const target = document.getElementById(id);
    if (target instanceof HTMLElement) target.focus();
  }

  // One document title per view (WCAG 2.4.2), and focus follows navigation (2.4.3): to the
  // control that opened the view when going back, to a named section when asked, else to
  // the view title.
  let first = true;
  $effect(() => {
    const route = router.route;
    document.title =
      route.name === "provider"
        ? `Flashboard: ${providerName[route.provider]}`
        : route.name === "chat"
          ? "Flashboard: Chat"
          : route.name === "settings"
            ? "Flashboard: Settings"
            : "Flashboard: Overview";
    if (first) {
      first = false;
      return;
    }
    void tick().then(() => {
      untrack(() => {
        const trigger = router.takeReturnFocus();
        if (trigger?.isConnected === true) {
          trigger.focus();
          return;
        }
        const section = router.takeSection();
        const target = document.getElementById(section ?? "view-title");
        if (target instanceof HTMLElement) target.focus();
      });
    });
  });
</script>

<svelte:window {onkeydown} />

<Header />
<main>
  {#if router.route.name === "overview" && settings.loaded && !settings.value.first_run_seen && usage.snapshot !== null}
    <FirstRun />
  {:else if router.route.name === "overview"}
    <Overview />
  {:else if router.route.name === "chat"}
    <Chat />
  {:else if router.route.name === "provider"}
    <ProviderDetail />
  {:else}
    <Settings />
  {/if}
</main>
<LiveRegion />

<style>
  main {
    min-height: calc(100vh - var(--space-12) - var(--rule));
    background: var(--color-bg);
  }
</style>
