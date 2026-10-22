/** Create or unlock the vault before anything that needs to be kept. */
import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { contactsManager } from "../../app/state";
import { createVault, unlockVault, vaultState } from "../../app/store";
import { t } from "../i18n";
import { Icon } from "./Icons";

const MIN = 10;

export function VaultGate(): JSX.Element {
  const creating = vaultState.value === "none";
  const [pass, setPass] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = (e: Event) => {
    e.preventDefault();
    if (creating && pass.length < MIN) return setError(t("vaultTooShort"));
    if (creating && pass !== confirm) return setError(t("vaultMismatch"));
    setBusy(true);
    setError(null);
    // Argon2id (64 MiB) takes a second: let the button repaint first.
    setTimeout(async () => {
      try {
        await (creating ? createVault(pass) : unlockVault(pass));
        contactsManager().watch();
      } catch {
        setError(creating ? t("vaultTooShort") : t("vaultWrong"));
      } finally {
        setBusy(false);
      }
    }, 30);
  };

  return (
    <form class="card stack" onSubmit={submit}>
      <div class="icon-title">
        <Icon name="lock" size={28} />
        <h2>{t("vaultTitle")}</h2>
      </div>
      {creating && <p class="hint">{t("vaultIntro")}</p>}
      <label class="field">
        <span>{t("vaultPassword")}</span>
        <input
          type="password"
          autocomplete={creating ? "new-password" : "current-password"}
          value={pass}
          data-testid="vault-pass"
          onInput={(e) => setPass((e.target as HTMLInputElement).value)}
        />
        {creating && <small class="hint">{t("vaultPasswordHint")}</small>}
      </label>
      {creating && (
        <label class="field">
          <span>{t("vaultPasswordConfirm")}</span>
          <input
            type="password"
            autocomplete="new-password"
            value={confirm}
            data-testid="vault-confirm"
            onInput={(e) => setConfirm((e.target as HTMLInputElement).value)}
          />
        </label>
      )}
      {error && <p class="error">{error}</p>}
      <button
        type="submit"
        class="btn btn-primary btn-large"
        disabled={busy || !pass}
        aria-busy={busy}
      >
        {busy ? t("vaultWorking") : creating ? t("vaultCreate") : t("vaultUnlock")}
      </button>
    </form>
  );
}
