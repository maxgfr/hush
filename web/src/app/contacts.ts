/**
 * Saved contacts and identifiers, once the vault is unlocked.
 *
 * - Saving: inside a verified conversation, both sides send a contact card
 *   (their signed bundle, bound to this session's transcript). Each side
 *   stores the other's card and the session's contact secret.
 * - Reconnecting: each contact has a rendezvous derived from that secret.
 *   Whoever comes online posts HERE; the side with the smaller identifier
 *   hash starts the contact handshake (C1/C2/C3), so the two never race.
 * - Identifier requests: when "reachable" is on, we answer lookups of our
 *   identifier with our bundle, and show incoming requests (each carrying a
 *   proof of work) until the user accepts or declines.
 */
import { signal } from "@preact/signals";
import {
  type BundleInfo,
  type Channel,
  type ContactInitiator,
  type ContactInitiatorReady,
  type ContactResponder,
  type ContactResponderPending,
  type EstablishedResult,
  hush,
  type Identity,
  Kind,
  type OpenedEnvelope,
  parseBundle,
  verifyCard,
} from "../core/wasm";
import { concat, equalBytes, fromB64u, fromHex, toB64u, toHex } from "../lib/bytes";
import type { Rendezvous } from "../net/rendezvous";
import { Chat, type ChatHooks, type ChatItem, type Pending } from "./chat";
import type { PeerConn } from "./peer";
import { decodeHello, encodeJson } from "./protocol";
import {
  appendHistory,
  type Contact,
  contacts,
  ensureIdentity,
  identity,
  isUnlocked,
  loadHistory,
  saveContact,
} from "./store";

const HERE_MS = 30_000;
const HANDSHAKE_TIMEOUT_MS = 45_000;
const LOOKUP_MS = 4000;
const LOOKUP_TRIES = 15;
const REQUEST_WAIT_MS = 5 * 60_000;

/** A one-to-one chat with a saved contact (or a new one met through its identifier). */
export class ContactChat extends Chat {
  readonly group = false;

  constructor(
    readonly contact: Contact | null,
    rendezvous: Rendezvous,
    myName: () => string,
    hooks: ChatHooks,
    private readonly onDone: () => void,
  ) {
    super(rendezvous, myName, hooks);
    this.status.value = "connecting";
  }

  adopt(tag: Uint8Array, res: EstablishedResult, name: string, pending: Pending): PeerConn | null {
    const peer = this.addPeer(tag, res, name, pending);
    if (peer) {
      // Contacts were verified when they were saved.
      peer.verifiedByMe.value = true;
      peer.verifiedByPeer.value = true;
    }
    return peer;
  }

  rtc(): Pending {
    return this.newRtc();
  }

  showHistory(items: ChatItem[]): void {
    this.items.value = [...items, ...this.items.value];
  }

  protected cleanup(): void {
    this.onDone();
  }
}

export interface IncomingRequest {
  id: string;
  name: string;
  note: string;
  bundle: BundleInfo;
  accept(): Promise<void>;
  decline(): void;
}

/** Contact offers received in link conversations, waiting for the user's answer. */
export interface CardOffer {
  chat: Chat;
  peer: PeerConn;
  info: BundleInfo;
}

const hex = toHex;

export class Contacts {
  /** Live chats by contact key. */
  readonly chats = signal<Map<string, ContactChat>>(new Map());
  readonly online = signal<Set<string>>(new Set());
  readonly offers = signal<CardOffer[]>([]);
  readonly requests = signal<IncomingRequest[]>([]);
  readonly reachable = signal(false);
  /** Chats with people met through their identifier, before they are saved. */
  readonly strangers = signal<ContactChat[]>([]);

  private readonly listening = new Map<string, { channel: Channel; off: () => void }>();
  private readonly handshaking = new Map<string, number>();
  private readonly savingFor = new WeakSet<PeerConn>();
  private readonly received = new WeakMap<PeerConn, BundleInfo>();
  private reach: { channel: Channel; off: () => void } | null = null;
  private timer: ReturnType<typeof setInterval> | undefined;

  constructor(
    private readonly rendezvous: Rendezvous,
    private readonly myName: () => string,
  ) {}

  private me(): Identity | null {
    return isUnlocked() ? identity.value : null;
  }

