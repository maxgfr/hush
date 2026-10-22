import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { Chat, ChatItem } from "../../app/chat";
import { ContactChat } from "../../app/contacts";
import { Conversation } from "../../app/conversation";
import type { PeerConn } from "../../app/peer";
import { contactsManager, go, view } from "../../app/state";
import { vaultState } from "../../app/store";
import { FileCard } from "../components/FileCard";
import { Icon } from "../components/Icons";
import { ManualPanel } from "../components/ManualPanel";
import { ShareLink } from "../components/ShareLink";
import { Verify } from "../components/Verify";
import { TTL_SECONDS, t, ttlLabel } from "../i18n";

const time = (ts: number) =>
  new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

function Waiting(props: { chat: Chat }): JSX.Element {
  const { chat } = props;
  const conv = chat instanceof Conversation ? chat : null;
  const connecting = chat.status.value === "connecting";
  return (
    <div class="waiting">
      {conv && conv.role === "creator" && (
        <ShareLink
          url={conv.url(location.href)}
          expiresMs={conv.expiresMs}
          group={conv.group}
          passphrase={conv.fragment.endsWith(".p")}
        />
      )}
      <p class="status-line" role="status" data-testid="status">
        <span class="spinner" aria-hidden="true" />
        {connecting
          ? t("connecting")
          : conv?.role === "joiner"
            ? t("waitingCreator")
            : t("waitingPeer")}
      </p>
      {conv?.role === "joiner" && conv.fragment.endsWith(".p") && !connecting && (
        <p class="hint center">{t("waitingPassHint")}</p>
      )}
      {conv && <ManualPanel />}
    </div>
  );
}

function PeerBadge(props: { peer: PeerConn }): JSX.Element {
  const s = props.peer.state.value;
  const verified = props.peer.verifiedByMe.value && props.peer.verifiedByPeer.value;
  return (
    <span class={`peer-badge ${s}`} data-testid="peer-state" data-state={s}>
      <span class="dot" aria-hidden="true" />
      {s === "direct" ? t("direct") : s === "relay" ? t("relay") : t("connectingShort")}
      {verified && (
        <span class="verified">
          <Icon name="shield" size={14} />
          {t("verified")}
        </span>
      )}
    </span>
  );
}

function Message(props: { chat: Chat; item: ChatItem }): JSX.Element {
  const { item } = props;
  if (item.from === undefined) {
    return (
      <li class="notice" role="status">
        {item.text}
      </li>
    );
  }
  const mine = item.from === null;
  return (
    <li class={mine ? "msg mine" : "msg"} data-testid="message">
      {props.chat.group && !mine && (
        <span class="author">{item.from?.name.value || t("someone")}</span>
      )}
      <div class="bubble">
        {item.file ? (
          <FileCard chat={props.chat} file={item.file} />
        ) : (
          <p class="text">{item.text}</p>
        )}
        <span class="when">
          {item.expiresAt && <Icon name="timer" size={12} />}
          {time(item.ts)}
        </span>
      </div>
    </li>
  );
}

function ContactOffers(props: { chat: Chat }): JSX.Element | null {
  const book = contactsManager();
  const offers = book.offers.value.filter((o) => o.chat === props.chat);
  if (!offers.length) return null;
  return (
    <>
      {offers.map((o) => (
        <div class="callout row-between" key={o.info.id}>
          <span>{t("contactAsked", { name: o.info.name || o.peer.name.value || t("them") })}</span>
          <span class="row">
            <button
              type="button"
              class="btn btn-small btn-primary"
              onClick={() =>
                vaultState.value === "unlocked"
                  ? void book.acceptOffer(o)
                  : go({ name: "contacts" })
              }
            >
              {t("saveContact")}
            </button>
            <button
              type="button"
              class="btn btn-small btn-ghost"
              onClick={() => book.dismissOffer(o)}
              aria-label={t("close")}
            >
              <Icon name="x" size={16} />
            </button>
          </span>
        </div>
      ))}
    </>
  );
}

function Composer(props: { chat: Chat }): JSX.Element {
  const [text, setText] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const send = () => {
    if (!text.trim()) return;
    void props.chat.sendText(text);
    setText("");
  };
  return (
    <form
      class="composer"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <input
        ref={file}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          const input = e.target as HTMLInputElement;
          for (const f of Array.from(input.files ?? [])) void props.chat.sendFile(f);
          input.value = "";
        }}
      />
      <button
        type="button"
        class="btn btn-ghost btn-icon"
        onClick={() => file.current?.click()}
        aria-label={t("attach")}
      >
        <Icon name="attach" />
      </button>
      <textarea
        rows={1}
        value={text}
        placeholder={t("messagePlaceholder")}
        aria-label={t("messagePlaceholder")}
        data-testid="composer"
        onInput={(e) => setText((e.target as HTMLTextAreaElement).value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
            e.preventDefault();
            send();
          }
        }}
      />
      <button
        type="submit"
        class="btn btn-primary btn-icon"
        disabled={!text.trim()}
        aria-label={t("send")}
      >
        <Icon name="send" />
      </button>
    </form>
  );
}

