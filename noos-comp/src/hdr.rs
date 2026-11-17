//! Sortie HDR10 (PQ / BT.2020) sur les écrans compatibles.
//!
//! Smithay (commit épinglé) ne sait rien du HDR : il ne touche jamais aux propriétés
//! `HDR_OUTPUT_METADATA`, `Colorspace` et `max bpc` du connecteur. On les écrit donc nous-mêmes
//! par une validation atomique séparée (le noyau garde ces valeurs ensuite ; les validations de
//! Smithay les reprennent telles quelles) :
//!
//! - **métadonnées HDR10** (`HDR_OUTPUT_METADATA`, InfoFrame CTA-861.3 de type 1) : EOTF
//!   SMPTE ST 2084 (PQ), primaires et luminances de l'écran lues dans l'EDID ;
//! - **espace de couleur** `Colorspace` = `BT2020_RGB` ;
//! - **profondeur** `max bpc` = 10.
//!
//! Le bureau, lui, est entièrement SDR (sRGB). Sans conversion il paraîtrait délavé et
//! sombre. Quand le HDR est actif, la liste d'éléments de l'écran est donc dessinée dans une
//! texture hors écran (comme en SDR), puis une dernière passe (shader [`SHADER`]) convertit
//! chaque pixel : sRGB (gamma 2,2) → linéaire → primaires BT.2020 → luminance absolue (blanc SDR
//! = `sdr_nits` cd/m², réglable) → encodage PQ. Le framebuffer de l'écran est en 10 bits.
//!
//! Limites de cette première version : pas de protocole `wp_color_management_v1` (les applis et
//! jeux HDR natifs sont traités comme du SDR), pas de scanout direct quand le HDR est actif
//! (toute l'image passe par la conversion).