  /** Hooks every chat gets, so cards and history are handled in one place. */
  readonly hooks: ChatHooks = {
    onCard: (chat, peer, card) => this.onCard(chat, peer, card),
    onText: (chat, item) => {
      if (!(chat instanceof ContactChat) || !chat.contact?.history || item.from === undefined)
        return;
      void appendHistory(chat.contact.key, {
        id: item.id,
        mine: item.from === null,
        ts: item.ts,
        text: item.text ?? "",
        ...(item.expiresAt ? { expiresAt: item.expiresAt } : {}),
      }).catch(() => {});
    },
  };

  // ----------------------------------------------------------------------------------------
  // Saving a contact from a conversation

  /** "Save contact" on a verified peer: sends our card; saved once theirs arrives. */
  async offerCard(chat: Chat, peer: PeerConn): Promise<"saved" | "waiting"> {
    const me = await ensureIdentity(this.myName());
    this.savingFor.add(peer);
    await chat.sendCard(peer, me.card(this.myName(), peer.hash, Date.now()));
    const theirs = this.received.get(peer);
    if (theirs) {
      await this.store(peer, theirs);
      return "saved";
    }
    return "waiting";
  }

  private onCard(chat: Chat, peer: PeerConn, card: Uint8Array): void {
    let info: BundleInfo;
    try {
      info = verifyCard(card, peer.hash, Date.now());
    } catch {
      return;
    }
    this.received.set(peer, info);
    if (chat instanceof ContactChat) {
      // A known contact refreshing its bundle (new prekey, new name).
      const c = chat.contact ?? contacts.value.find((x) => x.key === hex(info.idHash));
      if (c && c.key === hex(info.idHash) && this.me()) {
        void saveContact({ ...c, bundle: toB64u(info.bytes), name: info.name || c.name });
      } else if (!c && this.me()) {
        void this.store(peer, info);
      }
      return;
    }
    if (this.savingFor.has(peer)) {
      void this.store(peer, info);
    } else {
      this.offers.value = [
        ...this.offers.value.filter((o) => o.peer !== peer),
        { chat, peer, info },
      ];
    }
  }

  /** The user accepted a card offer. */
  async acceptOffer(offer: CardOffer): Promise<void> {
    this.offers.value = this.offers.value.filter((o) => o !== offer);
    await this.offerCard(offer.chat, offer.peer);
    await this.store(offer.peer, offer.info);
  }

  dismissOffer(offer: CardOffer): void {
    this.offers.value = this.offers.value.filter((o) => o !== offer);
  }

  private async store(peer: PeerConn, info: BundleInfo): Promise<void> {
    if (!this.me()) return;
    const key = hex(info.idHash);
    const existing = contacts.value.find((c) => c.key === key);
    await saveContact({
      id: info.id,
      key,
      name: info.name || peer.name.value || existing?.name || info.id.slice(0, 12),
      bundle: toB64u(info.bytes),
      secret: toB64u(peer.session.contactSecret()),
      added: existing?.added ?? Date.now(),
      history: existing?.history ?? false,
    });
    this.watch();
  }

  isSaved(peer: PeerConn): boolean {
    const info = this.received.get(peer);
    return !!info && contacts.value.some((c) => c.key === hex(info.idHash));
  }

  // ----------------------------------------------------------------------------------------
  // Presence and reconnection

  /** Starts (or refreshes) listening for every saved contact. */
  watch(): void {
    const me = this.me();
    if (!me) return;
    const keys = new Set(contacts.value.map((c) => c.key));
    for (const [k, l] of this.listening) {
      if (!keys.has(k)) {
        l.off();
        l.channel.free();
        this.listening.delete(k);
      }
    }
    for (const c of contacts.value) {
      if (this.listening.has(c.key)) continue;
      const channel = hush().Channel.forContact(fromB64u(c.secret));
      const off = this.rendezvous.listen(channel, (env) => this.onContactEnvelope(c.key, env));
      this.listening.set(c.key, { channel, off });
      this.here(c.key);
    }
    if (!this.timer) {
      this.timer = setInterval(() => {
        for (const k of this.listening.keys())
          if (!this.chats.value.get(k)?.connected.value) this.here(k);
      }, HERE_MS);
    }
  }

  stop(): void {
    clearInterval(this.timer);
    this.timer = undefined;
    for (const l of this.listening.values()) {
      l.off();
      l.channel.free();
    }
    this.listening.clear();
    for (const c of this.chats.value.values()) c.end();
    this.setReachable(false);
  }

  private here(key: string, to?: Uint8Array): void {
    const me = this.me();
    const l = this.listening.get(key);
    if (!me || !l) return;
    this.rendezvous.send(l.channel, Kind.HERE, me.idHash, to ? { to } : {});
  }