export function ChatScreen(props: { chat: Chat }): JSX.Element {
  const { chat } = props;
  const list = useRef<HTMLOListElement>(null);
  const [menu, setMenu] = useState(false);
  const [inviting, setInviting] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const peers = chat.peers.value.filter((p) => p.state.value !== "closed");
  const items = chat.items.value;
  const conv = chat instanceof Conversation ? chat : null;
  const contact = chat instanceof ContactChat ? chat.contact : null;

  useEffect(() => {
    chat.unread.value = 0;
    list.current?.lastElementChild?.scrollIntoView({ block: "end" });
  }, [items.length]);

  const title = chat.group
    ? t("members", { n: peers.length + 1 })
    : contact?.name || peers[0]?.name.value || t("them");

  const end = () => {
    if (peers.length === 0 || confirm(t("endConfirm"))) chat.end();
  };

  const save = async (peer: PeerConn) => {
    setMenu(false);
    if (!(peer.verifiedByMe.value && peer.verifiedByPeer.value)) {
      setNote(t("verifyFirst"));
      return;
    }
    if (vaultState.value !== "unlocked") {
      view.value = { name: "contacts" };
      return;
    }
    const r = await contactsManager().offerCard(chat, peer);
    setNote(
      r === "saved"
        ? t("contactSaved", { name: peer.name.value || t("them") })
        : t("contactWaiting", { name: peer.name.value || t("them") }),
    );
  };

  const open = peers.length > 0;
  return (
    <main class="chat">
      <header class="chat-head">
        <button
          type="button"
          class="btn btn-ghost btn-icon"
          aria-label={t("back")}
          onClick={() =>
            contact || chat instanceof ContactChat ? go({ name: "contacts" }) : end()
          }
        >
          <Icon name={contact ? "arrowLeft" : "x"} />
        </button>
        <div class="chat-title">
          <h1>{open || chat.group ? title : "hush"}</h1>
          <div class="chat-sub">
            {open && !chat.group && peers[0] && <PeerBadge peer={peers[0]} />}
            <span class="faint">{t("e2ePq")}</span>
          </div>
        </div>
        <div class="row">
          <label class="ttl" title={t("messageTimer")}>
            <Icon name="timer" size={18} />
            <select
              aria-label={t("messageTimer")}
              value={chat.ttl.value}
              onChange={(e) => chat.setTtl(Number((e.target as HTMLSelectElement).value))}
            >
              {TTL_SECONDS.map((s) => (
                <option key={s} value={s}>
                  {ttlLabel(s)}
                </option>
              ))}
            </select>
          </label>
          {(conv?.group || peers.length > 0) && (
            <button
              type="button"
              class="btn btn-ghost btn-icon"
              aria-label="Menu"
              aria-expanded={menu}
              onClick={() => setMenu(!menu)}
            >
              <Icon name="settings" />
            </button>
          )}
          {!contact && (
            <button type="button" class="btn btn-danger btn-small" onClick={end} data-testid="end">
              {t("endConversation")}
            </button>
          )}
        </div>
      </header>

      {menu && (
        <div class="menu card" role="menu">
          {conv?.group && (
            <>
              <button
                type="button"
                role="menuitem"
                class="menu-item"
                onClick={() => {
                  setInviting(!inviting);
                  setMenu(false);
                }}
              >
                <Icon name="link" />
                {t("invite")}
              </button>
              <button
                type="button"
                role="menuitem"
                class="menu-item"
                onClick={() => {
                  chat.setLocked(!chat.locked.value);
                  setMenu(false);
                }}
              >
                <Icon name="lock" />
                {chat.locked.value ? t("unlock") : t("lock")}
              </button>
            </>
          )}
          {!contact &&
            peers.map((p) => (
              <button
                type="button"
                role="menuitem"
                class="menu-item"
                key={p.name.value}
                onClick={() => void save(p)}
              >
                <Icon name="user" />
                {t("saveContact")}
                {chat.group ? ` · ${p.name.value || t("someone")}` : ""}
              </button>
            ))}
          {contact && (
            <button
              type="button"
              role="menuitem"
              class="menu-item"
              onClick={() => {
                chat.end();
                view.value = { name: "contacts" };
              }}
            >
              <Icon name="leave" />
              {t("endConversation")}
            </button>
          )}
        </div>
      )}

      <div class="chat-body">
        {!open && <Waiting chat={chat} />}
        {conv?.group && inviting && open && (
          <ShareLink
            url={conv.url(location.href)}
            expiresMs={conv.expiresMs}
            group
            passphrase={conv.fragment.endsWith(".p")}
          />
        )}
        {chat.locked.value && <p class="notice">{t("locked")}</p>}
        {peers.map((p) => (
          <Verify key={p.name.value} chat={chat} peer={p} />
        ))}
        <ContactOffers chat={chat} />
        {note && (
          <p class="callout row-between" role="status">
            <span>{note}</span>
            <button
              type="button"
              class="btn btn-ghost btn-small"
              onClick={() => setNote(null)}
              aria-label={t("close")}
            >
              <Icon name="x" size={16} />
            </button>
          </p>
        )}
        {open && (
          <p class="notice">
            <Icon name="lock" size={14} />
            {t("newSessionStarted")}
          </p>
        )}
        <ol class="messages" ref={list} aria-live="polite">
          {items.map((i) => (
            <Message key={i.id} chat={chat} item={i} />
          ))}
        </ol>
      </div>
      {open && <Composer chat={chat} />}
    </main>
  );
}
