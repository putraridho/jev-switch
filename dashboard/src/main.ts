import "./lib/design/fonts.css";
import "./lib/boot";
import "./lib/design/tokens.css";
import "./lib/design/base.css";
import { createTauriClient } from "./lib/ipc/tauri";
import { initStores } from "./lib/stores";
import { mount } from "svelte";
import App from "./App.svelte";

const client = createTauriClient();
void initStores(client);
mount(App, { target: document.body });
