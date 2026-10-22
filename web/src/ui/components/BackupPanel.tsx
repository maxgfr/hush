import type { JSX } from "preact";
import { useRef, useState } from "preact/hooks";
import { contactsManager } from "../../app/state";
import { exportBackup, importBackup } from "../../app/store";
import { t } from "../i18n";
import { Icon } from "./Icons";

export function BackupPanel(): JSX.Element {
  const [pass, setPass] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const run = (fn: () => Promise<void>) => {
    setBusy(true);
    setMsg(null);
    setTimeout(() => {
      void fn().finally(() => setBusy(false));
    }, 30);
  };

  const doExport = () =>
    run(async () => {
      const blob = await exportBackup(pass);
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = `hush-backup-${new Date().toISOString().slice(0, 10)}.hush`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
    });

  const doImport = (file: File) =>
    run(async () => {
      try {
        const n = await importBackup(new Uint8Array(await file.arrayBuffer()), pass);
        contactsManager().watch();
        setMsg({ ok: true, text: t("backupDone", { n }) });
      } catch {
        setMsg({ ok: false, text: t("backupBad") });
      }
    });

  return (
    <section class="card stack">
      <h2>{t("backup")}</h2>
      <p class="hint">{t("backupIntro")}</p>
      <label class="field">
        <span>{t("backupPassword")}</span>
        <input
          type="password"
          autocomplete="new-password"
          value={pass}
          onInput={(e) => setPass((e.target as HTMLInputElement).value)}
        />
      </label>
      <input
        ref={input}
        type="file"
        hidden
        accept=".hush,application/octet-stream"
        onChange={(e) => {
          const f = (e.target as HTMLInputElement).files?.[0];
          if (f) doImport(f);
          (e.target as HTMLInputElement).value = "";
        }}
      />
      <div class="row">
        <button type="button" class="btn" disabled={!pass || busy} onClick={doExport}>
          <Icon name="download" />
          {t("backupExport")}
        </button>
        <button
          type="button"
          class="btn"
          disabled={!pass || busy}
          onClick={() => input.current?.click()}
        >
          <Icon name="share" />
          {t("backupImport")}
        </button>
      </div>
      {msg && <p class={msg.ok ? "ok" : "error"}>{msg.text}</p>}
    </section>
  );
}
