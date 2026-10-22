import type { JSX } from "preact";
import { go } from "../../app/state";
import { contacts, vaultState } from "../../app/store";
import { Icon, Logo } from "../components/Icons";
import { lang, setLang, t } from "../i18n";

export function Home(): JSX.Element {
  const n = contacts.value.length;
  return (
    <main class="page home">
      <header class="topbar">
        <div class="brand">
          <Logo size={30} />
          <span class="wordmark">hush</span>
        </div>
        <button
          type="button"
          class="btn btn-ghost btn-small"
          onClick={() => setLang(lang.value === "fr" ? "en" : "fr")}
          aria-label={t("language")}
        >
          {lang.value === "fr" ? "EN" : "FR"}
        </button>
      </header>

      <section class="hero">
        <h1>{t("tagline")}</h1>
        <p class="lead">{t("intro")}</p>
        <div class="actions">
          <button
            type="button"
            class="btn btn-primary btn-large"
            onClick={() => go({ name: "new" })}
          >
            <Icon name="link" />
            {t("newConversation")}
          </button>
          <button type="button" class="btn btn-large" onClick={() => go({ name: "join" })}>
            <Icon name="scan" />
            {t("joinWithLink")}
          </button>
        </div>
      </section>

      <nav class="tiles" aria-label="hush">
        <button type="button" class="tile" onClick={() => go({ name: "contacts" })}>
          <Icon name="users" />
          <span>{t("contacts")}</span>
          {vaultState.value === "unlocked" && n > 0 && <span class="count">{n}</span>}
          {vaultState.value === "locked" && <Icon name="lock" size={16} class="faint" />}
        </button>
        <button type="button" class="tile" onClick={() => go({ name: "identity" })}>
          <Icon name="key" />
          <span>{t("myId")}</span>
        </button>
        <button type="button" class="tile" onClick={() => go({ name: "settings" })}>
          <Icon name="settings" />
          <span>{t("settings")}</span>
        </button>
      </nav>

      <ul class="facts">
        <li>
          <Icon name="shield" size={18} />
          <span>X25519 + ML-KEM-768 · Ed25519 + ML-DSA-65</span>
        </li>
        <li>
          <Icon name="globe" size={18} />
          <span>
            {lang.value === "fr"
              ? "Aucun serveur, aucun compte, aucune donnée gardée"
              : "No server, no account, nothing kept"}
          </span>
        </li>
      </ul>

      <footer class="footer">
        <a
          href="https://github.com/maxgfr/hush/blob/main/SECURITY.md"
          rel="noopener noreferrer"
          target="_blank"
        >
          {t("security")}
        </a>
        <span aria-hidden="true">·</span>
        <a href="https://github.com/maxgfr/hush" rel="noopener noreferrer" target="_blank">
          {t("source")}
        </a>
      </footer>
    </main>
  );
}
