import "../lib/design/fonts.css";
import "../lib/design/tokens.css";
import "../lib/design/base.css";
import { mount } from "svelte";
import Gallery from "./Gallery.svelte";

const theme = new URLSearchParams(location.search).get("theme");
if (theme === "light" || theme === "dark") {
  document.documentElement.dataset["theme"] = theme;
}
mount(Gallery, { target: document.body });
