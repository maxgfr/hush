/** The emoji comparison that proves nobody sits between the two people. */
import type { JSX } from "preact";
import type { Chat } from "../../app/chat";
import type { PeerConn } from "../../app/peer";
import { sasEmojis } from "../../core/wasm";
import { sasName, t } from "../i18n";
import { Icon } from "./Icons";

export function Verify(props: { chat: Chat; peer: PeerConn }): JSX.Element | null {
  const { peer } = props;
  if (!peer.sas) return null;
  const name = peer.name.value || t("them");
  if (peer.verifiedByMe.value && peer.verifiedByPeer.value) return null;
  const emojis = sasEmojis(peer.sas);
  return (
    <section class="verify card" aria-labelledby={`verify-${peer.name.value}`}>
      <div class="icon-title">
        <Icon name="shield" />
        <h2 id={`verify-${peer.name.value}`}>
          {t("verifyTitle")}
          {props.chat.group ? ` · ${name}` : ""}
        </h2>
      </div>
      <p class="hint">{t("verifyHint", { name })}</p>
      <ol class="sas" data-testid="sas">
        {emojis.map(([e, n], i) => (
          <li key={`${i}-${n}`}>
            <span class="sas-emoji" aria-hidden="true">
              {e}
            </span>
            <span class="sas-name">{sasName(peer.sas?.[i] ?? 0, n)}</span>
          </li>
        ))}
      </ol>
      {peer.verifiedByMe.value ? (
        <p class="meta">{t("waitingTheirCheck", { name })}</p>
      ) : (
        <div class="row center">
          <button
            type="button"
            class="btn btn-primary"
            onClick={() => props.chat.verify(peer, true)}
          >
            <Icon name="check" />
            {t("match")}
          </button>
          <button
            type="button"
            class="btn btn-danger-ghost"
            onClick={() => props.chat.verify(peer, false)}
          >
            <Icon name="x" />
            {t("noMatch")}
          </button>
        </div>
      )}
    </section>
  );
}
