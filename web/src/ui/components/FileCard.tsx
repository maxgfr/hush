import type { JSX } from "preact";
import type { Chat } from "../../app/chat";
import { type FileItem, PREVIEWABLE } from "../../app/files";
import { t } from "../i18n";
import { Icon } from "./Icons";

export function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

export function FileCard(props: { chat: Chat; file: FileItem }): JSX.Element {
  const f = props.file;
  const state = f.state.value;
  const url = f.url.value;
  const image = f.mime.startsWith("image/") && url && PREVIEWABLE.test(f.mime);
  return (
    <div class="file-card">
      {image && <img src={url} alt={f.name} class="file-preview" />}
      <div class="file-row">
        <Icon name="file" />
        <div class="file-meta">
          <span class="file-name">{f.path ?? f.name}</span>
          <span class="faint">
            {formatSize(f.size)}
            {f.sealed.value && ` · ${t("viaCopy")}`}
            {f.delivered.value && ` · ${t("delivered")} ✓`}
          </span>
        </div>
        {state === "offered" && (
          <button type="button" class="btn btn-small" onClick={() => props.chat.download(f)}>
            <Icon name="download" size={16} />
            {t("download")}
          </button>
        )}
        {(state === "done" || state === "sent") && url && (
          <a class="btn btn-small" href={url} download={f.name}>
            <Icon name="download" size={16} />
          </a>
        )}
      </div>
      {state === "downloading" && (
        <div class="progress">
          <progress value={f.progress.value} max={1} />
          <small>{t("downloading", { p: Math.round(f.progress.value * 100) })}</small>
        </div>
      )}
      {state === "sending" && <small class="faint">{t("sending")}</small>}
      {state === "failed" && (
        <small class="error">
          {f.owner && !f.owner.rtc?.isOpen ? t("directNeeded") : t("fileFailed")}
        </small>
      )}
    </div>
  );
}
