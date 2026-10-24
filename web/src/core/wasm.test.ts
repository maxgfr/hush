import { beforeAll, describe, expect, test } from "bun:test";
import {
  type EstablishedResult,
  type LinkCreator,
  type LinkCreatorPending,
  type LinkJoiner,
  type LinkJoinerReady,
  loadCore,
  openFrame,
} from "./wasm";

let core: Awaited<ReturnType<typeof loadCore>>;
beforeAll(async () => {
  core = await loadCore();
});

describe("the Rust core from JavaScript", () => {
  test("a full link handshake, then the ratchet", () => {
    const fragment = core.linkNew(false, 60, false, Date.now());
    expect(fragment).toMatch(/^2\.s\.[A-Za-z0-9_-]{43}\.[0-9a-z]+$/);
    const creator = new core.LinkSecrets(fragment, undefined);
    const joiner = new core.LinkSecrets(`https://x/#${fragment}`, undefined);
    const j = joiner.joinerStart(new TextEncoder().encode("hi")) as {
      state: LinkJoiner;
      hs1: Uint8Array;
    };
    const c1 = creator.creatorReadHs1(j.hs1) as { state: LinkCreatorPending; payload: Uint8Array };
    expect(new TextDecoder().decode(c1.payload)).toBe("hi");
    const c2 = c1.state.writeHs2(new Uint8Array([1])) as { state: LinkCreator; hs2: Uint8Array };
    const r = j.state.readHs2(c2.hs2) as { state: LinkJoinerReady; payload: Uint8Array };
    const done = r.state.writeHs3(new Uint8Array([2]), Date.now()) as EstablishedResult & {
      hs3: Uint8Array;
    };
    const end = c2.state.readHs3(done.hs3, Date.now()) as EstablishedResult & {
      payload: Uint8Array;
    };
    expect([...(done.sas ?? [])]).toEqual([...(end.sas ?? [])]);
    const frame = done.session.seal(1, new TextEncoder().encode("secret"));
    const opened = openFrame(end.session, frame, Date.now());
    expect(new TextDecoder().decode(opened.messages[0]?.data)).toBe("secret");
    expect(() => openFrame(end.session, frame, Date.now())).toThrow("Replay");
  });

  test("a wrong passphrase cannot open HS1, and errors are named", () => {
    const fragment = core.linkNew(false, 60, true, Date.now());
    const good = new core.LinkSecrets(fragment, "right one");
    const bad = new core.LinkSecrets(fragment, "wrong one");
    const j = bad.joinerStart(new Uint8Array()) as { hs1: Uint8Array };
    expect(() => good.creatorReadHs1(j.hs1)).toThrow("Decrypt");
    expect(() => new core.LinkSecrets(fragment, undefined)).toThrow("InvalidParameter");
  });

  test("identifiers are self-certifying and typos are caught", () => {
    const id = core.Identity.generate();
    expect(id.id).toMatch(/^hush1[a-z0-9]{59}$/);
    expect([...core.idDecode(id.id)]).toEqual([...id.idHash]);
    // A typo: another character in place of the 31st (never the same one, or nothing changes).
    const typo = id.id[30] === "q" ? "p" : "q";
    expect(() => core.idDecode(`${id.id.slice(0, 30)}${typo}${id.id.slice(31)}`)).toThrow();
    const again = core.Identity.fromSeed(id.seed());
    expect(again.id).toBe(id.id);
  });

  test("backups round-trip and refuse a wrong passphrase", () => {
    const file = core.backupExport("pass phrase", new TextEncoder().encode('{"v":1}'));
    expect(core.isBackup(file)).toBe(true);
    expect(new TextDecoder().decode(core.backupImport("pass phrase", file))).toBe('{"v":1}');
    expect(() => core.backupImport("nope", file)).toThrow("Decrypt");
  });
});
