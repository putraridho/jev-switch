import "./lib/design/fonts.css";
import "./lib/boot";
import "./lib/design/tokens.css";
import "./lib/design/base.css";
import { createClient } from "./lib/ipc/select";
import { initStores } from "./lib/stores";
import { mount } from "svelte";
import App from "./App.svelte";

void initStores(createClient());
mount(App, { target: document.body });
