//! SSRF- & DNS-rebinding-hardened `SafeExternalFetch` policy (`§20`, `GATE-09`).

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use thiserror::Error;

pub const MAX_REDIRECT_HOPS: u8 = 5;
pub const MAX_WIRE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SafeFetchError {
    #[error("non-HTTPS scheme rejected: {0}")]
    InsecureScheme(String),
    #[error("redirect hop limit ({MAX_REDIRECT_HOPS}) exceeded")]
    TooManyRedirects,
    #[error("SSRF blocked prohibited IP {0} ({1})")]
    BlockedIp(IpAddr, &'static str),
    #[error("DNS rebinding detected: pinned {pinned} != connect target {actual}")]
    DnsRebindingDetected {
        pinned: SocketAddr,
        actual: SocketAddr,
    },
    #[error("response exceeded byte cap ({0} bytes)")]
    SizeCapExceeded(u64),
    #[error("I/O error while reading response: {0}")]
    Io(String),
    #[error("XML external entity / DOCTYPE declaration rejected")]
    XmlEntityRejected,
}

/// Validates URL scheme (HTTPS-only), authority hygiene (no empty host, userinfo `@`, or control
/// chars), and redirect budget (`<= 5` hops).
pub fn validate_https_url(url: &str, redirect_hop: u8) -> Result<(), SafeFetchError> {
    if redirect_hop > MAX_REDIRECT_HOPS {
        return Err(SafeFetchError::TooManyRedirects);
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(SafeFetchError::InsecureScheme(url.to_owned()));
    };
    if rest.is_empty()
        || url
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return Err(SafeFetchError::InsecureScheme(url.to_owned()));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(SafeFetchError::InsecureScheme(url.to_owned()));
    }
    Ok(())
}

/// Canonicalizes IPv4-mapped IPv6 (`::ffff:127.0.0.1` -> `127.0.0.1`) and rejects
/// loopback, RFC1918, CGNAT `100.64.0.0/10`, IMDS `169.254.0.0/16`, `0.0.0.0/8`,
/// reserved/benchmarking ranges, NAT64 `64:ff9b::/96`, IPv4-compatible `::/96`, and link-local ranges.
pub fn validate_ip_safe(ip: IpAddr) -> Result<IpAddr, SafeFetchError> {
    let canonical = ip.to_canonical();
    match canonical {
        IpAddr::V4(v4) => validate_v4(v4)?,
        IpAddr::V6(v6) => validate_v6(v6)?,
    }
    Ok(canonical)
}

fn validate_v4(v4: Ipv4Addr) -> Result<(), SafeFetchError> {
    let oct = v4.octets();
    if v4.is_loopback() {
        return Err(SafeFetchError::BlockedIp(IpAddr::V4(v4), "loopback"));
    }
    if v4.is_private() {
        return Err(SafeFetchError::BlockedIp(IpAddr::V4(v4), "RFC1918_private"));
    }
    if v4.is_link_local() {
        return Err(SafeFetchError::BlockedIp(
            IpAddr::V4(v4),
            "IMDS_or_link_local",
        ));
    }
    if oct[0] == 0 {
        return Err(SafeFetchError::BlockedIp(
            IpAddr::V4(v4),
            "this_network_0_8",
        ));
    }
    if oct[0] == 100 && (oct[1] & 0b1100_0000) == 64 {
        return Err(SafeFetchError::BlockedIp(IpAddr::V4(v4), "CGNAT_100_64_10"));
    }
    if (oct[0] == 192 && oct[1] == 0 && oct[2] == 0)
        || (oct[0] == 198 && (oct[1] == 18 || oct[1] == 19))
        || oct[0] >= 240
    {
        return Err(SafeFetchError::BlockedIp(IpAddr::V4(v4), "reserved_v4"));
    }
    if v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() {
        return Err(SafeFetchError::BlockedIp(IpAddr::V4(v4), "non_global_v4"));
    }
    Ok(())
}

