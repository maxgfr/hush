import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { go, openLink } from "../../app/state";
import { Icon } from "../components/Icons";
import { QrScanner } from "../components/QrScanner";
import { type Key, t } from "../i18n";

export function Join(props: { error?: string }): JSX.Element {
  const [value, setValue] = useState("");
  const [scan, setScan] = useState(false);
  const submit = (e: Event) => {
    e.preventDefault();
    if (value.trim()) openLink(value.trim());
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
      {scan ? (
        <div class="card">
          <QrScanner
            onResult={(text) => {
              setScan(false);
              openLink(text);
            }}
            onCancel={() => setScan(false)}
          />
        </div>
      ) : (
        <form class="card stack" onSubmit={submit}>
          <label class="field">
            <span>{t("pasteLink")}</span>
            <textarea
              rows={3}
              value={value}
              spellcheck={false}
              autocomplete="off"
              placeholder="https://maxgfr.github.io/hush/#1.s.…"
              onInput={(e) => setValue((e.target as HTMLTextAreaElement).value)}
            />
          </label>
          {props.error && <p class="error">{t(props.error as Key)}</p>}
          <div class="row">
            <button type="submit" class="btn btn-primary" disabled={!value.trim()}>
              {t("join")}
            </button>
            <button type="button" class="btn" onClick={() => setScan(true)}>
              <Icon name="scan" />
              {t("scanQr")}
            </button>
          </div>
        </form>
      )}
    </main>
  );
}
