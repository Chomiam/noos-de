//! Textes expliquant la disponibilité du VRR (fréquence variable) d'un écran : carte graphique
//! incapable, écran qui ne l'annonce pas, ou tout va bien. Le compositeur fournit la cause
//! (`vrr_reason`), la plage annoncée par l'EDID (`vrr_display`) et le pilote (`gpu_driver`).

use noos_core::{
    edid::VrrRange,
    ipc::{OutputInfo, VrrReason},
};

pub struct VrrText {
    /// Sous-titre court de la ligne « Fréquence d'actualisation variable ».
    pub short: String,
    /// Explication détaillée quand le VRR est indisponible.
    pub detail: Option<String>,
}

fn range(r: &VrrRange) -> String {
    format!("{}–{} Hz", r.min_hz, r.max_hz)
}

/// Ce que le pilote de la carte graphique empêche, en clair.
fn gpu_limit(driver: &str) -> String {
    match driver {
        "i915" | "xe" => "Les puces graphiques Intel n'ont le circuit de fréquence variable qu'à partir de la \
             11e génération (Ice Lake, Iris Xe, Arc) : les HD et UHD Graphics plus anciennes \
             (UHD 620, UHD 630…) ne peuvent pas l'activer, quel que soit le réglage ou le paquet \
             installé. Une carte graphique AMD ou NVIDIA récente le permettrait."
            .into(),
        "amdgpu" => "Cette carte Radeon ne propose pas FreeSync sur cette sortie : il faut une Radeon \
             GCN 2 ou plus récente (R7 260, R9 290, RX…) et une prise DisplayPort ou HDMI \
             compatible."
            .into(),
        "radeon" => "L'ancien pilote « radeon » ne gère pas FreeSync : seul « amdgpu » le permet, sur \
             les Radeon GCN 2 et plus récentes."
            .into(),
        "nouveau" => "Le pilote libre « nouveau » ne gère pas la fréquence variable : installez le pilote \
             NVIDIA propriétaire (paquet nvidia-driver)."
            .into(),
        "nvidia-drm" | "nvidia" => "Le pilote NVIDIA n'expose pas l'Adaptive-Sync sur cette sortie : il faut une GeForce \
             GTX 10 ou plus récente, un pilote à jour et une prise DisplayPort (ou HDMI 2.1)."
            .into(),
        "" => "Le pilote de la carte graphique n'expose pas la fréquence variable sur cette sortie.".into(),
        d => format!("Le pilote « {d} » de la carte graphique n'expose pas la fréquence variable sur cette sortie."),
    }
}

/// `NOOS_VRR_DEMO=gpu:i915` ou `display` : simule une cause (captures, compositeur imbriqué),
/// avec un écran FreeSync 48–180 Hz.
fn demo(o: &OutputInfo) -> Option<OutputInfo> {
    let v = std::env::var("NOOS_VRR_DEMO").ok()?;
    let (reason, driver) = v.split_once(':').unwrap_or((&v, ""));
    let mut o = o.clone();
    o.vrr_reason = match reason {
        "gpu" => VrrReason::Gpu,
        "display" => VrrReason::Display,
        "supported" => VrrReason::Supported,
        _ => return None,
    };
    o.vrr_capable = o.vrr_reason == VrrReason::Supported;
    o.gpu_driver = driver.into();
    o.vrr_display = Some(VrrRange { min_hz: 48, max_hz: 180, freesync: true });
    Some(o)
}

pub fn explain(o: &OutputInfo) -> VrrText {
    let demo = demo(o);
    let o = demo.as_ref().unwrap_or(o);
    let r = o.vrr_display.as_ref();
    match o.vrr_reason {
        VrrReason::Supported => {
            let base = if o.vrr_active { "VRR actif en ce moment" } else { "FreeSync / Adaptive-Sync compatible" };
            VrrText { short: r.map(|r| format!("{base} ({})", range(r))).unwrap_or(base.into()), detail: None }
        }
        VrrReason::Gpu => {
            let screen = match r {
                Some(r) if r.freesync => {
                    format!("Votre écran gère FreeSync / Adaptive-Sync ({}), mais pas la carte graphique. ", range(r))
                }
                Some(r) => format!(
                    "Votre écran annonce une plage de fréquences de {} compatible avec le VRR, mais la carte \
                     graphique ne le propose pas. ",
                    range(r)
                ),
                None => String::new(),
            };
            VrrText {
                short: "Non pris en charge par la carte graphique".into(),
                detail: Some(format!("{screen}{}", gpu_limit(&o.gpu_driver))),
            }
        }
        VrrReason::Display => {
            let detail = match r {
                Some(r) if r.freesync => format!(
                    "L'écran se déclare compatible FreeSync ({}), mais ne l'a pas confirmé à la carte \
                     graphique. Activez « FreeSync » ou « Adaptive-Sync » dans le menu de l'écran (boutons \
                     de l'écran), utilisez un câble DisplayPort sans adaptateur, puis débranchez et \
                     rebranchez l'écran.",
                    range(r)
                ),
                _ => "La carte graphique gère la fréquence variable, mais cet écran ne l'annonce pas. S'il \
                      est compatible FreeSync, G-Sync Compatible ou Adaptive-Sync, activez l'option dans le \
                      menu de l'écran, utilisez un câble DisplayPort, puis rebranchez-le."
                    .into(),
            };
            VrrText { short: "Non annoncé par l'écran".into(), detail: Some(detail) }
        }
        VrrReason::Nested => {
            VrrText { short: "Indisponible dans la fenêtre de développement".into(), detail: None }
        }
        VrrReason::Unknown if o.vrr_capable => VrrText {
            short: if o.vrr_active { "VRR actif en ce moment" } else { "FreeSync / G-Sync compatible" }.into(),
            detail: None,
        },
        VrrReason::Unknown => VrrText { short: "Non pris en charge par cet écran ou ce GPU".into(), detail: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(reason: VrrReason, driver: &str, display: Option<VrrRange>) -> OutputInfo {
        OutputInfo {
            vrr_capable: reason == VrrReason::Supported,
            vrr_reason: reason,
            gpu_driver: driver.into(),
            vrr_display: display,
            ..Default::default()
        }
    }

    const FREESYNC: Option<VrrRange> = Some(VrrRange { min_hz: 48, max_hz: 180, freesync: true });

    #[test]
    fn intel_ancien_avec_ecran_freesync() {
        let t = explain(&out(VrrReason::Gpu, "i915", FREESYNC));
        assert_eq!(t.short, "Non pris en charge par la carte graphique");
        let d = t.detail.unwrap();
        assert!(d.starts_with("Votre écran gère FreeSync / Adaptive-Sync (48–180 Hz)"), "{d}");
        assert!(d.contains("11e génération"), "{d}");
    }

    #[test]
    fn ecran_a_activer_dans_son_menu() {
        let t = explain(&out(VrrReason::Display, "amdgpu", FREESYNC));
        assert_eq!(t.short, "Non annoncé par l'écran");
        assert!(t.detail.unwrap().contains("menu de l'écran"));
    }

    #[test]
    fn pris_en_charge_et_ancien_compositeur() {
        let t = explain(&out(VrrReason::Supported, "amdgpu", FREESYNC));
        assert_eq!(t.short, "FreeSync / Adaptive-Sync compatible (48–180 Hz)");
        assert!(t.detail.is_none());
        let t = explain(&out(VrrReason::Unknown, "", None));
        assert_eq!(t.short, "Non pris en charge par cet écran ou ce GPU");
    }
}
