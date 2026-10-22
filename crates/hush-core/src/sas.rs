//! Short authentication string: 5 emojis (30 bits) both people compare, by voice or in person.
//!
//! Derived from the final handshake hash and both sides' nonces. Each nonce is committed to before
//! the other side's is known, so someone in the middle cannot search for colliding values: each
//! attempt succeeds with probability 2^-30, and a failed attempt is visible.

use crate::kdf::{expand_into, extract};

/// Number of emojis shown.
pub const SAS_LEN: usize = 5;

/// Matrix's SAS emoji table: 64 symbols chosen to be easy to tell apart and to say aloud.
pub const EMOJIS: [(&str, &str); 64] = [
    ("🐶", "Dog"),
    ("🐱", "Cat"),
    ("🦁", "Lion"),
    ("🐎", "Horse"),
    ("🦄", "Unicorn"),
    ("🐷", "Pig"),
    ("🐘", "Elephant"),
    ("🐰", "Rabbit"),
    ("🐼", "Panda"),
    ("🐓", "Rooster"),
    ("🐧", "Penguin"),
    ("🐢", "Turtle"),
    ("🐟", "Fish"),
    ("🐙", "Octopus"),
    ("🦋", "Butterfly"),
    ("🌷", "Flower"),
    ("🌳", "Tree"),
    ("🌵", "Cactus"),
    ("🍄", "Mushroom"),
    ("🌏", "Globe"),
    ("🌙", "Moon"),
    ("☁️", "Cloud"),
    ("🔥", "Fire"),
    ("🍌", "Banana"),
    ("🍎", "Apple"),
    ("🍓", "Strawberry"),
    ("🌽", "Corn"),
    ("🍕", "Pizza"),
    ("🎂", "Cake"),
    ("❤️", "Heart"),
    ("😀", "Smiley"),
    ("🤖", "Robot"),
    ("🎩", "Hat"),
    ("👓", "Glasses"),
    ("🔧", "Spanner"),
    ("🎅", "Santa"),
    ("👍", "Thumbs up"),
    ("☂️", "Umbrella"),
    ("⌛", "Hourglass"),
    ("⏰", "Clock"),
    ("🎁", "Gift"),
    ("💡", "Light bulb"),
    ("📕", "Book"),
    ("✏️", "Pencil"),
    ("📎", "Paperclip"),
    ("✂️", "Scissors"),
    ("🔒", "Lock"),
    ("🔑", "Key"),
    ("🔨", "Hammer"),
    ("☎️", "Telephone"),
    ("🏁", "Flag"),
    ("🚂", "Train"),
    ("🚲", "Bicycle"),
    ("✈️", "Aeroplane"),
    ("🚀", "Rocket"),
    ("🏆", "Trophy"),
    ("⚽", "Ball"),
    ("🎸", "Guitar"),
    ("🎺", "Trumpet"),
    ("🔔", "Bell"),
    ("⚓", "Anchor"),
    ("🎧", "Headphones"),
    ("📁", "Folder"),
    ("📌", "Pin"),
];

/// Commitment to a SAS nonce, sent before the nonce itself.
#[must_use]
pub fn commit(nonce: &[u8; 32]) -> [u8; 32] {
    crate::kdf::sha256(&[b"hush/v1/sas-commit", nonce])
}

/// The emoji indices (each in 0..64) for a finished handshake.
#[must_use]
pub fn derive(handshake_hash: &[u8; 32], nonce_creator: &[u8; 32], nonce_joiner: &[u8; 32]) -> [u8; SAS_LEN] {
    let prk = extract(b"hush/v1/sas", handshake_hash);
    let mut bits = [0u8; 4];
    expand_into(&prk, b"hush/v1/sas/emoji", &[nonce_creator, nonce_joiner], &mut bits);
    let v = u32::from_be_bytes(bits) >> 2; // 30 bits
    let mut out = [0u8; SAS_LEN];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::try_from((v >> (24 - 6 * i)) & 63).expect("6 bits");
    }
    out
}

/// Renders indices as emojis separated by spaces.
#[must_use]
pub fn render(indices: &[u8]) -> String {
    indices
        .iter()
        .map(|&i| EMOJIS[usize::from(i & 63)].0)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Renders indices as their English names, for screen readers and terminals without emoji.
#[must_use]
pub fn names(indices: &[u8]) -> String {
    indices
        .iter()
        .map(|&i| EMOJIS[usize::from(i & 63)].1)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depends_on_every_input() {
        let base = derive(&[1; 32], &[2; 32], &[3; 32]);
        assert!(base.iter().all(|&i| i < 64));
        assert_ne!(base, derive(&[9; 32], &[2; 32], &[3; 32]));
        assert_ne!(base, derive(&[1; 32], &[9; 32], &[3; 32]));
        assert_ne!(base, derive(&[1; 32], &[2; 32], &[9; 32]));
        assert_eq!(render(&base).split(' ').count(), SAS_LEN);
    }
}
