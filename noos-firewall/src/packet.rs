//! Analyse des en-têtes IPv4 / IPv6 + TCP / UDP d'un paquet reçu par NFQUEUE (les premiers
//! octets suffisent : la file copie 128 octets).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub src: IpAddr,
    pub dst: IpAddr,
    /// Numéro du protocole (6 = TCP, 17 = UDP, 1 = ICMP, 58 = ICMPv6…).
    pub proto: u8,
    /// Ports (0 si le protocole n'en a pas).
    pub sport: u16,
    pub dport: u16,
}

pub fn parse(data: &[u8]) -> Option<Packet> {
    let version = data.first()? >> 4;
    let (src, dst, mut proto, mut off) = match version {
        4 => {
            if data.len() < 20 {
                return None;
            }
            let ihl = (data[0] & 0x0f) as usize * 4;
            if ihl < 20 {
                return None;
            }
            // Fragment non initial : pas d'en-tête de transport.
            let frag = u16::from_be_bytes([data[6], data[7]]) & 0x1fff;
            let src = IpAddr::V4(Ipv4Addr::new(data[12], data[13], data[14], data[15]));
            let dst = IpAddr::V4(Ipv4Addr::new(data[16], data[17], data[18], data[19]));
            if frag != 0 {
                return Some(Packet { src, dst, proto: data[9], sport: 0, dport: 0 });
            }
            (src, dst, data[9], ihl)
        }
        6 => {
            if data.len() < 40 {
                return None;
            }
            let a: [u8; 16] = data[8..24].try_into().ok()?;
            let b: [u8; 16] = data[24..40].try_into().ok()?;
            (IpAddr::V6(Ipv6Addr::from(a)), IpAddr::V6(Ipv6Addr::from(b)), data[6], 40)
        }
        _ => return None,
    };
    // En-têtes d'extension IPv6 courants (saut par saut, routage, destination).
    if version == 6 {
        for _ in 0..4 {
            match proto {
                0 | 43 | 60 => {
                    let h = data.get(off..off + 2)?;
                    proto = h[0];
                    off += (h[1] as usize + 1) * 8;
                }
                44 => {
                    let h = data.get(off..off + 8)?;
                    proto = h[0];
                    let frag = u16::from_be_bytes([h[2], h[3]]) >> 3;
                    if frag != 0 {
                        return Some(Packet { src, dst, proto, sport: 0, dport: 0 });
                    }
                    off += 8;
                }
                _ => break,
            }
        }
    }
    let (sport, dport) = match proto {
        6 | 17 | 132 | 136 => {
            let p = data.get(off..off + 4)?;
            (u16::from_be_bytes([p[0], p[1]]), u16::from_be_bytes([p[2], p[3]]))
        }
        _ => (0, 0),
    };
    Some(Packet { src, dst, proto, sport, dport })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv4_tcp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> Vec<u8> {
        let mut p = vec![0x45, 0, 0, 40, 0, 0, 0x40, 0, 64, 6, 0, 0];
        p.extend(src);
        p.extend(dst);
        p.extend(sport.to_be_bytes());
        p.extend(dport.to_be_bytes());
        p.extend([0u8; 16]);
        p
    }

    #[test]
    fn v4_tcp() {
        let p = parse(&ipv4_tcp([192, 168, 1, 10], [140, 82, 112, 3], 51234, 443)).unwrap();
        assert_eq!(p.src.to_string(), "192.168.1.10");
        assert_eq!(p.dst.to_string(), "140.82.112.3");
        assert_eq!((p.proto, p.sport, p.dport), (6, 51234, 443));
    }

    #[test]
    fn v4_options_and_fragment() {
        // IHL = 6 (une option de 4 octets) puis UDP.
        let mut p = vec![0x46, 0, 0, 40, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2, 1, 1, 1, 1];
        p.extend(5353u16.to_be_bytes());
        p.extend(53u16.to_be_bytes());
        let r = parse(&p).unwrap();
        assert_eq!((r.proto, r.sport, r.dport), (17, 5353, 53));
        // Fragment non initial : pas de ports.
        let mut f = ipv4_tcp([1, 1, 1, 1], [2, 2, 2, 2], 1, 2);
        f[7] = 10;
        assert_eq!(parse(&f).unwrap().dport, 0);
    }

    #[test]
    fn v6_udp_with_hop_by_hop() {
        let mut p = vec![0x60, 0, 0, 0, 0, 16, 0, 64];
        p.extend("fe80::1".parse::<Ipv6Addr>().unwrap().octets());
        p.extend("ff02::fb".parse::<Ipv6Addr>().unwrap().octets());
        // Saut par saut (8 octets) → UDP
        p.extend([17, 0, 0, 0, 0, 0, 0, 0]);
        p.extend(5353u16.to_be_bytes());
        p.extend(5353u16.to_be_bytes());
        p.extend([0, 8, 0, 0]);
        let r = parse(&p).unwrap();
        assert_eq!(r.dst.to_string(), "ff02::fb");
        assert_eq!((r.proto, r.dport), (17, 5353));
    }

    #[test]
    fn garbage() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0x45, 0, 0]).is_none());
        assert!(parse(&[0x20; 40]).is_none());
    }
}
