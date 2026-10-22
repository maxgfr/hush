import type { JSX } from "preact";
import { useEffect, useState } from "preact/hooks";
import { duration, t } from "../i18n";
import { Icon } from "./Icons";
import { QrCode } from "./QrCode";

export function ShareLink(props: {
  url: string;
  expiresMs: number;
  group: boolean;
  passphrase: boolean;
}): JSX.Element {
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const i = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(i);
  }, []);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard refused: the text stays selectable
    }
  };
  const share = () => void navigator.share?.({ title: "hush", url: props.url }).catch(() => {});
  return (
    <section class="share" aria-labelledby="share-title">
      <h2 id="share-title">{t("sendThisLink")}</h2>
      <p class="hint">{t("sendThisLinkHint")}</p>
      <div class="qr-wrap">
        <QrCode value={props.url} label={t("sendThisLink")} size={208} />
      </div>
      <output class="link-text" data-testid="share-link">
        {props.url}
      </output>
      <div class="row center">
        <button type="button" class="btn btn-primary" onClick={copy}>
          <Icon name={copied ? "check" : "copy"} />
          {copied ? t("copied") : t("copy")}
        </button>
        {"share" in navigator && (
          <button type="button" class="btn" onClick={share}>
            <Icon name="share" />
            {t("share")}
          </button>
        )}
      </div>
      <p class="meta">
        <Icon name="timer" size={16} />
        {t("expiresIn", { t: duration(props.expiresMs - now) })} ·{" "}
        {props.group ? t("anyoneWithLink") : t("singleUse")}
      </p>
      {props.passphrase && <p class="callout">{t("sharePassphrase")}</p>}
    </section>
  );
}
