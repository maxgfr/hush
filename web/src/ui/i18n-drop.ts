/**
 * Strings of the drop screens, loaded with them (the first screen stays under its budget).
 * Same rules as `i18n.ts`.
 */
import { t as base, lang } from "./i18n";

const fr = {
  pickFiles: "Choisir des fichiers",
  pickFolder: "Choisir un dossier",
  dropHere: "Déposez ici des fichiers ou des dossiers",
  dropToAdd: "Déposez pour ajouter",
  filesSummary: "{n} fichier(s) · {size}",
  remove: "Retirer",
  dropFor: "Pour",
  dropForLink: "Qui a le lien",
  dropForPeople: "Des contacts",
  dropForMe: "Moi (mes appareils)",
  dropPeopleHint:
    "Scellé à leur clé et signé par vous : le lien ne contient aucune clé, seuls eux peuvent l'ouvrir.",
  dropNeedsContacts: "Pour sceller à quelqu'un, déverrouillez vos contacts.",
  dropNoContacts: "Aucun contact enregistré pour l'instant.",
  dropOnce: "Supprimer après le premier téléchargement",
  dropP2pOnly: "Pair à pair uniquement, rien n'est stocké",
  dropP2pOnlyHint: "Le lien ne marche que tant que cette page reste ouverte.",
  dropAutoHint:
    "Si vous êtes tous deux en ligne, les fichiers passent directement. Sinon, une copie chiffrée attend sur des serveurs publics jusqu'à l'expiration. Ces serveurs ne voient que du chiffré.",
  dropNoServers: "Aucun serveur de fichiers : les fichiers ne passeront qu'en direct (Paramètres).",
  sealAndSend: "Chiffrer et créer le lien",
  sealing: "Chiffrement… {p} %",
  uploadingCopy: "Envoi de la copie chiffrée… {p} %",
  copyReady: "La copie chiffrée est en ligne : vous pouvez fermer cette page.",
  copyOn: "Stockée sur {servers}",
  p2pWaiting: "Si quelqu'un ouvre le lien maintenant, les fichiers passent directement.",
  p2pOnlyWaiting:
    "Gardez cette page ouverte : les fichiers passent directement quand le lien est ouvert.",
  p2pSending: "Lien ouvert : envoi direct en cours",
  p2pDelivered: "Livré directement (SHA-256 vérifié de l'autre côté)",
  dropFailedSend: "L'envoi a échoué : {e}",
  deleteCopy: "Supprimer des serveurs",
  copyDeleted: "Supprimé des serveurs",
  cancelSend: "Annuler l'envoi",
  sendMore: "Envoyer d'autres fichiers",
  sealedLinkHint:
    "Ce lien ne contient aucune clé : seules les personnes choisies peuvent l'ouvrir, avec leur identité. Il peut circuler n'importe où.",
  dropOnceShort: "supprimé après le premier téléchargement",
  dropManyShort: "téléchargeable plusieurs fois",
  filesForYou: "Des fichiers pour vous",
  dropFrom: "De {id} (signature vérifiée)",
  opening: "Ouverture…",
  availableFor: "Disponible encore {t}",
  downloadAll: "Tout télécharger",
  dropSenderOnline: "L'expéditeur est en ligne : transfert direct",
  dropFromCopy: "Depuis la copie chiffrée",
  dropWaitingSender: "En attente de l'expéditeur… Gardez cette page ouverte.",
  saving: "Téléchargement… {p} %",
  savedVerified: "Enregistré, chaque octet vérifié",
  dropDeletedNote: "Lien à usage unique : la copie a été supprimée des serveurs.",
  wrongPassphrase: "Mauvais mot de passe.",
  dropExpired: "Ce dépôt a expiré.",
  dropGone:
    "Ce dépôt n'est plus sur ses serveurs : il a déjà été téléchargé (lien à usage unique), l'expéditeur l'a supprimé, ou il a expiré.",
  dropGoneWait:
    "Si l'expéditeur a encore sa page ouverte, les fichiers arriveront directement ici.",
  senderGone: "L'expéditeur n'est plus en ligne et ce lien n'a pas de copie.",
  downloadFailed: "Le téléchargement a échoué. Réessayez.",
  notForYou: "Ce dépôt n'est pas scellé pour vous.",
  needIdentity: "Ce dépôt est scellé à une identité : déverrouillez vos contacts pour l'ouvrir.",
  folder: "Dossier",
} as const;

