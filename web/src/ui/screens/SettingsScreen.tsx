import type { JSX } from "preact";
import { useState } from "preact/hooks";
import { DEFAULT_SETTINGS, saveSettings, settings } from "../../app/settings";
import { go, transports } from "../../app/state";
import { Icon } from "../components/Icons";
import { lang, setLang, TTL_SECONDS, t, ttlLabel } from "../i18n";

const lines = (s: string) =>
  s
    .split(/\s+/)
    .map((x) => x.trim())
    .filter(Boolean);

export function SettingsScreen(): JSX.Element {
  const s = settings.value;
  const [relays, setRelays] = useState(s.relays.join("\n"));
  const [trackers, setTrackers] = useState(s.trackers.join("\n"));
  const [stun, setStun] = useState(s.stun.join("\n"));
  const [turn, setTurn] = useState(s.turn ?? { url: "", username: "", credential: "" });
  const [saved, setSaved] = useState(false);

  const save = (e: Event) => {
    e.preventDefault();
    saveSettings({
      relays: lines(relays).filter((u) => /^wss:\/\//.test(u)),
      trackers: lines(trackers).filter((u) => /^wss:\/\//.test(u)),
      stun: lines(stun).filter((u) => /^stuns?:/.test(u)),
      turn: turn.url ? turn : null,
    });
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };

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
        <h1 class="title">{t("settings")}</h1>
        <span />
      </header>
      <section class="card stack">
        <label class="field">
          <span>{t("displayName")}</span>
          <input
            type="text"
            maxLength={64}
            value={s.name}
            onInput={(e) => saveSettings({ name: (e.target as HTMLInputElement).value })}
          />
        </label>
        <label class="field">
          <span>{t("messageTimer")}</span>
          <select
            value={s.ttl}
            onChange={(e) => saveSettings({ ttl: Number((e.target as HTMLSelectElement).value) })}
          >
            {TTL_SECONDS.map((x) => (
              <option key={x} value={x}>
                {ttlLabel(x)}
              </option>
            ))}
          </select>
        </label>
        <label class="field">
          <span>{t("language")}</span>
          <select
            value={lang.value}
            onChange={(e) => setLang((e.target as HTMLSelectElement).value as "fr" | "en")}
          >
            <option value="fr">Français</option>
            <option value="en">English</option>
          </select>
        </label>
      </section>

      <form class="card stack" onSubmit={save}>
        <h2>{t("network")}</h2>
        <p class="hint">{t("networkHint")}</p>
        <ul class="transport-status">
          {transports.value.map((x) => (
            <li key={x.name} class={x.state}>
              <span class="dot" aria-hidden="true" />
              {x.name} · {x.state}
            </li>
          ))}
        </ul>
        <label class="check">
          <input
            type="checkbox"
            checked={s.nostr}
            onChange={(e) => saveSettings({ nostr: (e.target as HTMLInputElement).checked })}
          />
          <span>{t("relays")}</span>
        </label>
        <textarea
          rows={4}
          value={relays}
          spellcheck={false}
          onInput={(e) => setRelays((e.target as HTMLTextAreaElement).value)}
        />
        <label class="check">
          <input
            type="checkbox"
            checked={s.torrent}
            onChange={(e) => saveSettings({ torrent: (e.target as HTMLInputElement).checked })}
          />
          <span>{t("trackers")}</span>
        </label>
        <textarea
          rows={2}
          value={trackers}
          spellcheck={false}
          onInput={(e) => setTrackers((e.target as HTMLTextAreaElement).value)}
        />
        <label class="field">
          <span>{t("stun")}</span>
          <textarea
            rows={2}
            value={stun}
            spellcheck={false}
            onInput={(e) => setStun((e.target as HTMLTextAreaElement).value)}
          />
        </label>
        <fieldset class="field">
          <legend>{t("turn")}</legend>
          <input
            type="text"
            placeholder="turn:turn.example.org:3478"
            value={turn.url}
            onInput={(e) => setTurn({ ...turn, url: (e.target as HTMLInputElement).value })}
          />
          <input
            type="text"
            placeholder="username"
            autocomplete="off"
            value={turn.username}
            onInput={(e) => setTurn({ ...turn, username: (e.target as HTMLInputElement).value })}
          />
          <input
            type="password"
            placeholder="credential"
            autocomplete="off"
            value={turn.credential}
            onInput={(e) => setTurn({ ...turn, credential: (e.target as HTMLInputElement).value })}
          />
        </fieldset>
        <label class="check">
          <input
            type="checkbox"
            checked={s.relayOnly}
            onChange={(e) => saveSettings({ relayOnly: (e.target as HTMLInputElement).checked })}
          />
          <span>{t("relayOnly")}</span>
        </label>
        <div class="row">
          <button type="submit" class="btn btn-primary">
            {saved ? t("copied").replace(/.*/, "✓") : t("save")}
          </button>
          <button
            type="button"
            class="btn btn-ghost"
            onClick={() => {
              setRelays(DEFAULT_SETTINGS.relays.join("\n"));
              setTrackers(DEFAULT_SETTINGS.trackers.join("\n"));
              setStun(DEFAULT_SETTINGS.stun.join("\n"));
            }}
          >
            Reset
          </button>
        </div>
      </form>
    </main>
  );
}
