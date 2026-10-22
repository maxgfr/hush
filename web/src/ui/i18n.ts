/** UI strings, French and English. The language follows the browser, and can be switched. */
import { signal } from "@preact/signals";

const fr = {
  tagline: "Parlez sans laisser de trace.",
  noServer: "Aucun serveur, aucun compte, rien de conservé",
  reset: "Réinitialiser",
  saved: "Enregistré",
  intro:
    "Créez un lien, envoyez-le, discutez. Chiffré de bout en bout avec une cryptographie post-quantique, sans compte ni serveur. Quand vous terminez, tout disparaît.",
  newConversation: "Nouvelle conversation",
  joinWithLink: "Rejoindre avec un lien",
  contacts: "Contacts",
  myId: "Mon identifiant",
  settings: "Paramètres",
  security: "Sécurité",
  source: "Code source",
  kindDirect: "À deux",
  kindDirectHint:
    "La première personne qui ouvre le lien le prend. Personne d'autre ne peut l'utiliser ensuite.",
  kindGroup: "Groupe",
  kindGroupHint:
    "Toute personne qui a le lien peut rejoindre, jusqu'à ce que vous verrouilliez le salon.",
  linkExpiry: "Le lien expire après",
  passphrase: "Mot de passe (facultatif)",
  passphraseHint: "Envoyez-le par un autre moyen que le lien : sans lui, le lien ne sert à rien.",
  messageTimer: "Messages éphémères",
  createLink: "Créer le lien",
  cancel: "Annuler",
  back: "Retour",
  sendThisLink: "Envoyez ce lien",
  sendThisLinkHint:
    "Par la messagerie de votre choix. Il ne contient aucune donnée personnelle, seulement une clé.",
  copy: "Copier",
  copied: "Copié",
  share: "Partager",
  expiresIn: "Expire dans {t}",
  singleUse: "usage unique",
  anyoneWithLink: "toute personne ayant le lien peut rejoindre",
  sharePassphrase: "Communiquez le mot de passe par un autre canal (de vive voix, SMS…).",
  waitingPeer: "En attente de votre correspondant…",
  waitingCreator: "En attente de la personne qui a créé le lien…",
  waitingPassHint: "Si l'attente dure, vérifiez le mot de passe.",
  connecting: "Établissement de la connexion chiffrée…",
  offline: "Pas d'Internet ? Échangez des codes QR",
  offlineHint:
    "Sans réseau, chaque message de connexion s'échange à la main : montrez votre code, scannez celui de l'autre. Il faut être sur le même Wi-Fi ou partage de connexion.",
  yourCode: "Votre code à montrer",
  scanCode: "Scanner un code",
  pasteCode: "Coller un code",
  noCodeYet: "Aucun code pour l'instant.",
  invalidCode: "Ce n'est pas un code hush.",
  pasteLink: "Collez le lien reçu",
  scanQr: "Scanner un QR code",
  join: "Rejoindre",
  invalidLink: "Ce lien n'est pas valide.",
  expiredLink: "Ce lien a expiré.",
  protectedLink: "Ce lien est protégé par un mot de passe",
  protectedLinkHint: "La personne qui vous l'a envoyé doit vous le donner par un autre moyen.",
  deriving: "Dérivation de la clé…",
  them: "Votre correspondant",
  someone: "Quelqu'un",
  you: "Vous",
  direct: "Direct",
  relay: "Via relais",
  connectingShort: "Connexion…",
  e2ePq: "chiffré post-quantique",
  endConversation: "Terminer",
  endConfirm: "Terminer la conversation ? Les clés et les messages seront effacés des deux côtés.",
  verifyTitle: "Vérifiez la connexion",
  verifyHint:
    "Comparez ces emojis avec {name}, de vive voix ou en personne. S'ils sont identiques, personne ne s'est glissé entre vous.",
  match: "Identiques",
  noMatch: "Différents",
  waitingTheirCheck: "En attente de la confirmation de {name}…",
  verified: "Vérifié",
  notVerified: "Non vérifié",
  messagePlaceholder: "Message",
  send: "Envoyer",
  attach: "Joindre un fichier",
  directNeeded: "Les fichiers demandent une connexion directe.",
  download: "Télécharger",
  downloading: "Réception… {p} %",
  sending: "Préparation…",
  fileFailed: "Échec : fichier corrompu ou connexion perdue.",
  timerOff: "Désactivé",
  timerOffShort: "aucun",
  timerLabel: "Minuteur : {t}",
  invite: "Inviter",
  members: "{n} participants",
  lock: "Verrouiller le salon",
  unlock: "Déverrouiller le salon",
  locked: "Salon verrouillé : plus personne ne peut rejoindre.",
  ended: "Conversation terminée",
  left: "Votre correspondant a terminé la conversation",
  taken: "Ce lien a déjà été utilisé",
  takenHint:
    "Quelqu'un d'autre l'a ouvert avant vous. Si ce n'était pas votre correspondant, le lien a été intercepté : prévenez-le par un autre moyen et n'utilisez plus ce lien.",
  expired: "Ce lien a expiré",
  wiped: "Les clés ont été effacées de cet appareil. Rien n'a été enregistré.",
  wipedKept:
    "Les clés de session ont été effacées. Seul ce que vous avez choisi d'enregistrer est conservé, chiffré.",
  startOver: "Nouvelle conversation",
  saveContact: "Enregistrer le contact",
  saveContactHint:
    "Pour vous reconnecter plus tard sans nouveau lien. Votre correspondant doit aussi accepter. Vos contacts sont chiffrés par votre mot de passe.",
  contactSaved: "{name} est dans vos contacts.",
  contactAsked: "{name} veut vous enregistrer comme contact.",
  contactWaiting: "Demande envoyée, en attente de {name}.",
  verifyFirst: "Vérifiez d'abord les emojis.",
  // Vault
  vaultTitle: "Votre coffre",
  vaultIntro:
    "Rien n'est enregistré par défaut. Pour garder des contacts ou un historique, créez un coffre : tout y est chiffré par un mot de passe que vous seul connaissez.",
  vaultCreate: "Créer le coffre",
  vaultUnlock: "Déverrouiller",
  vaultLock: "Verrouiller",
  vaultPassword: "Mot de passe du coffre",
  vaultPasswordConfirm: "Confirmez le mot de passe",
  vaultPasswordHint: "Au moins 10 caractères. Il ne peut pas être récupéré : notez-le.",
  vaultMismatch: "Les mots de passe ne correspondent pas.",
  vaultTooShort: "Mot de passe trop court.",
  vaultWrong: "Mot de passe incorrect.",
  vaultWorking: "Chiffrement…",
  vaultDelete: "Effacer le coffre",
  vaultDeleteConfirm:
    "Effacer définitivement le coffre de cet appareil ? Pensez à faire une sauvegarde.",
  noContacts:
    "Aucun contact. Après une conversation vérifiée, choisissez « Enregistrer le contact ».",
  connect: "Discuter",
  online: "en ligne",
  offlineContact: "hors ligne",
  removeContact: "Supprimer",
  removeContactConfirm: "Supprimer {name} de vos contacts ?",
  history: "Historique",
  historyHint: "Garder les messages de ce contact dans le coffre (chiffrés).",
  addById: "Ajouter par identifiant",
  addByIdHint: "Collez l'identifiant hush1… de la personne. Elle doit être en ligne et joignable.",
  idPlaceholder: "hush1…",
  invalidId: "Identifiant invalide (faute de frappe ?).",
  searching: "Recherche de {id}…",
  requestSent: "Demande envoyée. En attente de son accord…",
  notFound: "Personne n'a répondu. La personne doit avoir ouvert hush et activé « Joignable ».",
  requests: "Demandes de contact",
  accept: "Accepter",
  decline: "Refuser",
  wantsToConnect: "{name} ({id}) veut vous contacter.",
  // Identity
  idTitle: "Votre identifiant",
  idIntro:
    "Pas d'inscription : cet identifiant est calculé sur votre appareil à partir de vos clés publiques (Ed25519 + ML-DSA-65). Personne ne peut l'usurper.",
  idCreate: "Créer mon identifiant",
  reachable: "Joignable par mon identifiant",
  reachableHint:
    "Tant que hush est ouvert, les personnes qui connaissent votre identifiant peuvent vous envoyer une demande. Désactivé, personne ne peut vous trouver.",
  displayName: "Nom affiché",
  displayNameHint: "Montré à vos correspondants. Facultatif.",
  // Backup
  backup: "Sauvegarde",
  backupIntro:
    "Un fichier chiffré contenant votre identifiant, vos contacts et l'historique gardé. Il s'ouvre ici ou avec la CLI (hush backup import).",
  backupExport: "Exporter une sauvegarde",
  backupImport: "Importer une sauvegarde",
  backupPassword: "Mot de passe de la sauvegarde",
  backupDone: "Sauvegarde importée : {n} contacts.",
  backupBad: "Mot de passe incorrect ou fichier modifié.",
  // Settings
  network: "Réseau",
  relays: "Relais Nostr",
  trackers: "Trackers WebTorrent",
  stun: "Serveurs STUN",
  turn: "Serveur TURN (facultatif)",
  relayOnly: "Toujours passer par TURN (masque votre IP à votre correspondant)",
  networkHint:
    "Les relais et trackers ne voient qu'un sujet qui change chaque jour et du chiffré. Ils voient votre adresse IP, comme tout site web.",
  save: "Enregistrer",
  language: "Langue",
  update: "Une nouvelle version est prête.",
  reload: "Mettre à jour",
  close: "Fermer",
  newSessionStarted: "Session chiffrée établie (X25519 + ML-KEM-768).",
  joinedSuffix: "a rejoint",
  leftSuffix: "est parti",
  mismatchClosed: "Les emojis ne correspondaient pas : connexion fermée.",
} as const;

