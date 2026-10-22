import type { JSX } from "preact";
import { bootError, ready, view } from "../app/state";
import { Logo } from "./components/Icons";
import { t } from "./i18n";
import { applyUpdate, updateReady } from "./pwa";
import { ChatScreen } from "./screens/ChatScreen";
import { ContactsScreen } from "./screens/ContactsScreen";
import { Ended } from "./screens/Ended";
import { Home } from "./screens/Home";
import { IdentityScreen } from "./screens/IdentityScreen";
import { Join } from "./screens/Join";
import { NewConversation } from "./screens/NewConversation";
import { Passphrase } from "./screens/Passphrase";
import { SettingsScreen } from "./screens/SettingsScreen";

function Screen(): JSX.Element {
  const v = view.value;
  switch (v.name) {
    case "home":
      return <Home />;
    case "new":
      return <NewConversation />;
    case "join":
      return <Join {...(v.error ? { error: v.error } : {})} />;
    case "passphrase":
      return <Passphrase fragment={v.fragment} {...(v.error ? { error: v.error } : {})} />;
    case "chat":
      return <ChatScreen chat={v.chat} />;
    case "ended":
      return <Ended status={v.status} />;
    case "contacts":
      return <ContactsScreen />;
    case "identity":
      return <IdentityScreen />;
    case "settings":
      return <SettingsScreen />;
  }
}

export function App(): JSX.Element {
  if (bootError.value) {
    return (
      <main class="page center">
        <Logo size={48} />
        <p class="error">{bootError.value}</p>
      </main>
    );
  }
  if (!ready.value) {
    return (
      <main class="page center" aria-busy="true">
        <Logo size={48} />
      </main>
    );
  }
  return (
    <>
      {updateReady.value && (
        <div class="banner" role="status">
          <span>{t("update")}</span>
          <button type="button" class="btn btn-small" onClick={applyUpdate}>
            {t("reload")}
          </button>
        </div>
      )}
      <Screen />
    </>
  );
}
