import { signal } from "@preact/signals";
import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { OutgoingDrop } from "../../app/drop";
import { blossomServers } from "../../app/settings";
import { go, view } from "../../app/state";
import { contacts, vaultState } from "../../app/store";
import { type DropEntry, entriesFromDrop, entriesFromFiles, MAX_TOTAL } from "../../drop/entries";
import { formatSize } from "../components/FileCard";
import { Icon } from "../components/Icons";
import { QrCode } from "../components/QrCode";
import { duration, LINK_MINUTES } from "../i18n";
import { t } from "../i18n-drop";

/** The drop being sent: it keeps going if the user looks at another screen meanwhile. */
const active = signal<OutgoingDrop | null>(null);

function Picker(props: { onAdd: (e: DropEntry[]) => void; big?: boolean }): JSX.Element {
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  useEffect(() => {
    // `webkitdirectory` is not in Preact's attribute types.
    folder.current?.setAttribute("webkitdirectory", "");
  }, []);
  const pick = (input: HTMLInputElement) => {
    props.onAdd(entriesFromFiles(input.files ?? []));
    input.value = "";
  };
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: a drop target; its buttons are the keyboard path.
    <div
      class={`dropzone${over ? " over" : ""}${props.big ? " big" : ""}`}
      data-testid="dropzone"
      onDragOver={(e) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        if (e.dataTransfer) void entriesFromDrop(e.dataTransfer).then(props.onAdd);
      }}
    >
      <Icon name="upload" size={props.big ? 36 : 24} />
      <p>{over ? t("dropToAdd") : t("dropHere")}</p>
      <div class="row center">
        <button type="button" class="btn" onClick={() => files.current?.click()}>
          <Icon name="file" size={18} />
          {t("pickFiles")}
        </button>
        <button type="button" class="btn" onClick={() => folder.current?.click()}>
          <Icon name="folder" size={18} />
          {t("pickFolder")}
        </button>
      </div>
      <input
        ref={files}
        type="file"
        multiple
        hidden
        data-testid="drop-files"
        onChange={(e) => pick(e.target as HTMLInputElement)}
      />
      <input
        ref={folder}
        type="file"
        multiple
        hidden
        data-testid="drop-folder"
        onChange={(e) => pick(e.target as HTMLInputElement)}
      />
    </div>
  );
}