export type Key = keyof typeof fr;

const en: Record<Key, string> = {
  tagline: "Talk without leaving a trace.",
  noServer: "No server, no account, nothing kept",
  reset: "Reset",
  saved: "Saved",
  intro:
    "Create a link, send it, talk. End-to-end encrypted with post-quantum cryptography, no account, no server. When you end it, everything is gone.",
  newConversation: "New conversation",
  joinWithLink: "Join with a link",
  contacts: "Contacts",
  myId: "My identifier",
  settings: "Settings",
  security: "Security",
  source: "Source code",
  kindDirect: "One-to-one",
  kindDirectHint: "The first person to open the link takes it. Nobody else can use it afterwards.",
  kindGroup: "Group",
  kindGroupHint: "Anyone holding the link can join, until you lock the room.",
  linkExpiry: "Link expires after",
  passphrase: "Passphrase (optional)",
  passphraseHint: "Send it through another channel than the link: without it, the link is useless.",
  messageTimer: "Disappearing messages",
  createLink: "Create the link",
  cancel: "Cancel",
  back: "Back",
  sendThisLink: "Send this link",
  sendThisLinkHint: "Through any messenger. It holds no personal data, only a key.",
  copy: "Copy",
  copied: "Copied",
  share: "Share",
  expiresIn: "Expires in {t}",
  singleUse: "single use",
  anyoneWithLink: "anyone with the link can join",
  sharePassphrase: "Give the passphrase through another channel (voice, text message…).",
  waitingPeer: "Waiting for the other person…",
  waitingCreator: "Waiting for the person who made the link…",
  waitingPassHint: "If this lasts, check the passphrase.",
  connecting: "Setting up the encrypted connection…",
  offline: "No Internet? Swap QR codes",
  offlineHint:
    "Without a network, each connection message is carried by hand: show your code, scan theirs. You must be on the same Wi-Fi or hotspot.",
  yourCode: "Your code to show",
  scanCode: "Scan a code",
  pasteCode: "Paste a code",
  noCodeYet: "No code yet.",
  invalidCode: "This is not a hush code.",
  pasteLink: "Paste the link you received",
  scanQr: "Scan a QR code",
  join: "Join",
  invalidLink: "This link is not valid.",
  expiredLink: "This link has expired.",
  protectedLink: "This link is protected by a passphrase",
  protectedLinkHint: "Whoever sent it must give it to you another way.",
  deriving: "Deriving the key…",
  them: "The other person",
  someone: "Someone",
  you: "You",
  direct: "Direct",
  relay: "Via relays",
  connectingShort: "Connecting…",
  e2ePq: "post-quantum encrypted",
  endConversation: "End",
  endConfirm: "End the conversation? Keys and messages are erased on both sides.",
  verifyTitle: "Verify the connection",
  verifyHint:
    "Compare these emojis with {name}, by voice or in person. If they match, nobody is sitting between you.",
  match: "They match",
  noMatch: "They differ",
  waitingTheirCheck: "Waiting for {name} to confirm…",
  verified: "Verified",
  notVerified: "Not verified",
  messagePlaceholder: "Message",
  send: "Send",
  attach: "Attach a file",
  directNeeded: "Files need a direct connection.",
  download: "Download",
  downloading: "Receiving… {p}%",
  sending: "Preparing…",
  fileFailed: "Failed: corrupt file or lost connection.",
  timerOff: "Off",
  timerOffShort: "off",
  timerLabel: "Timer: {t}",
  invite: "Invite",
  members: "{n} people",
  lock: "Lock the room",
  unlock: "Unlock the room",
  locked: "Room locked: nobody else can join.",
  ended: "Conversation ended",
  left: "The other person ended the conversation",
  taken: "This link was already used",
  takenHint:
    "Someone else opened it before you. If it was not the person you expected, the link was intercepted: warn them another way and stop using it.",
  expired: "This link has expired",
  wiped: "Keys were erased from this device. Nothing was saved.",
  wipedKept: "Session keys were erased. Only what you chose to save is kept, encrypted.",
  startOver: "New conversation",
  saveContact: "Save contact",
  saveContactHint:
    "To reconnect later without a new link. The other person must accept too. Contacts are encrypted with your passphrase.",
  contactSaved: "{name} is in your contacts.",
  contactAsked: "{name} wants to save you as a contact.",
  contactWaiting: "Request sent, waiting for {name}.",
  verifyFirst: "Verify the emojis first.",
  vaultTitle: "Your vault",
  vaultIntro:
    "Nothing is saved by default. To keep contacts or history, create a vault: everything in it is encrypted with a passphrase only you know.",
  vaultCreate: "Create the vault",
  vaultUnlock: "Unlock",
  vaultLock: "Lock",
  vaultPassword: "Vault passphrase",
  vaultPasswordConfirm: "Confirm the passphrase",
  vaultPasswordHint: "At least 10 characters. It cannot be recovered: write it down.",
  vaultMismatch: "Passphrases do not match.",
  vaultTooShort: "Passphrase too short.",
  vaultWrong: "Wrong passphrase.",
  vaultWorking: "Encrypting…",
  vaultDelete: "Erase the vault",
  vaultDeleteConfirm: "Permanently erase the vault from this device? Make a backup first.",
  noContacts: "No contacts. After a verified conversation, choose “Save contact”.",
  connect: "Talk",
  online: "online",
  offlineContact: "offline",
  removeContact: "Remove",
  removeContactConfirm: "Remove {name} from your contacts?",
  history: "History",
  historyHint: "Keep this contact's messages in the vault (encrypted).",
  addById: "Add by identifier",
  addByIdHint: "Paste the person's hush1… identifier. They must be online and reachable.",
  idPlaceholder: "hush1…",
  invalidId: "Invalid identifier (typo?).",
  searching: "Looking for {id}…",
  requestSent: "Request sent. Waiting for them to accept…",
  notFound: "Nobody answered. The person must have hush open with “Reachable” on.",
  requests: "Contact requests",
  accept: "Accept",
  decline: "Decline",
  wantsToConnect: "{name} ({id}) wants to contact you.",
  idTitle: "Your identifier",
  idIntro:
    "No sign-up: this identifier is computed on your device from your public keys (Ed25519 + ML-DSA-65). Nobody can impersonate it.",
  idCreate: "Create my identifier",
  reachable: "Reachable by my identifier",
  reachableHint:
    "While hush is open, people who know your identifier can send you a request. Off, nobody can find you.",
  displayName: "Display name",
  displayNameHint: "Shown to the people you talk to. Optional.",
  backup: "Backup",
  backupIntro:
    "An encrypted file with your identifier, contacts and kept history. It opens here or with the CLI (hush backup import).",
  backupExport: "Export a backup",
  backupImport: "Import a backup",
  backupPassword: "Backup passphrase",
  backupDone: "Backup imported: {n} contacts.",
  backupBad: "Wrong passphrase or modified file.",
  network: "Network",
  relays: "Nostr relays",
  trackers: "WebTorrent trackers",
  stun: "STUN servers",
  turn: "TURN server (optional)",
  relayOnly: "Always go through TURN (hides your IP from the other person)",
  networkHint:
    "Relays and trackers only see a topic that changes daily and ciphertext. They see your IP address, like any website.",
  save: "Save",
  language: "Language",
  update: "A new version is ready.",
  reload: "Update",
  close: "Close",
  newSessionStarted: "Encrypted session established (X25519 + ML-KEM-768).",
  joinedSuffix: "joined",
  leftSuffix: "left",
  mismatchClosed: "The emojis did not match: connection closed.",
};