export type DropKey = keyof typeof fr;

const en: Record<DropKey, string> = {
  pickFiles: "Choose files",
  pickFolder: "Choose a folder",
  dropHere: "Drop files or folders here",
  dropToAdd: "Drop to add",
  filesSummary: "{n} file(s) · {size}",
  remove: "Remove",
  dropFor: "For",
  dropForLink: "Whoever has the link",
  dropForPeople: "Contacts",
  dropForMe: "Me (my devices)",
  dropPeopleHint:
    "Sealed to their key and signed by you: the link holds no key, only they can open it.",
  dropNeedsContacts: "To seal to someone, unlock your contacts.",
  dropNoContacts: "No saved contacts yet.",
  dropOnce: "Delete after the first download",
  dropP2pOnly: "Peer to peer only, nothing is stored",
  dropP2pOnlyHint: "The link works only while this page stays open.",
  dropAutoHint:
    "If you are both online, the files go directly. Otherwise a sealed copy waits on public servers until it expires. Those servers only see ciphertext.",
  dropNoServers: "No file server: files will only go directly (Settings).",
  sealAndSend: "Encrypt and create the link",
  sealing: "Encrypting… {p}%",
  uploadingCopy: "Uploading the sealed copy… {p}%",
  copyReady: "The sealed copy is online: you can close this page.",
  copyOn: "Stored on {servers}",
  p2pWaiting: "If someone opens the link now, the files go directly.",
  p2pOnlyWaiting: "Keep this page open: the files go directly when the link is opened.",
  p2pSending: "Link opened: sending directly",
  p2pDelivered: "Delivered directly (SHA-256 checked on their side)",
  dropFailedSend: "Sending failed: {e}",
  deleteCopy: "Delete from the servers",
  copyDeleted: "Deleted from the servers",
  cancelSend: "Cancel",
  sendMore: "Send more files",
  sealedLinkHint:
    "This link holds no key: only the people you chose can open it, with their identity. It can travel anywhere.",
  dropOnceShort: "deleted after the first download",
  dropManyShort: "can be downloaded several times",
  filesForYou: "Files for you",
  dropFrom: "From {id} (signature verified)",
  opening: "Opening…",
  availableFor: "Available for {t}",
  downloadAll: "Download all",
  dropSenderOnline: "The sender is online: direct transfer",
  dropFromCopy: "From the sealed copy",
  dropWaitingSender: "Waiting for the sender… Keep this page open.",
  saving: "Downloading… {p}%",
  savedVerified: "Saved, every byte verified",
  dropDeletedNote: "One-time link: the copy was deleted from the servers.",
  wrongPassphrase: "Wrong passphrase.",
  dropExpired: "This drop has expired.",
  dropGone:
    "This drop is no longer on its servers: it was already downloaded (one-time link), the sender deleted it, or it expired.",
  dropGoneWait: "If the sender still has their page open, the files will come straight here.",
  senderGone: "The sender is no longer online and this link has no copy.",
  downloadFailed: "The download failed. Try again.",
  notForYou: "This drop is not sealed to you.",
  needIdentity: "This drop is sealed to an identity: unlock your contacts to open it.",
  folder: "Folder",
};

const dicts = { fr, en } as const;

/** A drop string, or any string of the app. */
export function t(
  key: DropKey | Parameters<typeof base>[0],
  vars: Record<string, string | number> = {},
): string {
  if (!(key in fr)) return base(key as Parameters<typeof base>[0], vars);
  const s: string = dicts[lang.value][key as DropKey] ?? fr[key as DropKey];
  return s.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? ""));
}