use smithay::{
    backend::{
        allocator::Fourcc,
        drm::DrmDevice,
        renderer::{
            Bind, Color32F, Offscreen,
            damage::OutputDamageTracker,
            element::{Element, Id, Kind, RenderElement, RenderElementStates, UnderlyingStorage},
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
            utils::{CommitCounter, DamageBag, DamageSet, DamageSnapshot, OpaqueRegions},
        },
    },
    output::Output,
    reexports::drm::control::{
        AtomicCommitFlags, Device as _, atomic::AtomicModeReq, connector, property,
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
};
use tracing::{info, warn};

/// Conversion SDR → HDR10. `mode` : 0 = PQ (sortie réelle), 1 = aller-retour PQ → SDR
/// (aperçu imbriqué : l'image doit être identique au rendu SDR), 2 = signal PQ brut.
const SHADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform float sdr_nits;
uniform float mode;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// Primaires BT.709 (sRGB) → BT.2020, et l'inverse (colonnes : mat3 est en ordre colonne).
const mat3 BT709_TO_BT2020 = mat3(
    0.627404, 0.069097, 0.016391,
    0.329283, 0.919540, 0.088013,
    0.043313, 0.011362, 0.895595);
const mat3 BT2020_TO_BT709 = mat3(
     1.660491, -0.124550, -0.018151,
    -0.587641,  1.132900, -0.100579,
    -0.072850, -0.008349,  1.118730);

const float M1 = 0.1593017578125;
const float M2 = 78.84375;
const float C1 = 0.8359375;
const float C2 = 18.8515625;
const float C3 = 18.6875;

// Luminance absolue (cd/m², 0 – 10 000) → signal PQ (SMPTE ST 2084).
vec3 pq_encode(vec3 nits) {
    vec3 y = pow(clamp(nits / 10000.0, 0.0, 1.0), vec3(M1));
    return pow((C1 + C2 * y) / (1.0 + C3 * y), vec3(M2));
}

vec3 pq_decode(vec3 e) {
    vec3 p = pow(clamp(e, 0.0, 1.0), vec3(1.0 / M2));
    return 10000.0 * pow(max(p - C1, 0.0) / (C2 - C3 * p), vec3(1.0 / M1));
}

void main() {
    vec4 color = texture2D(tex, v_coords);
    // Alpha prémultiplié : on revient aux composantes réelles (le fond est opaque).
    vec3 rgb = color.a > 0.0 ? color.rgb / color.a : vec3(0.0);
    // Gamma 2,2 (comme la plupart des écrans SDR) plutôt que la courbe sRGB par morceaux :
    // des noirs moins délavés.
    vec3 linear = pow(clamp(rgb, 0.0, 1.0), vec3(2.2));
    vec3 nits = max(BT709_TO_BT2020 * linear, 0.0) * sdr_nits;
    vec3 pq = pq_encode(nits);
    vec3 outc = pq;
    if (mode > 0.5 && mode < 1.5) {
        vec3 back = BT2020_TO_BT709 * (pq_decode(pq) / sdr_nits);
        outc = pow(clamp(back, 0.0, 1.0), vec3(1.0 / 2.2));
    }
    gl_FragColor = vec4(outc, 1.0) * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

// ─────────────────────────── Capacités de l'écran (EDID) ───────────────────────────

/// Ce que l'EDID dit du HDR de l'écran.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdidHdr {
    /// EOTF PQ (HDR10) annoncée.
    pub pq: bool,
    /// Luminance maximale souhaitée (cd/m², 0 = inconnue).
    pub max_nits: f32,
    /// Luminance moyenne maximale d'une image (cd/m², 0 = inconnue).
    pub max_fall_nits: f32,
    /// Luminance minimale (cd/m²).
    pub min_nits: f32,
    /// Primaires rouge, vert, bleu puis point blanc (CIE 1931 x, y). None = inconnues.
    pub primaries: Option<[[f32; 2]; 4]>,
}

impl EdidHdr {
    pub fn from_info(info: &libdisplay_info::info::Info) -> Self {
        let meta = info.hdr_static_metadata();
        let prim = info.default_color_primaries();
        let primaries = (prim.has_primaries && prim.has_default_white_point).then(|| {
            [
                [prim.primary[0].x, prim.primary[0].y],
                [prim.primary[1].x, prim.primary[1].y],
                [prim.primary[2].x, prim.primary[2].y],
                [prim.default_white.x, prim.default_white.y],
            ]
        });
        Self {
            pq: meta.pq,
            max_nits: meta.desired_content_max_luminance,
            max_fall_nits: meta.desired_content_max_frame_avg_luminance,
            min_nits: meta.desired_content_min_luminance,
            primaries,
        }
    }

    /// Luminance de crête retenue (EDID, sinon 400 cd/m², le minimum DisplayHDR).
    pub fn peak_nits(&self) -> f32 {
        if self.max_nits > 0.0 { self.max_nits } else { 400.0 }
    }
}

/// Primaires BT.2020 et point blanc D65 (repli quand l'EDID ne les donne pas).
const BT2020_PRIMARIES: [[f32; 2]; 4] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], [0.3127, 0.3290]];

/// `struct hdr_output_metadata` du noyau (uapi `drm_mode.h`), 32 octets : type (u32), puis
/// l'InfoFrame HDMI/CTA de type 1 (EOTF, type, 3 primaires, point blanc, luminances).
/// Sérialisé à la main pour maîtriser le remplissage (le noyau exige la taille exacte).
pub fn hdr10_metadata_blob(edid: &EdidHdr) -> [u8; 32] {
    const EOTF_SMPTE_ST2084: u8 = 2;
    const STATIC_METADATA_TYPE1: u8 = 0;
    let mut b = [0u8; 32];
    b[0..4].copy_from_slice(&(STATIC_METADATA_TYPE1 as u32).to_ne_bytes());
    b[4] = EOTF_SMPTE_ST2084;
    b[5] = STATIC_METADATA_TYPE1;
    // Chromaticités en unités de 0,00002.
    let chroma = |v: f32| ((v * 50_000.0).round().clamp(0.0, 50_000.0)) as u16;
    let prim = edid.primaries.unwrap_or(BT2020_PRIMARIES);
    let mut off = 6;
    for p in prim {
        b[off..off + 2].copy_from_slice(&chroma(p[0]).to_ne_bytes());
        b[off + 2..off + 4].copy_from_slice(&chroma(p[1]).to_ne_bytes());
        off += 4;
    }
    let peak = edid.peak_nits().round().clamp(1.0, 10_000.0) as u16;
    // Luminance minimale en unités de 0,0001 cd/m².
    let min = if edid.min_nits > 0.0 { (edid.min_nits * 10_000.0).round().clamp(1.0, 65_535.0) as u16 } else { 500 };
    let fall = if edid.max_fall_nits > 0.0 { edid.max_fall_nits.round().clamp(1.0, 10_000.0) as u16 } else { peak };
    for (i, v) in [peak, min, peak, fall.min(peak)].into_iter().enumerate() {
        b[22 + 2 * i..24 + 2 * i].copy_from_slice(&v.to_ne_bytes());
    }
    b
}