const dicts = { fr, en } as const;

/** French names of the SAS emojis, in the core's order (crates/hush-core/src/sas.rs). */
const SAS_FR = [
  "Chien",
  "Chat",
  "Lion",
  "Cheval",
  "Licorne",
  "Cochon",
  "Éléphant",
  "Lapin",
  "Panda",
  "Coq",
  "Pingouin",
  "Tortue",
  "Poisson",
  "Pieuvre",
  "Papillon",
  "Fleur",
  "Arbre",
  "Cactus",
  "Champignon",
  "Globe",
  "Lune",
  "Nuage",
  "Feu",
  "Banane",
  "Pomme",
  "Fraise",
  "Maïs",
  "Pizza",
  "Gâteau",
  "Cœur",
  "Sourire",
  "Robot",
  "Chapeau",
  "Lunettes",
  "Clé à molette",
  "Père Noël",
  "Pouce",
  "Parapluie",
  "Sablier",
  "Réveil",
  "Cadeau",
  "Ampoule",
  "Livre",
  "Crayon",
  "Trombone",
  "Ciseaux",
  "Cadenas",
  "Clé",
  "Marteau",
  "Téléphone",
  "Drapeau",
  "Train",
  "Vélo",
  "Avion",
  "Fusée",
  "Trophée",
  "Ballon",
  "Guitare",
  "Trompette",
  "Cloche",
  "Ancre",
  "Casque",
  "Dossier",
  "Punaise",
];