function Compose(props: { initial: DropEntry[] }): JSX.Element {
  const [entries, setEntries] = useState<DropEntry[]>(props.initial);
  const [ttlMin, setTtlMin] = useState(1440);
  const [once, setOnce] = useState(true);
  const [pass, setPass] = useState("");
  const noServers = blossomServers().length === 0;
  const [p2pOnly, setP2pOnly] = useState(noServers);
  const [target, setTarget] = useState<"link" | "people">("link");
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [me, setMe] = useState(false);
  const total = entries.reduce((n, e) => n + e.file.size, 0);
  const add = (more: DropEntry[]) => {
    const seen = new Set(entries.map((e) => e.path.toLowerCase()));
    setEntries([...entries, ...more.filter((e) => !seen.has(e.path.toLowerCase()))]);
  };
  const unlocked = vaultState.value === "unlocked";
  const people = target === "people";
  const picked = contacts.value.filter((c) => chosen.has(c.key));
  const ready =
    entries.length > 0 &&
    total <= MAX_TOTAL &&
    (!people || (unlocked && (picked.length > 0 || me)));
  const send = (e: Event) => {
    e.preventDefault();
    if (!ready) return;
    const drop = new OutgoingDrop(entries, {
      ttlMin,
      once,
      p2pOnly: !people && p2pOnly,
      ...(pass && !people ? { passphrase: pass } : {}),
      target: people ? { kind: "people", contacts: picked, me } : { kind: "link" },
    });
    active.value = drop;
    void drop.start();
  };
  const folders = new Set(
    entries.filter((e) => e.path.includes("/")).map((e) => e.path.split("/")[0]),
  );
  return (
    <form class="stack" onSubmit={send}>
      <Picker onAdd={add} big={entries.length === 0} />
      {entries.length > 0 && (
        <section class="card stack">
          <p class="row-between">
            <strong>{t("filesSummary", { n: entries.length, size: formatSize(total) })}</strong>
            {total > MAX_TOTAL && <span class="error">≤ 4 GB</span>}
          </p>
          <ul class="drop-list">
            {entries.slice(0, 200).map((e) => (
              <li key={e.path}>
                <Icon name={folders.has(e.path.split("/")[0]) ? "folder" : "file"} size={16} />
                <span class="file-name">{e.path}</span>
                <span class="faint">{formatSize(e.file.size)}</span>
                <button
                  type="button"
                  class="btn btn-ghost btn-icon btn-small"
                  aria-label={t("remove")}
                  onClick={() => setEntries(entries.filter((x) => x !== e))}
                >
                  <Icon name="x" size={14} />
                </button>
              </li>
            ))}
            {entries.length > 200 && <li class="faint">… +{entries.length - 200}</li>}
          </ul>
        </section>
      )}
      <section class="card stack">
        <fieldset class="segmented" aria-label={t("dropFor")}>
          <label class={!people ? "on" : ""}>
            <input type="radio" name="to" checked={!people} onChange={() => setTarget("link")} />
            <Icon name="link" size={18} />
            {t("dropForLink")}
          </label>
          <label class={people ? "on" : ""}>
            <input type="radio" name="to" checked={people} onChange={() => setTarget("people")} />
            <Icon name="users" size={18} />
            {t("dropForPeople")}
          </label>
        </fieldset>
        {people ? (
          <div class="stack">
            <p class="hint">{t("dropPeopleHint")}</p>
            {!unlocked ? (
              <p class="callout">
                {t("dropNeedsContacts")}{" "}
                <button
                  type="button"
                  class="btn btn-small"
                  onClick={() => go({ name: "contacts" })}
                >
                  {t("contacts")}
                </button>
              </p>
            ) : (
              <>
                {contacts.value.length === 0 && <p class="faint">{t("dropNoContacts")}</p>}
                {contacts.value.map((c) => (
                  <label class="check" key={c.key}>
                    <input
                      type="checkbox"
                      checked={chosen.has(c.key)}
                      onChange={() => {
                        const next = new Set(chosen);
                        if (!next.delete(c.key)) next.add(c.key);
                        setChosen(next);
                      }}
                    />
                    <span>
                      {c.name || c.id.slice(0, 14)}{" "}
                      <small class="faint">{c.id.slice(0, 14)}…</small>
                    </span>
                  </label>
                ))}
                <label class="check">
                  <input type="checkbox" checked={me} onChange={() => setMe(!me)} />
                  <span>{t("dropForMe")}</span>
                </label>
              </>
            )}
          </div>
        ) : (
          <>
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
            <label class="check">
              <input
                type="checkbox"
                checked={p2pOnly}
                disabled={noServers}
                onChange={() => setP2pOnly(!p2pOnly)}
              />
              <span>{t("dropP2pOnly")}</span>
            </label>
          </>
        )}
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
        {!(p2pOnly && !people) && (
          <label class="check">
            <input type="checkbox" checked={once} onChange={() => setOnce(!once)} />
            <span>{t("dropOnce")}</span>
          </label>
        )}
        <p class="hint">
          {noServers && !people
            ? t("dropNoServers")
            : p2pOnly && !people
              ? t("dropP2pOnlyHint")
              : t("dropAutoHint")}
        </p>
        <button
          type="submit"
          class="btn btn-primary btn-large"
          disabled={!ready}
          data-testid="drop-send"
        >
          <Icon name="lock" />
          {t("sealAndSend")}
        </button>
      </section>
    </form>
  );
}

