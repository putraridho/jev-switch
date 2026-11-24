import "./lib/boot";
import "./lib/design/tokens.css";
import { mount } from "svelte";
import App from "./App.svelte";

mount(App, { target: document.body });
