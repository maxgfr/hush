/** App-wide state: the current screen, the rendezvous, contacts, and starting conversations. */
import { effect, signal } from "@preact/signals";
import { coreError, linkInfo, loadCore } from "../core/wasm";
import type { DropEntry } from "../drop/entries";
import { Rendezvous } from "../net/rendezvous";
import type { TransportStatus } from "../signal/types";
import type { Chat, ChatStatus } from "./chat";
import { Contacts } from "./contacts";
import { Conversation } from "./conversation";
import type { IncomingDrop } from "./drop";
import { settings, signalConfig } from "./settings";
import { initStore } from "./store";

export type View =
  | { name: "home" }
  | { name: "new" }
  | { name: "join"; error?: string }
  | { name: "passphrase"; fragment: string; error?: string; drop?: boolean }
  | { name: "drop"; entries?: DropEntry[] }
  | { name: "dropReceive"; drop: IncomingDrop }
  | { name: "chat"; chat: Chat }
  | { name: "ended"; status: ChatStatus }
  | { name: "contacts" }
  | { name: "identity" }
  | { name: "settings" };

export const view = signal<View>({ name: "home" });

/** Navigates to a screen. */
export function go(v: View): void {
  view.value = v;
}
export const ready = signal(false);
export const bootError = signal<string | null>(null);
export const manualCodes = signal<string[]>([]);
/** The offline (QR) panel is open: it stays up until the direct link opens. */
export const manualOpen = signal(false);
export const transports = signal<TransportStatus[]>([]);

let rdv: Rendezvous | null = null;
let book: Contacts | null = null;

export const myName = () => settings.value.name.trim();

export function rendezvous(): Rendezvous {
  if (!rdv) throw new Error("not booted");
  return rdv;
}

export function contactsManager(): Contacts {
  if (!book) throw new Error("not booted");
  return book;
}

/** Reads (and immediately removes) a link from the address bar: it must not linger in history. */
export function takeFragment(): string | null {
  const hash = location.hash.slice(1);
  // Any version: an old link must reach openLink to get its own message.
  if (!/^\d+\./.test(hash)) return null;
  history.replaceState(null, "", location.pathname + location.search);
  return hash;
}

export async function boot(): Promise<void> {
  try {
    await loadCore();
  } catch (e) {
    bootError.value = e instanceof Error ? e.message : String(e);
    return;
  }
  rdv = new Rendezvous(
    { config: signalConfig() },
    {
      onStatus: (s) => (transports.value = s),
      onManualOutgoing: (code) => (manualCodes.value = [...manualCodes.value.slice(-5), code]),
    },
  );
  book = new Contacts(rdv, myName);
  await initStore();
  ready.value = true;
  const fragment = takeFragment();
  if (fragment) openLink(fragment);
  window.addEventListener("hashchange", () => {
    const f = takeFragment();
    if (f && view.value.name !== "chat" && view.value.name !== "dropReceive") openLink(f);
  });
}

/** Shows a chat and follows it to its end. */
export function showChat(chat: Chat): void {
  view.value = { name: "chat", chat };
  const stop = effect(() => {
    const s = chat.status.value;
    if (s === "ended" || s === "left" || s === "taken" || s === "expired") {
      queueMicrotask(() => stop());
      if (view.value.name === "chat" && view.value.chat === chat)
        view.value = { name: "ended", status: s };
    }
  });
}

export function startConversation(opts: {
  group: boolean;
  ttlMin: number;
  passphrase?: string;
  ttl: number;
}): void {
  manualCodes.value = [];
  const conv = Conversation.create(
    rendezvous(),
    { ...opts, name: myName },
    contactsManager().hooks,
  );
  showChat(conv);
}

/** Opens a drop (the drop code loads on first use). */
export async function openDrop(fragment: string, passphrase?: string): Promise<void> {
  const { IncomingDrop } = await import("./drop");
  try {
    const drop = /^\d+\.r\./.test(fragment)
      ? IncomingDrop.sealed(fragment)
      : IncomingDrop.link(fragment, passphrase);
    view.value = { name: "dropReceive", drop };
  } catch {
    view.value = { name: "join", error: "invalidLink" };
  }
}

export function openLink(input: string): void {
  if (/^\d+\.r\./.test(input.slice(input.lastIndexOf("#") + 1))) {
    void openDrop(input.slice(input.lastIndexOf("#") + 1));
    return;
  }
  let info: ReturnType<typeof linkInfo>;
  try {
    info = linkInfo(input);
  } catch (e) {
    view.value = {
      name: "join",
      error: coreError(e) === "Outdated" ? "outdatedLink" : "invalidLink",
    };
    return;
  }
  if (info.expiresMs <= Date.now()) {
    view.value = { name: "ended", status: "expired" };
    return;
  }
  if (info.passphrase) {
    view.value = { name: "passphrase", fragment: info.fragment, drop: info.mode === "drop" };
    return;
  }
  if (info.mode === "drop") {
    void openDrop(info.fragment);
    return;
  }
  joinLink(info.fragment);
}

/** Joins a link (with its passphrase, if any). Argon2id makes this take a second. */
export function joinLink(fragment: string, passphrase?: string): void {
  manualCodes.value = [];
  try {
    const conv = Conversation.join(
      rendezvous(),
      fragment,
      { ...(passphrase ? { passphrase } : {}), name: myName, ttl: settings.value.ttl },
      contactsManager().hooks,
    );
    showChat(conv);
  } catch (e) {
    const expired = e instanceof Error && e.message === "Expired";
    view.value = expired
      ? { name: "ended", status: "expired" }
      : { name: "join", error: "invalidLink" };
  }
}

/** After a conversation: back to a clean page (a reload drops every byte of it from memory). */
export function startOver(): void {
  location.replace(location.pathname + location.search);
}