function Sending(props: { drop: OutgoingDrop }): JSX.Element {
  const d = props.drop;
  const phase = d.phase.value;
  const url = d.url.value;
  const pct = Math.round(d.progress.value * 100);
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const i = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(i);
  }, []);
  const copy = async () => {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // the link stays selectable
    }
  };
  const p2pOnly = d.opts.p2pOnly && !d.sealedToPeople;
  const again = () => {
    d.close();
    active.value = null;
  };
  return (
    <section class="card stack" aria-live="polite">
      {phase === "sealing" && (
        <div class="progress">
          <progress value={d.progress.value} max={1} />
          <small>{t("sealing", { p: pct })}</small>
        </div>
      )}
      {phase === "failed" && (
        <p class="error" role="alert">
          {t("dropFailedSend", { e: d.error.value ?? "" })}
        </p>
      )}
      {url && (
        <div class="share">
          <h2>{t("sendThisLink")}</h2>
          <div class="qr-wrap">
            <QrCode value={url} label={t("sendThisLink")} size={208} />
          </div>
          <output class="link-text" data-testid="drop-link">
            {url}
          </output>
          <div class="row center">
            <button type="button" class="btn btn-primary" onClick={copy}>
              <Icon name={copied ? "check" : "copy"} />
              {copied ? t("copied") : t("copy")}
            </button>
            {"share" in navigator && (
              <button
                type="button"
                class="btn"
                onClick={() => void navigator.share?.({ title: "hush", url }).catch(() => {})}
              >
                <Icon name="share" />
                {t("share")}
              </button>
            )}
          </div>
          <p class="meta">
            <Icon name="timer" size={16} />
            {t("expiresIn", { t: duration(d.expiresMs - now) })} ·{" "}
            {d.opts.once && !p2pOnly ? t("dropOnceShort") : t("dropManyShort")}
          </p>
          {d.opts.passphrase && <p class="callout">{t("sharePassphrase")}</p>}
          {d.sealedToPeople && <p class="hint">{t("sealedLinkHint")}</p>}
        </div>
      )}
      {phase === "uploading" && (
        <div class="progress">
          <progress value={d.progress.value} max={1} />
          <small>{t("uploadingCopy", { p: pct })}</small>
        </div>
      )}
      {phase === "ready" && !p2pOnly && !d.copyDeleted.value && (
        <p class="ok-line" data-testid="drop-ready">
          <Icon name="check" size={18} />
          {t("copyReady")}
          <small class="faint"> {t("copyOn", { servers: d.servers.value.join(", ") })}</small>
        </p>
      )}
      {!d.sealedToPeople && url && (
        <p class="status-line" data-testid="drop-p2p">
          <span class={`dot${d.peer.value === "none" ? "" : " on"}`} />
          {d.peer.value === "delivered"
            ? t("p2pDelivered")
            : d.peer.value === "sending"
              ? t("p2pSending")
              : p2pOnly
                ? t("p2pOnlyWaiting")
                : t("p2pWaiting")}
        </p>
      )}
      <div class="row center">
        {phase !== "failed" && phase !== "cancelled" && !p2pOnly && !d.copyDeleted.value && (
          <button type="button" class="btn btn-danger-ghost" onClick={() => void d.deleteCopy()}>
            <Icon name="trash" size={18} />
            {phase === "ready" ? t("deleteCopy") : t("cancelSend")}
          </button>
        )}
        {d.copyDeleted.value && <span class="faint">{t("copyDeleted")}</span>}
        <button type="button" class="btn" onClick={again}>
          <Icon name="plus" size={18} />
          {t("sendMore")}
        </button>
      </div>
    </section>
  );
}

export function DropScreen(): JSX.Element {
  const v = view.value;
  const initial = v.name === "drop" ? (v.entries ?? []) : [];
  const drop = active.value;
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
        <h1 class="title">{t("sendFiles")}</h1>
        <span />
      </header>
      {drop ? <Sending drop={drop} /> : <Compose initial={initial} />}
    </main>
  );
}
