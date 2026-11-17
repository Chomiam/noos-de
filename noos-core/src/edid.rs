//! Lecture minimale de l'EDID d'un écran : ce qu'il annonce pour la fréquence variable (VRR).
//!
//! Le noyau ne dit au compositeur que « capable / pas capable » (propriété `vrr_capable`), ce
//! qui mélange deux causes : une carte graphique sans Adaptive-Sync et un écran qui ne l'annonce
//! pas. L'EDID permet de savoir ce que l'écran, lui, prétend gérer, pour l'expliquer dans les
//! Paramètres.

use serde::{Deserialize, Serialize};

/// Plage de fréquence variable annoncée par un écran.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct VrrRange {
    pub min_hz: u32,
    pub max_hz: u32,
    /// Bloc fournisseur AMD (FreeSync) présent dans l'extension CTA : l'écran se déclare
    /// explicitement compatible. Sinon, la plage vient du descripteur « limites de fréquence »,
    /// signe probable (mais pas certain) de l'Adaptive-Sync.
    pub freesync: bool,
}

/// Plage VRR annoncée par l'EDID, si l'écran en annonce une.
pub fn vrr_range(edid: &[u8]) -> Option<VrrRange> {
    if edid.len() < 128 || edid[..8] != [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00] {
        return None;
    }
    amd_vsdb(edid).or_else(|| range_limits(edid))
}

/// Bloc fournisseur AMD (OUI 00-00-1A) des extensions CTA-861 : version, capacités (bit 0 =
/// FreeSync), fréquence minimale, fréquence maximale.
fn amd_vsdb(edid: &[u8]) -> Option<VrrRange> {
    for ext in edid[128..].chunks_exact(128) {
        if ext[0] != 0x02 {
            continue;
        }
        let end = (ext[2] as usize).clamp(4, 127);
        let mut i = 4;
        while i < end {
            let tag = ext[i] >> 5;
            let len = (ext[i] & 0x1f) as usize;
            let block = ext.get(i + 1..i + 1 + len)?;
            if tag == 3 && len >= 7 && block[..3] == [0x1a, 0x00, 0x00] {
                let (caps, min, max) = (block[4], block[5] as u32, block[6] as u32);
                if caps & 1 != 0 && min > 0 && max > min {
                    return Some(VrrRange { min_hz: min, max_hz: max, freesync: true });
                }
            }
            i += 1 + len;
        }
    }
    None
}

/// Descripteur « Display Range Limits » (0xFD) du bloc de base, comme `drm_get_monitor_range`
/// du noyau : EDID 1.4 avec fréquences continues, et une vraie plage (plus de 10 Hz).
fn range_limits(edid: &[u8]) -> Option<VrrRange> {
    let (version, revision, features) = (edid[18], edid[19], edid[24]);
    if version != 1 || revision < 4 || features & 1 == 0 {
        return None;
    }
    [54, 72, 90, 108].into_iter().find_map(|o| {
        let d = &edid[o..o + 18];
        if d[..4] != [0, 0, 0, 0xfd] {
            return None;
        }
        let flags = d[4];
        let min = d[5] as u32 + if flags & 0x03 == 0x03 { 255 } else { 0 };
        let max = d[6] as u32 + if flags & 0x02 != 0 { 255 } else { 0 };
        (min > 0 && max > min + 10).then_some(VrrRange { min_hz: min, max_hz: max, freesync: false })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// EDID réel d'un AOC Q27G4 (2560×1440, 48–180 Hz, FreeSync), numéro de série remplacé.
    const AOC_Q27G4: &str = "00ffffffffffff0005e303b4000000001f220104b53c22783f1815aa5447ac270b5054bfef00d1c081803168317c4568457c6168617c6a5e00a0a0a029503020350055502100001e000000ff004e4f4f532d544553540a202020000000fc0051323747340a20202020202020000000fd0c30b419194a010a20202020202002ec020332f14c0103051404131f120211903f2309070783010000e305e301e60607016262006d1a0000020130b400000000000065e7006aa0a067500820980455502100001a70c200a0a0a055503020350055502100001ef03c00d051a0355060883a0055502100001c0000000000000000000000000000000000000000000000ae701279030003013c04060104ff09690007801f009f052700020004008d180184ff0987002f801f009f05270002000400eb010104ff09690007801f009f053c000200040000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f590";

    #[test]
    fn freesync_aoc() {
        let r = vrr_range(&hex(AOC_Q27G4)).unwrap();
        assert_eq!(r, VrrRange { min_hz: 48, max_hz: 180, freesync: true });
    }

    #[test]
    fn descripteur_seul() {
        // Sans l'extension CTA : plage du descripteur 0xFD (48–180 Hz).
        let base = &hex(AOC_Q27G4)[..128];
        let r = vrr_range(base).unwrap();
        assert_eq!((r.min_hz, r.max_hz, r.freesync), (48, 180, false));
    }

    #[test]
    fn plage_fixe_ou_edid_invalide() {
        let mut base = hex(AOC_Q27G4)[..128].to_vec();
        // Écran classique 56–61 Hz : pas de VRR.
        let fd = 108;
        assert_eq!(base[fd + 3], 0xfd);
        base[fd + 4] = 0;
        base[fd + 5] = 56;
        base[fd + 6] = 61;
        assert_eq!(vrr_range(&base), None);
        // Fréquences non continues : pas de VRR.
        base[fd + 6] = 144;
        base[24] &= !1;
        assert_eq!(vrr_range(&base), None);
        assert_eq!(vrr_range(&[0u8; 64]), None);
    }
}
