import type { JSX } from "preact";
import type { ChatStatus } from "../../app/chat";
import { go, startOver } from "../../app/state";
import { vaultState } from "../../app/store";
import { Icon } from "../components/Icons";
import { t } from "../i18n";

export function Ended(props: { status: ChatStatus }): JSX.Element {
  const warn = props.status === "taken";
  const title =
    props.status === "left"
      ? t("left")
      : props.status === "taken"
        ? t("taken")
        : props.status === "expired"
          ? t("expired")
          : t("ended");
  const kept = vaultState.value === "unlocked";
  return (
    <main class="page narrow center ended">
      <div class={warn ? "badge-circle warn" : "badge-circle"}>
        <Icon name={warn ? "alert" : "check"} size={32} />
      </div>
      <h1>{title}</h1>
      {warn && <p class="callout warn">{t("takenHint")}</p>}
      <p class="lead">{kept ? t("wipedKept") : t("wiped")}</p>
      <button
        type="button"
        class="btn btn-primary btn-large"
        // With an unlocked vault, contacts stay connected: go home without reloading.
        onClick={() => (kept ? go({ name: "home" }) : startOver())}
      >
        {t("startOver")}
      </button>
    </main>
  );
}
