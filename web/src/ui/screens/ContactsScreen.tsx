import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { contactLabel } from "../../app/contacts";
import { contactsManager, go, myName, showChat } from "../../app/state";
import {
  type Contact,
  contacts,
  eraseVault,
  identity,
  lockVault,
  removeContact,
  saveContact,
  vaultState,
} from "../../app/store";
import { BackupPanel } from "../components/BackupPanel";
import { Icon } from "../components/Icons";
import { VaultGate } from "../components/VaultGate";
import { t } from "../i18n";

function ContactRow(props: { c: Contact }): JSX.Element {
  const book = contactsManager();
  const { c } = props;
  const chat = book.chats.value.get(c.key);
  const online = !!chat?.connected.value || book.online.value.has(c.key);
  const unread = chat?.unread.value ?? 0;
  return (
    <li class="contact">
      <button
        type="button"
        class="contact-main"
        onClick={() => {
          const live = book.chatWith(c.key);
          if (live) showChat(live);
        }}
        disabled={!chat?.connected.value}
      >
        <span class={online ? "dot on" : "dot"} aria-hidden="true" />
        <span class="contact-name">{contactLabel(c)}</span>
        <span class="faint">{online ? t("online") : t("offlineContact")}</span>
        {unread > 0 && <span class="count">{unread}</span>}
      </button>
      <details class="contact-more">
        <summary aria-label="…">
          <Icon name="settings" size={16} />
        </summary>
        <div class="stack">
          <code class="id">{c.id}</code>
          <label class="check">
            <input
              type="checkbox"
              checked={c.history}
              onChange={(e) =>
                void saveContact({ ...c, history: (e.target as HTMLInputElement).checked })
              }
            />
            <span>
              {t("history")}
              <small class="hint">{t("historyHint")}</small>
            </span>
          </label>
          <button
            type="button"
            class="btn btn-small btn-danger-ghost"
            onClick={() => {
              if (confirm(t("removeContactConfirm", { name: contactLabel(c) }))) {
                book.chats.value.get(c.key)?.end();
                void removeContact(c.key).then(() => book.watch());
              }
            }}
          >
            <Icon name="trash" size={16} />
            {t("removeContact")}
          </button>
        </div>
      </details>
    </li>
  );
}

function AddById(): JSX.Element {
  const [id, setId] = useState("");
  const [state, setState] = useState<"idle" | "searching" | "requested" | "notfound" | "invalid">(
    "idle",
  );
  const submit = async (e: Event) => {
    e.preventDefault();
    try {
      const chat = await contactsManager().addById(id, (s) => setState(s));
      if (chat) showChat(chat);
      else setState("notfound");
    } catch {
      setState("invalid");
    }
  };
  const busy = state === "searching" || state === "requested";
  return (
    <form class="card stack" onSubmit={submit}>
      <h2>{t("addById")}</h2>
      <p class="hint">{t("addByIdHint")}</p>
      <input
        type="text"
        value={id}
        spellcheck={false}
        autocomplete="off"
        placeholder={t("idPlaceholder")}
        data-testid="add-id"
        onInput={(e) => setId((e.target as HTMLInputElement).value)}
      />
      {state === "searching" && (
        <p class="status-line">
          <span class="spinner" />
          {t("searching", { id: `${id.slice(0, 14)}…` })}
        </p>
      )}
      {state === "requested" && (
        <p class="status-line">
          <span class="spinner" />
          {t("requestSent")}
        </p>
      )}
      {state === "notfound" && <p class="error">{t("notFound")}</p>}
      {state === "invalid" && <p class="error">{t("invalidId")}</p>}
      <button
        type="submit"
        class="btn btn-primary"
        disabled={busy || !id.trim() || !identity.value}
      >
        {t("connect")}
      </button>
    </form>
  );
}

function Requests(): JSX.Element | null {
  const list = contactsManager().requests.value;
  if (!list.length) return null;
  return (
    <section class="card stack">
      <h2>{t("requests")}</h2>
      {list.map((r) => (
        <div class="row-between" key={r.id}>
          <span>{t("wantsToConnect", { name: r.name || "?", id: `${r.id.slice(0, 14)}…` })}</span>
          <span class="row">
            <button
              type="button"
              class="btn btn-small btn-primary"
              onClick={() =>
                void r.accept().then(() => {
                  const chat = contactsManager().strangers.value.at(-1);
                  if (chat) showChat(chat);
                })
              }
            >
              {t("accept")}
            </button>
            <button type="button" class="btn btn-small btn-ghost" onClick={r.decline}>
              {t("decline")}
            </button>
          </span>
        </div>
      ))}
    </section>
  );
}

export function ContactsScreen(): JSX.Element {
  const unlocked = vaultState.value === "unlocked";
  const list = [...contacts.value].sort((a, b) => contactLabel(a).localeCompare(contactLabel(b)));
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
        <h1 class="title">{t("contacts")}</h1>
        {unlocked ? (
          <button
            type="button"
            class="btn btn-ghost btn-small"
            onClick={() => {
              contactsManager().stop();
              lockVault();
            }}
          >
            <Icon name="lock" size={16} />
            {t("vaultLock")}
          </button>
        ) : (
          <span />
        )}
      </header>
      {!unlocked ? (
        <VaultGate />
      ) : (
        <>
          <Requests />
          {list.length === 0 ? (
            <p class="card hint">{t("noContacts")}</p>
          ) : (
            <ul class="contacts card">
              {list.map((c) => (
                <ContactRow key={c.key} c={c} />
              ))}
            </ul>
          )}
          {identity.value ? <AddById /> : null}
          <BackupPanel />
          <button
            type="button"
            class="btn btn-danger-ghost btn-small"
            onClick={() => {
              if (confirm(t("vaultDeleteConfirm"))) {
                contactsManager().stop();
                void eraseVault();
              }
            }}
          >
            <Icon name="trash" size={16} />
            {t("vaultDelete")}
          </button>
          <p class="faint center small">{myName() || ""}</p>
        </>
      )}
    </main>
  );
}
