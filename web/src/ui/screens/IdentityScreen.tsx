import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { saveSettings, settings } from "../../app/settings";
import { contactsManager, go } from "../../app/state";
import { ensureIdentity, identity, vaultState } from "../../app/store";
import { Icon } from "../components/Icons";
import { QrCode } from "../components/QrCode";
import { VaultGate } from "../components/VaultGate";
import { t } from "../i18n";

export function IdentityScreen(): JSX.Element {
  const me = identity.value;
  const book = contactsManager();
  const [copied, setCopied] = useState(false);
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
        <h1 class="title">{t("myId")}</h1>
        <span />
      </header>
      {vaultState.value !== "unlocked" ? (
        <VaultGate />
      ) : !me ? (
        <section class="card stack">
          <p class="hint">{t("idIntro")}</p>
          <button
            type="button"
            class="btn btn-primary btn-large"
            onClick={() => void ensureIdentity(settings.value.name)}
          >
            <Icon name="key" />
            {t("idCreate")}
          </button>
        </section>
      ) : (
        <>
          <section class="card stack center">
            <p class="hint">{t("idIntro")}</p>
            <div class="qr-wrap">
              <QrCode value={me.id} label={t("idTitle")} size={200} />
            </div>
            <code class="id big" data-testid="my-id">
              {me.id}
            </code>
            <button
              type="button"
              class="btn"
              onClick={() =>
                void navigator.clipboard.writeText(me.id).then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                })
              }
            >
              <Icon name={copied ? "check" : "copy"} />
              {copied ? t("copied") : t("copy")}
            </button>
          </section>
          <section class="card stack">
            <label class="check">
              <input
                type="checkbox"
                checked={book.reachable.value}
                data-testid="reachable"
                onChange={(e) => book.setReachable((e.target as HTMLInputElement).checked)}
              />
              <span>
                {t("reachable")}
                <small class="hint">{t("reachableHint")}</small>
              </span>
            </label>
          </section>
        </>
      )}
      <section class="card stack">
        <label class="field">
          <span>{t("displayName")}</span>
          <input
            type="text"
            maxLength={64}
            value={settings.value.name}
            data-testid="name"
            onInput={(e) => saveSettings({ name: (e.target as HTMLInputElement).value })}
          />
          <small class="hint">{t("displayNameHint")}</small>
        </label>
      </section>
    </main>
  );
}
