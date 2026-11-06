//! Petites opérations de couleur (sRGB) pour dériver une palette à partir d'un accent.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb { r: 1.0, g: 1.0, b: 1.0 };
    pub const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };

    /// `#rgb`, `#rrggbb` (le `#` est facultatif).
    pub fn parse(s: &str) -> Option<Rgb> {
        let h = s.trim().trim_start_matches('#');
        let v = |x: &str| u8::from_str_radix(x, 16).ok().map(|n| n as f64 / 255.0);
        match h.len() {
            3 => {
                let c: Vec<String> = h.chars().map(|c| format!("{c}{c}")).collect();
                Some(Rgb { r: v(&c[0])?, g: v(&c[1])?, b: v(&c[2])? })
            }
            6 | 8 => Some(Rgb { r: v(&h[0..2])?, g: v(&h[2..4])?, b: v(&h[4..6])? }),
            _ => None,
        }
    }

    pub fn hex(&self) -> String {
        let c = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
    }

    pub fn mix(&self, other: Rgb, t: f64) -> Rgb {
        Rgb {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
        }
    }

    /// Luminance relative WCAG.
    pub fn luminance(&self) -> f64 {
        let lin = |c: f64| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn contrast(&self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Texte lisible (blanc ou noir) sur cette couleur.
    pub fn readable_fg(&self) -> Rgb {
        if self.contrast(Rgb::WHITE) >= self.contrast(Rgb::BLACK) * 0.75 { Rgb::WHITE } else { Rgb::BLACK }
    }

    /// Variante « standalone » d'un accent (texte/icônes sur le fond de fenêtre) :
    /// éclaircie sur fond sombre, assombrie sur fond clair, jusqu'à un contraste de 4.5:1.
    pub fn standalone_on(&self, bg: Rgb) -> Rgb {
        let target = if bg.luminance() < 0.2 { Rgb::WHITE } else { Rgb::BLACK };
        let mut c = *self;
        let mut t = 0.0;
        while c.contrast(bg) < 4.5 && t < 0.9 {
            t += 0.05;
            c = self.mix(target, t);
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_hex() {
        assert_eq!(Rgb::parse("#89b4fa").unwrap().hex(), "#89b4fa");
        assert_eq!(Rgb::parse("fff").unwrap().hex(), "#ffffff");
        assert!(Rgb::parse("nope").is_none());
    }

    #[test]
    fn standalone_has_contrast() {
        let bg = Rgb::parse("#1e1e2e").unwrap();
        let accent = Rgb::parse("#1a5fb4").unwrap();
        assert!(accent.standalone_on(bg).contrast(bg) >= 4.5);
    }
}
