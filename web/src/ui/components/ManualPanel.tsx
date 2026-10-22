/** Offline signaling: show our latest handshake code as a QR, scan or paste theirs. */
import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { manualCodes, rendezvous } from "../../app/state";
import { t } from "../i18n";
import { QrCode } from "./QrCode";
import { QrScanner } from "./QrScanner";

export function ManualPanel(): JSX.Element {
  const [open, setOpen] = useState(false);
  const [scan, setScan] = useState(false);
  const [paste, setPaste] = useState("");
  const [error, setError] = useState(false);
  const code = manualCodes.value.at(-1);
  const ingest = (text: string) => {
    const ok = rendezvous().ingestManual(text);
    setError(!ok);
    if (ok) setPaste("");
  };
  if (!open) {
    return (
      <button
        type="button"
        class="btn btn-ghost btn-small manual-toggle"
        onClick={() => setOpen(true)}
      >
        {t("offline")}
      </button>
    );
  }
  return (
    <section class="manual card stack" aria-label={t("offline")}>
      <p class="hint">{t("offlineHint")}</p>
      <h3>{t("yourCode")}</h3>
      {code ? (
        <>
          <div class="qr-wrap">
            <QrCode value={code} label={t("yourCode")} size={260} />
          </div>
          <textarea class="code" readOnly rows={3} value={code} data-testid="manual-code" />
        </>
      ) : (
        <p class="faint">{t("noCodeYet")}</p>
      )}
      {scan ? (
        <QrScanner
          onResult={(text) => {
            setScan(false);
            ingest(text);
          }}
          onCancel={() => setScan(false)}
        />
      ) : (
        <>
          <label class="field">
            <span>{t("pasteCode")}</span>
            <textarea
              rows={3}
              value={paste}
              spellcheck={false}
              data-testid="manual-input"
              onInput={(e) => setPaste((e.target as HTMLTextAreaElement).value)}
            />
          </label>
          {error && <p class="error">{t("invalidCode")}</p>}
          <div class="row">
            <button
              type="button"
              class="btn btn-primary"
              disabled={!paste.trim()}
              onClick={() => ingest(paste)}
            >
              {t("join")}
            </button>
            <button type="button" class="btn" onClick={() => setScan(true)}>
              {t("scanCode")}
            </button>
          </div>
        </>
      )}
    </section>
  );
}