  private contact(key: string): Contact | undefined {
    return contacts.value.find((c) => c.key === key);
  }

  private onContactEnvelope(key: string, env: OpenedEnvelope): void {
    const c = this.contact(key);
    const me = this.me();
    if (!c || !me) return;
    switch (env.kind) {
      case Kind.HERE: {
        if (hex(env.body) !== key) return;
        this.online.value = new Set([...this.online.value, key]);
        if (this.chats.value.get(key)?.connected.value || this.busy(key)) return;
        if (hex(me.idHash) < key) void this.initiate(c, env.from, null);
        else if (!env.to) this.here(key, env.from); // let them know we are here: they start
        break;
      }
      case Kind.C1:
        if (env.to) void this.respond(c, env, null);
        break;
    }
  }

  private busy(key: string): boolean {
    const since = this.handshaking.get(key);
    return since !== undefined && Date.now() - since < HANDSHAKE_TIMEOUT_MS;
  }

  private newChat(contact: Contact | null): ContactChat {
    const chat: ContactChat = new ContactChat(
      contact,
      this.rendezvous,
      this.myName,
      this.hooks,
      () => {
        if (contact) {
          const m = new Map(this.chats.value);
          if (m.get(contact.key) === chat) m.delete(contact.key);
          this.chats.value = m;
          const on = new Set(this.online.value);
          on.delete(contact.key);
          this.online.value = on;
        } else {
          this.strangers.value = this.strangers.value.filter((s) => s !== chat);
        }
      },
    );
    if (contact) {
      const m = new Map(this.chats.value);
      m.get(contact.key)?.end();
      m.set(contact.key, chat);
      this.chats.value = m;
    } else {
      this.strangers.value = [...this.strangers.value, chat];
    }
    return chat;
  }

  private async onEstablished(
    chat: ContactChat,
    tag: Uint8Array,
    res: EstablishedResult,
    name: string,
    pending: Pending,
  ) {
    const peer = chat.adopt(tag, res, name, pending);
    if (!peer) return;
    const me = this.me();
    if (me)
      await chat.sendCard(peer, me.card(this.myName(), peer.hash, Date.now())).catch(() => {});
    if (chat.contact?.history) {
      const past = await loadHistory(chat.contact.key);
      chat.showHistory(
        past.map((e) => ({
          id: e.id,
          from: e.mine ? null : peer,
          author: e.mine ? this.myName() : peer.name.value,
          ts: e.ts,
          text: e.text,
          ...(e.expiresAt ? { expiresAt: e.expiresAt } : {}),
        })),
      );
    }
  }

  /**
   * Initiator side of the contact handshake. `channel` is the contact's (or, for a first
   * request, the identifier's); `request` carries the proof of work for a first request.
   */
  private async initiate(
    contact: Contact,
    to: Uint8Array,
    first: { channel: Channel; bundle: Uint8Array; id: string } | null,
  ): Promise<ContactChat | null> {
    const me = this.me();
    if (!me) return null;
    const key = contact.key;
    this.handshaking.set(key, Date.now());
    const channel = first?.channel ?? this.listening.get(key)?.channel;
    if (!channel) return null;
    const psk = first ? new Uint8Array(0) : fromB64u(contact.secret);
    const payload = first
      ? encodeJson({
          v: 1,
          name: this.myName(),
          bundle: toB64u(me.bundle(this.myName(), Date.now())),
        })
      : encodeJson({ v: 1, name: this.myName() });
    let start: { state: ContactInitiator; c1: Uint8Array };
    try {
      start = me.contactStart(psk, fromB64u(contact.bundle), payload) as typeof start;
    } catch {
      this.handshaking.delete(key);
      return null;
    }
    const chat = this.newChat(first ? null : contact);
    if (first) {
      const day = hush().utcDay(Date.now());
      const challenge = hush().powChallenge(first.id, day, start.c1);
      const nonce = hush().powSolve(challenge, hush().powBits());
      const n = new Uint8Array(8);
      new DataView(n.buffer).setBigUint64(0, BigInt(nonce));
      this.rendezvous.send(channel, Kind.REQUEST, concat(me.idHash, n, start.c1), { to });
    } else {
      this.rendezvous.send(channel, Kind.C1, start.c1, { to });
    }

    const c2 = await this.waitFor(
      channel,
      Kind.C2,
      to,
      first ? REQUEST_WAIT_MS : HANDSHAKE_TIMEOUT_MS,
    );
    if (!c2 || chat.isFinished) {
      this.handshaking.delete(key);
      if (!c2) chat.end();
      return c2 ? chat : null;
    }
    try {
      const read = start.state.readC2(me, c2.body, Date.now()) as {
        state: ContactInitiatorReady;
        payload: Uint8Array;
      };
      const hello = decodeHello(read.payload);
      if (!hello?.sdp) throw new Error("no offer");
      const pending = chat.rtc();
      const answer = await pending.rtc.acceptOffer(hello.sdp);
      const res = read.state.writeC3(
        encodeJson({ v: 1, sdp: answer }),
        Date.now(),
      ) as EstablishedResult & {
        c3: Uint8Array;
      };
      this.rendezvous.send(channel, Kind.C3, res.c3, { to });
      await this.onEstablished(chat, to, res, hello.name || contact.name, pending);
    } catch {
      chat.end();
    } finally {
      this.handshaking.delete(key);
    }
    return chat;
  }