// ─────────────────────────── Propriétés DRM du connecteur ───────────────────────────

/// Propriétés HDR du connecteur, telles que le pilote les expose.
#[derive(Debug, Clone, Default)]
pub struct ConnectorProps {
    pub hdr_metadata: Option<property::Handle>,
    /// (propriété, valeur « Default », valeur « BT2020_RGB » si proposée)
    pub colorspace: Option<(property::Handle, u64, Option<u64>)>,
    /// (propriété, minimum, maximum)
    pub max_bpc: Option<(property::Handle, u64, u64)>,
}

impl ConnectorProps {
    pub fn read(drm: &DrmDevice, conn: connector::Handle) -> Self {
        let mut out = Self::default();
        let Ok(props) = drm.get_properties(conn) else { return out };
        for (handle, _) in props.into_iter() {
            let Ok(info) = drm.get_property(handle) else { continue };
            match info.name().to_str() {
                Ok("HDR_OUTPUT_METADATA") => out.hdr_metadata = Some(handle),
                Ok("Colorspace") => {
                    if let property::ValueType::Enum(values) = info.value_type() {
                        let (_, enums) = values.values();
                        let find = |n: &str| enums.iter().find(|e| e.name().to_str() == Ok(n)).map(|e| e.value());
                        out.colorspace = Some((handle, find("Default").unwrap_or(0), find("BT2020_RGB")));
                    }
                }
                Ok("max bpc") => {
                    if let property::ValueType::UnsignedRange(min, max) = info.value_type() {
                        out.max_bpc = Some((handle, min, max));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Raison pour laquelle le HDR est impossible sur cet écran (None = possible).
pub fn unavailable_reason(edid: Option<&EdidHdr>, props: &ConnectorProps) -> Option<String> {
    if !edid.is_some_and(|e| e.pq) {
        return Some("L'écran n'annonce pas le HDR10 (PQ) dans son EDID".into());
    }
    if props.hdr_metadata.is_none() {
        return Some("Le pilote graphique n'expose pas HDR_OUTPUT_METADATA sur ce connecteur".into());
    }
    if props.colorspace.and_then(|c| c.2).is_none() {
        return Some("Le pilote graphique ne propose pas l'espace de couleur BT.2020 sur ce connecteur".into());
    }
    None
}

/// Écrit (ou retire) les propriétés HDR du connecteur par une validation atomique bloquante.
/// `ALLOW_MODESET` : changer l'EOTF ou l'espace de couleur demande en général un nouveau
/// réglage du mode (écran noir une à deux secondes, comme sous Windows).
pub fn apply_connector(
    drm: &DrmDevice,
    conn: connector::Handle,
    props: &ConnectorProps,
    hdr: Option<&EdidHdr>,
) -> Result<(), String> {
    let mut req = AtomicModeReq::new();
    let mut blob = None;
    match hdr {
        Some(edid) => {
            let meta = props.hdr_metadata.ok_or("HDR_OUTPUT_METADATA absent")?;
            let (cs, _, bt2020) = props.colorspace.ok_or("Colorspace absent")?;
            let bt2020 = bt2020.ok_or("BT2020_RGB non proposé")?;
            let data = hdr10_metadata_blob(edid);
            let value = drm.create_property_blob(&data).map_err(|e| format!("création des métadonnées HDR : {e}"))?;
            if let property::Value::Blob(id) = value {
                blob = Some(id);
            }
            req.add_property(conn, meta, value);
            req.add_property(conn, cs, property::Value::Unknown(bt2020));
            if let Some((bpc, min, max)) = props.max_bpc {
                req.add_property(conn, bpc, property::Value::UnsignedRange(10.clamp(min, max)));
            }
        }
        None => {
            if let Some(meta) = props.hdr_metadata {
                req.add_property(conn, meta, property::Value::Unknown(0));
            }
            if let Some((cs, default, _)) = props.colorspace {
                req.add_property(conn, cs, property::Value::Unknown(default));
            }
        }
    }
    let result = drm.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req).map_err(|e| format!("validation DRM refusée : {e}"));
    // Le noyau garde sa propre référence sur le blob une fois la validation faite.
    if let Some(id) = blob {
        let _ = drm.destroy_property_blob(id);
    }
    result
}

// ─────────────────────────── Passe de conversion ───────────────────────────

/// Mode du shader pour l'aperçu imbriqué (`NOOS_HDR_PREVIEW=roundtrip|pq`).
pub fn preview_mode() -> Option<f32> {
    match std::env::var("NOOS_HDR_PREVIEW").ok()?.as_str() {
        "pq" | "raw" => Some(2.0),
        "" | "0" => None,
        _ => Some(1.0),
    }
}

/// Rendu hors écran du bureau SDR puis conversion en PQ, pour un écran.
pub struct HdrPass {
    texture: Option<GlesTexture>,
    size: Size<i32, Physical>,
    scale: f64,
    tracker: Option<OutputDamageTracker>,
    damage: DamageBag<i32, Physical>,
    id: Id,
    program: Option<GlesTexProgram>,
    sdr_nits: f32,
    mode: f32,
}

impl HdrPass {
    pub fn new(sdr_nits: u32, mode: f32) -> Self {
        Self {
            texture: None,
            size: Size::default(),
            scale: 1.0,
            tracker: None,
            damage: DamageBag::new(4),
            id: Id::new(),
            program: None,
            sdr_nits: sdr_nits as f32,
            mode,
        }
    }

    pub fn set_sdr_nits(&mut self, nits: u32) {
        if (self.sdr_nits - nits as f32).abs() > 0.1 {
            self.sdr_nits = nits as f32;
            // Toute l'image change : dommage complet.
            self.damage.add([Rectangle::from_size(self.size)]);
        }
    }

    /// Dessine `elements` (bureau SDR de `output`) dans la texture et renvoie l'élément unique
    /// à afficher, avec l'état de rendu des éléments (retours de présentation).
    pub fn render<E: RenderElement<GlesRenderer>>(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        elements: &[E],
        clear: impl Into<Color32F>,
    ) -> Result<(HdrElement, RenderElementStates), String> {
        let mode = output.current_mode().ok_or("écran sans mode")?;
        let size = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        if self.texture.is_none() || self.size != size || (self.scale - scale).abs() > f64::EPSILON {
            let buffer_size: Size<i32, Buffer> = (size.w, size.h).into();
            // Demi-flottants si possible (plus de précision), sinon 10 bits, sinon 8 bits.
            let texture = [Fourcc::Abgr16161616f, Fourcc::Abgr2101010, Fourcc::Abgr8888]
                .into_iter()
                .find_map(|f| Offscreen::<GlesTexture>::create_buffer(renderer, f, buffer_size).ok())
                .ok_or("texture hors écran impossible")?;
            self.texture = Some(texture);
            self.size = size;
            self.scale = scale;
            self.tracker = Some(OutputDamageTracker::new(size, scale, Transform::Normal));
            self.damage.add([Rectangle::from_size(size)]);
        }
        if self.program.is_none() {
            let program = renderer
                .compile_custom_texture_shader(
                    SHADER,
                    &[UniformName::new("sdr_nits", UniformType::_1f), UniformName::new("mode", UniformType::_1f)],
                )
                .map_err(|e| format!("shader HDR : {e:?}"))?;
            self.program = Some(program);
        }
        let texture = self.texture.as_mut().unwrap();
        let tracker = self.tracker.as_mut().unwrap();
        let states = {
            let mut fb = renderer.bind(texture).map_err(|e| format!("liaison de la texture : {e:?}"))?;
            // La texture garde l'image précédente : âge 1 (rien à redessiner hors dommages).
            let result = tracker
                .render_output(renderer, &mut fb, 1, elements, clear)
                .map_err(|e| format!("rendu hors écran : {e:?}"))?;
            if let Some(damage) = result.damage {
                if !damage.is_empty() {
                    self.damage.add(damage.iter().copied());
                }
            }
            result.states
        };
        let element = HdrElement {
            id: self.id.clone(),
            texture: texture.clone(),
            size,
            snapshot: self.damage.snapshot(),
            program: self.program.clone().unwrap(),
            uniforms: vec![Uniform::new("sdr_nits", self.sdr_nits), Uniform::new("mode", self.mode)],
        };
        Ok((element, states))
    }
}

/// L'image convertie, posée sur tout l'écran.
pub struct HdrElement {
    id: Id,
    texture: GlesTexture,
    size: Size<i32, Physical>,
    snapshot: DamageSnapshot<i32, Physical>,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl std::fmt::Debug for HdrElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HdrElement").field("size", &self.size).finish()
    }
}

impl Element for HdrElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.snapshot.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((self.size.w as f64, self.size.h as f64).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::new(Point::from((0, 0)), self.size)
    }

    fn damage_since(&self, _scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.snapshot
            .damage_since(commit)
            .unwrap_or_else(|| DamageSet::from_slice(&[Rectangle::from_size(self.size)]))
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::from_slice(&[Rectangle::from_size(self.size)])
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for HdrElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            1.0,
            Some(&self.program),
            &self.uniforms,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Journalise les propriétés HDR d'un connecteur (diagnostic sur matériel réel).
pub fn log_props(name: &str, edid: Option<&EdidHdr>, props: &ConnectorProps) {
    info!(
        name,
        edid = ?edid,
        hdr_output_metadata = props.hdr_metadata.is_some(),
        colorspace_bt2020 = props.colorspace.and_then(|c| c.2).is_some(),
        max_bpc = ?props.max_bpc.map(|(_, a, b)| (a, b)),
        "HDR : capacités du connecteur"
    );
}

pub fn warn_failed(name: &str, err: &str) {
    warn!(name, "HDR : {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadonnees_hdr10() {
        let edid = EdidHdr {
            pq: true,
            max_nits: 400.0,
            max_fall_nits: 0.0,
            min_nits: 0.05,
            primaries: None,
        };
        let b = hdr10_metadata_blob(&edid);
        let u16_at = |o: usize| u16::from_ne_bytes([b[o], b[o + 1]]);
        assert_eq!(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]), 0);
        assert_eq!(b[4], 2, "EOTF PQ");
        assert_eq!(b[5], 0);
        // Rouge BT.2020 (0,708 ; 0,292), point blanc D65.
        assert_eq!((u16_at(6), u16_at(8)), (35_400, 14_600));
        assert_eq!((u16_at(18), u16_at(20)), (15_635, 16_450));
        assert_eq!(u16_at(22), 400, "luminance max");
        assert_eq!(u16_at(24), 500, "luminance min (0,0001 cd/m²)");
        assert_eq!(u16_at(26), 400, "MaxCLL");
        assert_eq!(u16_at(28), 400, "MaxFALL");
        assert_eq!(&b[30..], &[0, 0]);
    }

    #[test]
    fn raisons() {
        let props = ConnectorProps::default();
        assert!(unavailable_reason(None, &props).unwrap().contains("EDID"));
        let edid = EdidHdr { pq: true, max_nits: 0.0, max_fall_nits: 0.0, min_nits: 0.0, primaries: None };
        assert!(unavailable_reason(Some(&edid), &props).unwrap().contains("HDR_OUTPUT_METADATA"));
        assert_eq!(edid.peak_nits(), 400.0);
    }

    /// Mêmes formules que le shader : le blanc SDR doit tomber sur `sdr_nits` et l'aller-retour
    /// PQ être exact.
    #[test]
    fn pq_aller_retour() {
        let (m1, m2, c1, c2, c3) = (0.1593017578125f64, 78.84375, 0.8359375, 18.8515625, 18.6875);
        let enc = |n: f64| {
            let y = (n / 10_000.0).powf(m1);
            ((c1 + c2 * y) / (1.0 + c3 * y)).powf(m2)
        };
        let dec = |e: f64| {
            let p = e.powf(1.0 / m2);
            10_000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
        };
        // 203 cd/m² ≈ 58 % du signal PQ (référence BT.2408), 10 000 cd/m² = 1.
        assert!((enc(203.0) - 0.58).abs() < 0.01);
        assert!((enc(10_000.0) - 1.0).abs() < 1e-9);
        for n in [0.1, 1.0, 80.0, 203.0, 1000.0] {
            assert!((dec(enc(n)) - n).abs() / n < 1e-6);
        }
    }
}
