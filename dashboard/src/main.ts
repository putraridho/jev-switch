import "@fontsource/dm-sans/latin-400.css";
import "@fontsource/dm-sans/latin-500.css";
import "@fontsource/dm-sans/latin-600.css";
import "@fontsource/instrument-serif/latin-400.css";
import "@fontsource/dm-mono/latin-400.css";
import "./lib/boot";
import "./lib/design/tokens.css";
import { createTauriClient } from "./lib/ipc/tauri";
import { initStores } from "./lib/stores";
import { mount } from "svelte";
import App from "./App.svelte";

const client = createTauriClient();
void initStores(client);
mount(App, { target: document.body });
