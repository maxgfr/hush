import type { ComponentType, JSX } from "preact";
import { useEffect, useState } from "preact/hooks";
import { bootError, ready, view } from "../app/state";
import { Logo } from "./components/Icons";
import { t } from "./i18n";
import { applyUpdate, updateReady } from "./pwa";
import { ChatScreen } from "./screens/ChatScreen";
import { Ended } from "./screens/Ended";
import { Home } from "./screens/Home";
import { Join } from "./screens/Join";
import { NewConversation } from "./screens/NewConversation";
import { Passphrase } from "./screens/Passphrase";

/** Screens off the main path (contacts, identity, settings) load on first use. */
function lazy(load: () => Promise<ComponentType>): () => JSX.Element {
  let loaded: ComponentType | null = null;
  return function Lazy(): JSX.Element {
    const [C, setC] = useState<ComponentType | null>(() => loaded);
    useEffect(() => {
      if (!C)
        void load().then((c) => {
          loaded = c;
          setC(() => c);
        });
    }, [C]);
    return C ? <C /> : <main class="page center" aria-busy="true" />;
  };
}

const ContactsScreen = lazy(() => import("./screens/ContactsScreen").then((m) => m.ContactsScreen));
const IdentityScreen = lazy(() => import("./screens/IdentityScreen").then((m) => m.IdentityScreen));
const SettingsScreen = lazy(() => import("./screens/SettingsScreen").then((m) => m.SettingsScreen));
const DropScreen = lazy(() => import("./screens/DropScreen").then((m) => m.DropScreen));
const DropReceiveScreen = lazy(() =>
  import("./screens/DropReceiveScreen").then((m) => m.DropReceiveScreen),
);

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
      return (
        <Passphrase
          fragment={v.fragment}
          {...(v.error ? { error: v.error } : {})}
          {...(v.drop ? { drop: true } : {})}
        />
      );
    case "drop":
      return <DropScreen />;
    case "dropReceive":
      return <DropReceiveScreen />;
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
