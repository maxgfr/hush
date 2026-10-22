import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { go, joinLink } from "../../app/state";
import { Icon } from "../components/Icons";
import { t } from "../i18n";

export function Passphrase(props: { fragment: string; error?: string }): JSX.Element {
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = (e: Event) => {
    e.preventDefault();
    if (!pass) return;
    setBusy(true);
    setTimeout(() => joinLink(props.fragment, pass), 30);
  };
  return (
    <main class="page narrow">
      <header class="topbar">
        <button
          type="button"
          class="btn btn-ghost btn-icon"
          onClick={() => go({ name: "home" })}
          aria-label={t("back")}
        >
          <Icon name="arrowLeft" />
        </button>
        <h1 class="title">{t("joinWithLink")}</h1>
        <span />
      </header>
      <form class="card stack" onSubmit={submit}>
        <div class="icon-title">
          <Icon name="lock" size={28} />
          <h2>{t("protectedLink")}</h2>
        </div>
        <p class="hint">{t("protectedLinkHint")}</p>
        <label class="field">
          <span>{t("passphrase").replace(/ \(.*\)$/, "")}</span>
          <input
            type="password"
            autocomplete="off"
            value={pass}
            onInput={(e) => setPass((e.target as HTMLInputElement).value)}
          />
        </label>
        <button
          type="submit"
          class="btn btn-primary btn-large"
          disabled={!pass || busy}
          aria-busy={busy}
        >
          {busy ? t("deriving") : t("join")}
        </button>
      </form>
    </main>
  );
}
