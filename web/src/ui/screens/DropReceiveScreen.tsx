import type { JSX } from "preact";
import { useEffect, useState } from "preact/hooks";
import { startOver, view } from "../../app/state";
import { contacts } from "../../app/store";
import { formatSize } from "../components/FileCard";
import { Icon } from "../components/Icons";
import { duration } from "../i18n";
import { type DropKey, t } from "../i18n-drop";

export function DropReceiveScreen(): JSX.Element {
  const v = view.value;
  const drop = v.name === "dropReceive" ? v.drop : null;
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const i = setInterval(() => setNow(Date.now()), 30_000);
    return () => {
      clearInterval(i);
      drop?.close();
    };
  }, [drop]);
  if (!drop) return <main class="page center" />;
  const state = drop.state.value;
  const files = drop.files();
  const m = drop.manifest.value;
  const total = files.reduce((n, f) => n + f.size, 0);
  const expires = m?.expiresMs ?? drop.expiresMs;
  const sender = drop.sender.value;
  const known = sender ? contacts.value.find((c) => c.id === sender) : undefined;
  const error = drop.error.value;
  const pct = Math.round(drop.progress.value * 100);
  // A drop without its copy (or not opened yet) can still come from the sender directly.
  const waiting = files.length === 0 && state !== "failed";
  return (
    <main class="page narrow">
      <header class="topbar">
        <button
          type="button"
          class="btn btn-ghost btn-icon"
          onClick={startOver}
          aria-label={t("back")}
        >
          <Icon name="arrowLeft" />
        </button>
        <h1 class="title">{t("filesForYou")}</h1>
        <span />
      </header>
      <section class="card stack" aria-live="polite" data-testid="drop-receive">
        {sender && (
          <p class="verified">
            <Icon name="shield" size={18} />
            {t("dropFrom", { id: known ? `${known.name} · ${sender.slice(0, 14)}…` : sender })}
          </p>
        )}
        {state === "opening" && !error && <p class="faint">{t("opening")}</p>}
        {waiting && state !== "opening" && <p class="faint">{t("dropWaitingSender")}</p>}
        {files.length > 0 && (
          <>
            <p class="row-between">
              <strong>{t("filesSummary", { n: files.length, size: formatSize(total) })}</strong>
              {expires > 0 && (
                <small class="faint">{t("availableFor", { t: duration(expires - now) })}</small>
              )}
            </p>
            <ul class="drop-list" data-testid="drop-files-list">
              {files.slice(0, 200).map((f) => (
                <li key={f.path}>
                  <Icon name="file" size={16} />
                  <span class="file-name">{f.path}</span>
                  <span class="faint">{formatSize(f.size)}</span>
                </li>
              ))}
              {files.length > 200 && <li class="faint">… +{files.length - 200}</li>}
            </ul>
            {m?.once && <p class="hint">{t("dropOnceShort")}</p>}
          </>
        )}
        <p class="status-line">
          <span class={`dot${drop.online.value || drop.source.value === "p2p" ? " on" : ""}`} />
          {drop.source.value === "p2p" || (!drop.source.value && drop.online.value)
            ? t("dropSenderOnline")
            : m
              ? t("dropFromCopy")
              : t("dropWaitingSender")}
        </p>
        {state === "downloading" && (
          <div class="progress">
            <progress value={drop.progress.value} max={1} />
            <small>{t("saving", { p: pct })}</small>
          </div>
        )}
        {state === "done" && (
          <p class="ok-line" data-testid="drop-saved">
            <Icon name="check" size={18} />
            {t("savedVerified")}
          </p>
        )}
        {state === "done" && drop.deleted.value && <p class="hint">{t("dropDeletedNote")}</p>}
        {error && (
          <p class="error" role="alert">
            {t(error as DropKey)}
          </p>
        )}
        {files.length > 0 && state !== "done" && (
          <button
            type="button"
            class="btn btn-primary btn-large"
            disabled={state === "downloading"}
            data-testid="drop-download"
            onClick={() => void drop.save()}
          >
            <Icon name="download" />
            {files.length === 1 ? t("download") : t("downloadAll")}
          </button>
        )}
      </section>
    </main>
  );
}
