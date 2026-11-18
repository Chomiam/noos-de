//! Jours fériés en France (widget Calendrier) : dates fixes et fêtes mobiles calculées
//! depuis Pâques (algorithme de Meeus / Butcher).

/// Dimanche de Pâques (mois, jour) du calendrier grégorien.
pub fn easter(year: i32) -> (u32, u32) {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    (month as u32, day as u32)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// (mois, jour) + `n` jours, dans la même année.
fn add_days(year: i32, (mut m, mut d): (u32, u32), n: u32) -> (u32, u32) {
    d += n;
    while d > days_in_month(year, m) {
        d -= days_in_month(year, m);
        m += 1;
    }
    (m, d)
}

/// Jours fériés de l'année, dans l'ordre : (mois, jour, nom).
pub fn holidays(year: i32) -> Vec<(u32, u32, &'static str)> {
    let e = easter(year);
    let mobile = |n, name| {
        let (m, d) = add_days(year, e, n);
        (m, d, name)
    };
    let mut v = vec![
        (1, 1, "Jour de l'an"),
        mobile(1, "Lundi de Pâques"),
        (5, 1, "Fête du Travail"),
        (5, 8, "Victoire 1945"),
        mobile(39, "Ascension"),
        mobile(50, "Lundi de Pentecôte"),
        (7, 14, "Fête nationale"),
        (8, 15, "Assomption"),
        (11, 1, "Toussaint"),
        (11, 11, "Armistice 1918"),
        (12, 25, "Noël"),
    ];
    v.sort();
    v
}

/// Prochain jour férié à partir du (année, mois, jour) donné (inclus) : (année, mois, jour, nom).
pub fn next_holiday(year: i32, month: u32, day: u32) -> (i32, u32, u32, &'static str) {
    for y in [year, year + 1] {
        for (m, d, name) in holidays(y) {
            if y > year || (m, d) >= (month, day) {
                return (y, m, d, name);
            }
        }
    }
    (year + 1, 1, 1, "Jour de l'an")
}

/// Le jour est-il férié ?
pub fn holiday_name(year: i32, month: u32, day: u32) -> Option<&'static str> {
    holidays(year).into_iter().find(|(m, d, _)| *m == month && *d == day).map(|(_, _, n)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paques() {
        assert_eq!(easter(2024), (3, 31));
        assert_eq!(easter(2025), (4, 20));
        assert_eq!(easter(2026), (4, 5));
        assert_eq!(easter(2027), (3, 28));
    }

    #[test]
    fn feries() {
        let h = holidays(2026);
        assert_eq!(h.len(), 11);
        assert!(h.contains(&(4, 6, "Lundi de Pâques")));
        assert!(h.contains(&(5, 14, "Ascension")));
        assert!(h.contains(&(5, 25, "Lundi de Pentecôte")));
        assert_eq!(next_holiday(2026, 11, 18), (2026, 12, 25, "Noël"));
        assert_eq!(next_holiday(2026, 12, 26), (2027, 1, 1, "Jour de l'an"));
        assert_eq!(next_holiday(2026, 11, 11).3, "Armistice 1918");
        assert_eq!(holiday_name(2026, 7, 14), Some("Fête nationale"));
        assert_eq!(holiday_name(2026, 7, 15), None);
    }
}
