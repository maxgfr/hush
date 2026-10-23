import { afterEach, describe, expect, setSystemTime, test } from "bun:test";
import { signal } from "@preact/signals";
import { Lane } from "../core/wasm";
import type { Rendezvous } from "../net/rendezvous";
import { Chat, type ChatHooks, MAX_ITEMS } from "./chat";
import type { PeerConn } from "./peer";
import { CHUNK_SIZE, newId } from "./protocol";

class TestChat extends Chat {
  readonly group = true;
  protected cleanup(): void {}
  receive(peer: PeerConn, msg: unknown): Promise<void> {
    // onChat is private: reach it as a peer's frame would.
    const self = this as unknown as { onChat(p: PeerConn, m: unknown): Promise<void> };
    return self.onChat(peer, msg);
  }
  tick(): void {
    this.sweep();
  }
}

/** Just enough of a peer: a tag, a name, a link and what it was sent. */
function fakePeer(name: string) {
  const sent: { lane: number; data: Uint8Array }[] = [];
  const peer = {
    tag: crypto.getRandomValues(new Uint8Array(16)),
    name: signal(name),
    state: signal("direct"),
    rtc: { isOpen: true },
    isVerified: true,
    send: async (lane: number, data: Uint8Array) => {
      // A real link takes its time: other frames arrive while a file streams.
      await new Promise((r) => setTimeout(r, 1));
      sent.push({ lane, data });
    },
    end() {},
  } as unknown as PeerConn;
  return { peer, sent };
}

const chats: Chat[] = [];
function newChat(hooks: ChatHooks = {}): TestChat {
  const chat = new TestChat({} as Rendezvous, () => "me", hooks);
  chats.push(chat);
  return chat;
}
afterEach(() => {
  for (const c of chats.splice(0)) c.end();
  setSystemTime();
});

const msg = (id: string, text = "hi") => ({ t: "msg", id, ts: 1, text });
const offer = (id: string) => ({
  t: "file",
  id,
  ts: 1,
  name: "a.bin",
  size: 10,
  mime: "application/octet-stream",
  sha256: "0".repeat(64),
});

describe("message ids chosen by peers", () => {
  test("a message or file reusing an id already shown is dropped, whoever wrote it", async () => {
    const chat = newChat();
    const bob = fakePeer("bob");
    const eve = fakePeer("eve");
    await chat.sendText("mine");
    const mine = chat.items.value[0]?.id as string;
    const theirs = newId();
    await chat.receive(bob.peer, msg(theirs));
    await chat.receive(eve.peer, msg(mine, "spoof"));
    await chat.receive(eve.peer, msg(theirs, "spoof"));
    await chat.receive(eve.peer, offer(mine));
    await chat.receive(bob.peer, msg(theirs, "again"));
    expect(chat.items.value.map((i) => i.text)).toEqual(["mine", "hi"]);
    expect(new Set(chat.items.value.map((i) => i.id)).size).toBe(chat.items.value.length);
  });

  test("only a message's sender can delete it, and kept history hears whose it was", async () => {
    const deleted: [string, boolean][] = [];
    const chat = newChat({ onDelete: (_c, id, mine) => deleted.push([id, mine]) });
    const bob = fakePeer("bob");
    const eve = fakePeer("eve");
    await chat.sendText("mine");
    const mine = chat.items.value[0]?.id as string;
    const theirs = newId();
    await chat.receive(bob.peer, msg(theirs));
    await chat.receive(eve.peer, { t: "del", id: mine });
    await chat.receive(eve.peer, { t: "del", id: theirs });
    expect(chat.items.value).toHaveLength(2);
    expect(deleted).toEqual([]);
    await chat.receive(bob.peer, { t: "del", id: theirs });
    expect(chat.items.value.map((i) => i.text)).toEqual(["mine"]);
    expect(deleted).toEqual([[theirs, false]]);
    await chat.retract(mine);
    expect(chat.items.value).toEqual([]);
    expect(deleted).toEqual([
      [theirs, false],
      [mine, true],
    ]);
  });
});

describe("limits on what a peer can pile up", () => {
  test("a conversation keeps the newest items only", async () => {
    const chat = newChat();
    const bob = fakePeer("bob");
    const first = newId();
    await chat.receive(bob.peer, msg(first));
    for (let n = 0; n < MAX_ITEMS + 10; n++) await chat.receive(bob.peer, msg(newId(), `${n}`));
    expect(chat.items.value).toHaveLength(MAX_ITEMS);
    expect(chat.items.value.at(-1)?.text).toBe(`${MAX_ITEMS + 9}`);
    expect(chat.items.value.some((i) => i.id === first)).toBe(false);
  });

  test("a file streams once at a time per peer, however often it is asked for", async () => {
    const chat = newChat();
    const bob = fakePeer("bob");
    chat.peers.value = [bob.peer];
    await chat.sendFile(new File([new Uint8Array(CHUNK_SIZE * 2 + 1)], "a.bin"));
    const id = chat.items.value[0]?.id as string;
    const chunks = () => bob.sent.filter((s) => s.lane === Lane.FILE).length;
    await Promise.all(
      Array.from({ length: 5 }, () => chat.receive(bob.peer, { t: "file-get", id })),
    );
    expect(chunks()).toBe(3);
    // Once done, asking again is a fresh download.
    await chat.receive(bob.peer, { t: "file-get", id });
    expect(chunks()).toBe(6);
  });
});

describe("files leave with their message", () => {
  test("an expired file is no longer served and its object URL is revoked", async () => {
    const chat = newChat();
    const bob = fakePeer("bob");
    chat.peers.value = [bob.peer];
    chat.setTtl(30);
    await chat.sendFile(new File([new Uint8Array(100)], "a.bin"));
    const item = chat.items.value[0];
    const url = item?.file?.url.value as string;
    expect(url).toStartWith("blob:");
    setSystemTime(new Date(Date.now() + 31_000));
    chat.tick();
    expect(chat.items.value).toEqual([]);
    expect(item?.file?.url.value).toBeNull();
    await chat.receive(bob.peer, { t: "file-get", id: item?.id });
    expect(bob.sent.filter((s) => s.lane === Lane.FILE)).toEqual([]);
  });

  test("a download is dropped when its offer is deleted", async () => {
    const chat = newChat();
    const bob = fakePeer("bob");
    const id = newId();
    await chat.receive(bob.peer, offer(id));
    const file = chat.items.value[0]?.file;
    if (!file) throw new Error("no file");
    chat.download(file);
    expect(file.state.value).toBe("downloading");
    await chat.receive(bob.peer, { t: "del", id });
    expect(file.state.value).toBe("failed");
    // Late chunks find no download.
    const downloads = (chat as unknown as { downloads: Map<string, unknown> }).downloads;
    expect(downloads.size).toBe(0);
  });
});
