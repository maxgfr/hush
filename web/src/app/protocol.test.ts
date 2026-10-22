import { describe, expect, test } from "bun:test";
import { utf8 } from "../lib/bytes";
import {
  cleanName,
  decodeChat,
  decodeChunk,
  decodeCtrl,
  decodeHello,
  encodeChunk,
  newId,
} from "./protocol";

const json = (v: unknown) => utf8(JSON.stringify(v));

describe("application protocol", () => {
  test("ids are 16 base64url characters", () => {
    for (let i = 0; i < 50; i++) expect(newId()).toMatch(/^[A-Za-z0-9_-]{16}$/);
  });

  test("names lose control, zero-width and bidi-override characters", () => {
    expect(cleanName(" Al‮ice​\u0007 ")).toBe("Alice");
    expect(cleanName("x".repeat(200)).length).toBe(64);
  });

  test("chat messages are validated strictly", () => {
    expect(decodeChat(json({ t: "msg", id: newId(), ts: 1, text: "hi" }))).toMatchObject({
      text: "hi",
    });
    expect(decodeChat(json({ t: "msg", id: "x", ts: 1, text: "hi" }))).toBeNull();
    expect(decodeChat(json({ t: "msg", id: newId(), ts: -1, text: "hi" }))).toBeNull();
    expect(decodeChat(json({ t: "msg", id: newId(), ts: 1, text: "hi", ttl: 1e9 }))).toBeNull();
    expect(decodeChat(json({ t: "nope" }))).toBeNull();
    expect(decodeChat(utf8("not json"))).toBeNull();
    expect(decodeChat(json([1, 2]))).toBeNull();
  });

  test("file offers sanitize names and types", () => {
    const f = decodeChat(
      json({
        t: "file",
        id: newId(),
        ts: 1,
        name: "../../x.sh",
        size: 3,
        mime: "text/html;<script>",
        sha256: "a".repeat(64),
      }),
    );
    expect(f).toMatchObject({ name: ".._.._x.sh", mime: "application/octet-stream" });
    expect(
      decodeChat(
        json({ t: "file", id: newId(), ts: 1, name: "a", size: 3, mime: "x/y", sha256: "zz" }),
      ),
    ).toBeNull();
  });

  test("control messages and handshake payloads", () => {
    expect(decodeCtrl(json({ t: "verified" }))).toEqual({ t: "verified" });
    expect(decodeCtrl(json({ t: "lock", locked: "yes" }))).toBeNull();
    expect(decodeHello(json({ v: 1, name: "Bob", sdp: "v=0" }))).toEqual({
      v: 1,
      name: "Bob",
      sdp: "v=0",
    });
    expect(decodeHello(json({ v: 2 }))).toBeNull();
  });

  test("file chunks round-trip", () => {
    const id = newId();
    const c = decodeChunk(encodeChunk(id, 7, new Uint8Array([1, 2, 3])));
    expect(c?.id).toBe(id);
    expect(c?.index).toBe(7);
    expect([...(c?.data ?? [])]).toEqual([1, 2, 3]);
    expect(decodeChunk(new Uint8Array(5))).toBeNull();
    expect(() => encodeChunk("short", 0, new Uint8Array())).toThrow();
  });
});
