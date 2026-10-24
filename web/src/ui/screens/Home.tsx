import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { go } from "../../app/state";
import { contacts, vaultState } from "../../app/store";
import { entriesFromDrop } from "../../drop/entries";
import { Icon, Logo } from "../components/Icons";
import { lang, setLang, t } from "../i18n";

export function Home(): JSX.Element {
  const n = contacts.value.length;
  const [over, setOver] = useState(false);
  return (
    // Files dropped anywhere on the page start a drop.
    <main
      class={`page home${over ? " drag-over" : ""}`}
      onDragOver={(e) => {
        if (!e.dataTransfer?.types.includes("Files")) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={(e) => {
        if (e.target === e.currentTarget) setOver(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        if (e.dataTransfer)
          void entriesFromDrop(e.dataTransfer).then((entries) => go({ name: "drop", entries }));
      }}
    >
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
          <button
            type="button"
            class="btn btn-large"
            data-testid="home-send-files"
            onClick={() => go({ name: "drop" })}
          >
            <Icon name="upload" />
            {t("sendFiles")}
          </button>
          <button type="button" class="btn btn-large" onClick={() => go({ name: "join" })}>
            <Icon name="scan" />
            {t("joinWithLink")}
          </button>
        </div>
      </section>

      <p class="drop-hint faint">
        <Icon name="upload" size={16} />
        {over ? t("dropToSend") : t("sendFilesLead")}
      </p>

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
          <span>{t("noServer")}</span>
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
        {typeof __APP_VERSION__ === "string" && (
          <>
            <span aria-hidden="true">·</span>
            <a
              href={`https://github.com/maxgfr/hush/releases/tag/cli-v${__APP_VERSION__}`}
              rel="noopener noreferrer"
              target="_blank"
              data-testid="version"
            >
              v{__APP_VERSION__}
            </a>
          </>
        )}
      </footer>
    </main>
  );
}
