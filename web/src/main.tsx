import { render } from "preact";
import { boot } from "./app/state";
import { frameBust } from "./security";
import { App } from "./ui/App";
import { lang } from "./ui/i18n";
import { registerServiceWorker } from "./ui/pwa";

if (!frameBust()) {
  document.documentElement.lang = lang.value;
  const root = document.getElementById("app");
  if (root) render(<App />, root);
  registerServiceWorker();
  void boot();
}