  /** Responder side: someone (a contact, or a requester we accepted) sent C1. */
  private async respond(
    contact: Contact,
    env: OpenedEnvelope,
    first: {
      channel: Channel;
      c1: Uint8Array;
      pending: ContactResponderPending;
      name: string;
    } | null,
  ): Promise<ContactChat | null> {
    const me = this.me();
    if (!me) return null;
    const key = contact.key;
    if (!first && this.busy(key)) return null;
    const channel = first?.channel ?? this.listening.get(key)?.channel;
    if (!channel) return null;
    let pending: ContactResponderPending;
    let name = contact.name;
    if (first) {
      pending = first.pending;
      name = first.name || name;
    } else {
      try {
        const read = me.contactRead(
          fromB64u(contact.secret),
          fromHex(contact.key),
          env.body,
          Date.now(),
        ) as {
          state: ContactResponderPending;
          payload: Uint8Array;
        };
        pending = read.state;
        name = decodeHello(read.payload)?.name || name;
      } catch {
        return null;
      }
    }
    this.handshaking.set(key, Date.now());
    const chat = this.newChat(first ? null : contact);
    try {
      const link = chat.rtc();
      const offer = await link.rtc.createOffer();
      const out = pending.writeC2(
        fromB64u(contact.bundle),
        encodeJson({ v: 1, name: this.myName(), sdp: offer }),
      ) as {
        state: ContactResponder;
        c2: Uint8Array;
      };
      this.rendezvous.send(channel, Kind.C2, out.c2, { to: env.from });
      const c3 = await this.waitFor(channel, Kind.C3, env.from, HANDSHAKE_TIMEOUT_MS);
      if (!c3) throw new Error("timeout");
      const res = out.state.readC3(c3.body, Date.now()) as EstablishedResult & {
        payload: Uint8Array;
      };
      const hello = decodeHello(res.payload);
      if (hello?.sdp) await link.rtc.acceptAnswer(hello.sdp);
      await this.onEstablished(chat, env.from, res, name, link);
    } catch {
      chat.end();
    } finally {
      this.handshaking.delete(key);
    }
    return chat;
  }

  /** Resolves with the next envelope of `kind` from `from` on `channel`, or null on timeout. */
  private waitFor(
    channel: Channel,
    kind: number,
    from: Uint8Array,
    ms: number,
  ): Promise<OpenedEnvelope | null> {
    return new Promise((resolve) => {
      const off = this.rendezvous.listen(channel, (env) => {
        if (env.kind !== kind || !equalBytes(env.from, from) || !env.to) return;
        clearTimeout(timer);
        off();
        resolve(env);
      });
      const timer = setTimeout(() => {
        off();
        resolve(null);
      }, ms);
    });
  }

  /** Opens (or returns) the chat with a contact; it connects when they come online. */
  chatWith(key: string): ContactChat | null {
    const live = this.chats.value.get(key);
    if (live && !live.isFinished) return live;
    this.here(key);
    return null;
  }

  // ----------------------------------------------------------------------------------------
  // Identifiers: being reachable, and adding someone by identifier

  setReachable(on: boolean): void {
    const me = this.me();
    if (this.reach) {
      this.reach.off();
      this.reach.channel.free();
      this.reach = null;
    }
    this.reachable.value = on && !!me;
    if (!on || !me) return;
    const channel = hush().Channel.forId(me.id);
    const off = this.rendezvous.listen(channel, (env) => this.onIdEnvelope(channel, env));
    this.reach = { channel, off };
  }

