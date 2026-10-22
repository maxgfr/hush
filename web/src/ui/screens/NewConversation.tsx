import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { settings } from "../../app/settings";
import { go, startConversation } from "../../app/state";
import { Icon } from "../components/Icons";
import { duration, LINK_MINUTES, TTL_SECONDS, t, ttlLabel } from "../i18n";

export function NewConversation(): JSX.Element {
  const [group, setGroup] = useState(false);
  const [ttlMin, setTtlMin] = useState(60);
  const [pass, setPass] = useState("");
  const [ttl, setTtl] = useState(settings.value.ttl);
  const [busy, setBusy] = useState(false);

  const create = (e: Event) => {
    e.preventDefault();
    setBusy(true);
    // Let the spinner paint: a passphrase means a second of Argon2id.
    setTimeout(
      () => startConversation({ group, ttlMin, ttl, ...(pass ? { passphrase: pass } : {}) }),
      30,
    );
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
        <h1 class="title">{t("newConversation")}</h1>
        <span />
      </header>
      <form class="card stack" onSubmit={create}>
        <fieldset class="segmented" aria-label={t("newConversation")}>
          <label class={!group ? "on" : ""}>
            <input type="radio" name="kind" checked={!group} onChange={() => setGroup(false)} />
            <Icon name="user" size={18} />
            {t("kindDirect")}
          </label>
          <label class={group ? "on" : ""}>
            <input type="radio" name="kind" checked={group} onChange={() => setGroup(true)} />
            <Icon name="users" size={18} />
            {t("kindGroup")}
          </label>
        </fieldset>
        <p class="hint">{group ? t("kindGroupHint") : t("kindDirectHint")}</p>

        <label class="field">
          <span>{t("linkExpiry")}</span>
          <select
            value={ttlMin}
            onChange={(e) => setTtlMin(Number((e.target as HTMLSelectElement).value))}
          >
            {LINK_MINUTES.map((m) => (
              <option key={m} value={m}>
                {duration(m * 60_000)}
              </option>
            ))}
          </select>
        </label>

        <label class="field">
          <span>{t("messageTimer")}</span>
          <select
            value={ttl}
            onChange={(e) => setTtl(Number((e.target as HTMLSelectElement).value))}
          >
            {TTL_SECONDS.map((s) => (
              <option key={s} value={s}>
                {ttlLabel(s)}
              </option>
            ))}
          </select>
        </label>

        <label class="field">
          <span>{t("passphrase")}</span>
          <input
            type="password"
            autocomplete="new-password"
            value={pass}
            onInput={(e) => setPass((e.target as HTMLInputElement).value)}
          />
          <small class="hint">{t("passphraseHint")}</small>
        </label>

        <button type="submit" class="btn btn-primary btn-large" disabled={busy} aria-busy={busy}>
          {busy && pass ? t("deriving") : t("createLink")}
        </button>
      </form>
    </main>
  );
}