/** The spoken name of a SAS emoji, in the interface language. */
export function sasName(index: number, english: string): string {
  return lang.value === "fr" ? (SAS_FR[index & 63] ?? english) : english;
}
export type Lang = keyof typeof dicts;

function initial(): Lang {
  try {
    const saved = localStorage.getItem("hush.lang");
    if (saved === "fr" || saved === "en") return saved;
  } catch {
    // no storage
  }
  return typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("fr")
    ? "fr"
    : "en";
}

export const lang = signal<Lang>(typeof window === "undefined" ? "en" : initial());

export function setLang(l: Lang): void {
  lang.value = l;
  document.documentElement.lang = l;
  try {
    localStorage.setItem("hush.lang", l);
  } catch {
    // no storage
  }
}

export function t(key: Key, vars: Record<string, string | number> = {}): string {
  const s: string = dicts[lang.value][key] ?? fr[key];
  return s.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? ""));
}

/** "59 min", "23 h", "6 j" — coarse, for expiry countdowns. */
export function duration(ms: number): string {
  const m = Math.max(0, Math.ceil(ms / 60_000));
  const day = lang.value === "fr" ? "j" : "d";
  if (m < 60) return `${m} min`;
  if (m < 48 * 60) return `${Math.round(m / 60)} h`;
  return `${Math.round(m / 1440)} ${day}`;
}

export function ttlLabel(seconds: number, short = false): string {
  if (!seconds) return short ? t("timerOffShort") : t("timerOff");
  const fr = lang.value === "fr";
  if (seconds < 60) return `${seconds} s`;
  if (seconds < 3600) return `${seconds / 60} min`;
  if (seconds < 86_400) return `${seconds / 3600} h`;
  if (seconds < 604_800) return `${seconds / 86_400} ${fr ? "j" : "d"}`;
  return fr ? `${seconds / 604_800} sem.` : `${seconds / 604_800} wk`;
}

export const TTL_SECONDS = [0, 30, 300, 3600, 86_400, 604_800];
export const LINK_MINUTES = [10, 60, 1440, 10_080];