fn validate_v6(v6: Ipv6Addr) -> Result<(), SafeFetchError> {
    if v6.is_loopback() {
        return Err(SafeFetchError::BlockedIp(IpAddr::V6(v6), "ipv6_loopback"));
    }
    let segs = v6.segments();
    let seg0 = segs[0];
    if (seg0 & 0xfe00) == 0xfc00 {
        return Err(SafeFetchError::BlockedIp(
            IpAddr::V6(v6),
            "ipv6_unique_local",
        ));
    }
    if (seg0 & 0xffc0) == 0xfe80 {
        return Err(SafeFetchError::BlockedIp(IpAddr::V6(v6), "ipv6_link_local"));
    }
    // Deprecated IPv4-compatible (::/96) and NAT64 well-known prefix (64:ff9b::/96)
    if segs[0..6] == [0, 0, 0, 0, 0, 0] {
        return Err(SafeFetchError::BlockedIp(
            IpAddr::V6(v6),
            "ipv4_compatible_v6",
        ));
    }
    if segs[0] == 0x0064 && segs[1] == 0xff9b && segs[2..6] == [0, 0, 0, 0] {
        let oct = v6.octets();
        let embedded = Ipv4Addr::new(oct[12], oct[13], oct[14], oct[15]);
        validate_v4(embedded)
            .map_err(|_| SafeFetchError::BlockedIp(IpAddr::V6(v6), "nat64_private_v4"))?;
    }
    if segs[0] == 0x2001 && segs[1] == 0x0db8 {
        return Err(SafeFetchError::BlockedIp(
            IpAddr::V6(v6),
            "ipv6_documentation",
        ));
    }
    if v6.is_unspecified() || v6.is_multicast() {
        return Err(SafeFetchError::BlockedIp(IpAddr::V6(v6), "non_global_v6"));
    }
    Ok(())
}

/// Connect-time `SocketAddr` pinning verification against `TTL=0` DNS rebinding.
pub fn verify_pinned_connect(pinned: SocketAddr, actual: SocketAddr) -> Result<(), SafeFetchError> {
    validate_ip_safe(pinned.ip())?;
    if pinned != actual {
        return Err(SafeFetchError::DnsRebindingDetected { pinned, actual });
    }
    Ok(())
}

/// Rejects XML payloads containing `<!DOCTYPE` or `<!ENTITY` (XXE protection, including UTF-16 NUL-interleaved smuggling).
pub fn reject_unsafe_xml(body: &[u8]) -> Result<(), SafeFetchError> {
    let stripped: Vec<u8> = body.iter().copied().filter(|&b| b != 0).collect();
    let text = String::from_utf8_lossy(&stripped).to_ascii_uppercase();
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(SafeFetchError::XmlEntityRejected);
    }
    Ok(())
}

/// Bounded `Read::take` reader enforcing hard byte caps (`25 MiB` wire / `64 MiB` uncompressed).
pub fn read_capped<R: Read>(reader: R, cap: u64) -> Result<Vec<u8>, SafeFetchError> {
    let mut buf = Vec::new();
    let mut limited = reader.take(cap.saturating_add(1));
    limited
        .read_to_end(&mut buf)
        .map_err(|e| SafeFetchError::Io(e.to_string()))?;
    if (buf.len() as u64) > cap {
        return Err(SafeFetchError::SizeCapExceeded(cap));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn blocks_ssrf_ipv6_mapped_imds_cgnat_rebinding_and_xxe() {
        assert!(validate_https_url("http://example.com", 0).is_err());
        assert!(validate_https_url("https://", 0).is_err());
        assert!(validate_https_url("https://user:pass@example.com/a", 0).is_err());
        assert!(validate_https_url("https://example.com\r\nX-Inject: 1", 0).is_err());
        assert!(validate_https_url("https://example.com", 6).is_err());
        assert!(validate_https_url("https://example.com", 2).is_ok());

        for bad in [
            "127.0.0.1",
            "0.1.2.3",
            "240.0.0.1",
            "::1",
            "::127.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::169.254.169.254",
            "169.254.169.254",
            "10.0.0.5",
            "100.64.0.1",
            "fe80::1",
        ] {
            let ip = IpAddr::from_str(bad).expect("parse ip");
            assert!(validate_ip_safe(ip).is_err(), "expected {bad} blocked");
        }
        assert!(validate_ip_safe(IpAddr::from_str("93.184.216.34").expect("ip")).is_ok());

        let pinned = SocketAddr::from_str("93.184.216.34:443").expect("addr");
        let rebind = SocketAddr::from_str("127.0.0.1:443").expect("addr");
        assert!(verify_pinned_connect(pinned, rebind).is_err());
        assert!(reject_unsafe_xml(b"<?xml version='1.0'?><!DOCTYPE foo><a/>").is_err());
        assert!(reject_unsafe_xml(b"<\0!\0D\0O\0C\0T\0Y\0P\0E\0 ").is_err());
        assert!(read_capped(&b"123456"[..], 4).is_err());
        assert_eq!(read_capped(&b"1234"[..], 4).expect("ok"), b"1234");
    }
}