  private onIdEnvelope(channel: Channel, env: OpenedEnvelope): void {
    const me = this.me();
    if (!me) return;
    if (env.kind === Kind.LOOKUP) {
      this.rendezvous.send(channel, Kind.BUNDLE, me.bundle(this.myName(), Date.now()), {
        to: env.from,
      });
      return;
    }
    if (env.kind !== Kind.REQUEST || !env.to || env.body.length < 40) return;
    if (this.requests.value.length >= 20) return; // per-session cap against floods
    const claimed = env.body.subarray(0, 32);
    const nonce = Number(
      new DataView(env.body.buffer, env.body.byteOffset + 32, 8).getBigUint64(0),
    );
    const c1 = env.body.subarray(40);
    const core = hush();
    const day = core.utcDay(Date.now());
    const ok = [day, day - 1].some((d) =>
      core.powCheck(core.powChallenge(me.id, d, c1), nonce, core.powBits()),
    );
    if (!ok) return;
    let read: { state: ContactResponderPending; payload: Uint8Array };
    try {
      read = me.contactRead(new Uint8Array(0), claimed, c1, Date.now()) as typeof read;
    } catch {
      return;
    }
    const hello = decodeHello(read.payload);
    if (!hello?.bundle) return;
    let bundle: BundleInfo;
    try {
      bundle = parseBundle(fromB64u(hello.bundle), Date.now());
    } catch {
      return;
    }
    if (!equalBytes(bundle.idHash, claimed)) return;
    if (this.requests.value.some((r) => r.id === bundle.id)) return;
    const request: IncomingRequest = {
      id: bundle.id,
      name: hello.name || "",
      note: hello.note ?? "",
      bundle,
      accept: async () => {
        this.requests.value = this.requests.value.filter((r) => r !== request);
        const contact: Contact = {
          id: bundle.id,
          key: hex(bundle.idHash),
          name: hello.name || bundle.id.slice(0, 12),
          bundle: toB64u(bundle.bytes),
          secret: "",
          added: Date.now(),
          history: false,
        };
        const chat = await this.respond(contact, env, {
          channel,
          c1,
          pending: read.state,
          name: contact.name,
        });
        const peer = chat?.peers.value[0];
        if (chat && peer) this.savingFor.add(peer);
      },
      decline: () => {
        this.requests.value = this.requests.value.filter((r) => r !== request);
        read.state.free();
      },
    };
    this.requests.value = [...this.requests.value, request];
  }

  /** Looks up an identifier and sends a contact request; resolves with the chat once accepted. */
  async addById(
    id: string,
    progress: (s: "searching" | "requested") => void,
  ): Promise<ContactChat | null> {
    const me = this.me();
    if (!me) return null;
    const core = hush();
    core.idDecode(id); // throws on typos
    const channel = core.Channel.forId(id.trim().toLowerCase());
    progress("searching");
    let found: { bundle: BundleInfo; from: Uint8Array } | null = null;
    const off = this.rendezvous.listen(channel, (env) => {
      if (env.kind !== Kind.BUNDLE || !env.to || found) return;
      try {
        const b = parseBundle(env.body, Date.now());
        if (b.id === id.trim().toLowerCase()) found = { bundle: b, from: env.from };
      } catch {
        // not a valid bundle for this identifier
      }
    });
    for (let i = 0; i < LOOKUP_TRIES && !found; i++) {
      this.rendezvous.send(channel, Kind.LOOKUP, new Uint8Array(0));
      await new Promise((r) => setTimeout(r, LOOKUP_MS));
    }
    off();
    const hit = found as { bundle: BundleInfo; from: Uint8Array } | null;
    if (!hit) {
      channel.free();
      return null;
    }
    progress("requested");
    const contact: Contact = {
      id: hit.bundle.id,
      key: hex(hit.bundle.idHash),
      name: hit.bundle.name || hit.bundle.id.slice(0, 12),
      bundle: toB64u(hit.bundle.bytes),
      secret: "",
      added: Date.now(),
      history: false,
    };
    const chat = await this.initiate(contact, hit.from, {
      channel,
      bundle: hit.bundle.bytes,
      id: hit.bundle.id,
    });
    const peer = chat?.peers.value[0];
    if (chat && peer) {
      // Both sides asked to talk: save each other right away (cards are bound to this session).
      this.savingFor.add(peer);
      const info = this.received.get(peer);
      if (info) await this.store(peer, info);
    }
    return chat && !chat.isFinished ? chat : null;
  }
}

export const contactLabel = (c: Contact) => c.name || `${c.id.slice(0, 12)}…`;
