import "@fontsource/dm-sans/latin-400.css";
import "@fontsource/dm-sans/latin-500.css";
import "@fontsource/dm-sans/latin-600.css";
import "@fontsource/instrument-serif/latin-400.css";
import "@fontsource/dm-mono/latin-400.css";
import "./lib/boot";
import "./lib/design/tokens.css";
import { mount } from "svelte";
import App from "./App.svelte";

mount(App, { target: document.body });
