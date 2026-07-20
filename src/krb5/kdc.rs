//! Kerberos KDC client: the AS and TGS exchanges, std-only.
//!
//! Drives both KDC round trips (RFC 4120 3.1, 3.3) over TCP, using the
//! 4-byte length-prefixed framing RFC 4120 7.2.2 specifies for that
//! transport: [`get_tgt`] trades a realm/username/password for a
//! Ticket-Granting Ticket (TGT), and [`tgs_exchange`] trades that TGT for a
//! service ticket. [`build_ap_req`] then assembles the AP-REQ from that
//! service ticket — no network needed — and [`fetch_ap_req`] chains all
//! three to go straight from a realm/username/password/service-principal to
//! the `(ap_req_bytes, session_key)` pair `crate::tls::connect_tls_kerberos`
//! takes.
//!
//! [`get_tgt`] always sends optimistic `PA-ENC-TIMESTAMP` pre-authentication
//! (RFC 4120 5.2.7.2), since virtually every deployed KDC requires it, using
//! the RFC 4120/3961 *default* salt (`REALM` concatenated with the principal
//! name components). A KDC configured with a non-default salt (returned via
//! `PA-ETYPE-INFO2` in a `KDC_ERR_PREAUTH_REQUIRED` error) is not handled —
//! this fails with that error instead of retrying. [`tgs_exchange`] only
//! handles the same-realm case — no cross-realm referral chasing.
//!
//! AES only (etypes 17/18, [`crate::krb5::aes`]); the RC4-HMAC profile
//! ([`crate::krb5::crypto`]) is not wired into these exchanges.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};

use super::aes::{AesKey, ETYPE_AES256_CTS_HMAC_SHA1_96};
use super::asn1::{self, EncryptedData, KerberosTime, PrincipalName, NT_PRINCIPAL, NT_SRV_INST};
use super::messages::{
    ApReq, Authenticator, EncKdcRepPart, KdcRep, KdcReq, KdcReqBody, KrbError, PaData, Ticket,
    KDC_OPT_CANONICALIZE, KDC_OPT_FORWARDABLE, KDC_OPT_RENEWABLE, KRB_AS_REP, KRB_AS_REQ,
    KRB_TGS_REP, KRB_TGS_REQ, PA_ENC_TIMESTAMP, PA_TGS_REQ,
};
use crate::cursor::Writer;
use crate::error::Error;

/// Key usage 1: `AS-REQ PA-ENC-TIMESTAMP` padata, encrypted with the client key.
const USAGE_AS_REQ_PA_ENC_TIMESTAMP: u32 = 1;
/// Key usage 3: `AS-REP` encrypted part, encrypted with the client key.
const USAGE_AS_REP_ENCPART: u32 = 3;
/// Key usage 7: `TGS-REQ PA-TGS-REQ AP-REQ` Authenticator, encrypted with
/// the TGT's session key (no Authenticator subkey is used here).
const USAGE_TGS_REQ_AUTHENTICATOR: u32 = 7;
/// Key usage 9: `TGS-REP` encrypted part, encrypted with the TGT's session
/// key (used when the Authenticator carried no subkey, as here).
const USAGE_TGS_REP_ENCPART: u32 = 9;
/// Key usage 11: `AP-REQ` Authenticator, encrypted with the application
/// session key.
const USAGE_AP_REQ_AUTHENTICATOR: u32 = 11;

/// RFC 4120 7.5.9: additional pre-authentication is required (the KDC wants
/// `PA-ENC-TIMESTAMP`, or a different salt/etype than what was offered).
pub const KDC_ERR_PREAUTH_REQUIRED: i32 = 25;

/// A far-future requested ticket expiry — real KDCs clamp this to their own
/// policy maximum regardless, so there is no benefit to computing a tighter
/// one from the current time.
fn distant_till() -> KerberosTime {
    KerberosTime::from_utc(2037, 1, 1, 0, 0, 0)
}

/// Ask a KDC for a Ticket-Granting Ticket over an already-connected `stream`,
/// authenticating with `PA-ENC-TIMESTAMP` derived from `key`.
///
/// `nonce`/`confounder`/`now` are the exchange's nondeterministic inputs,
/// supplied by the caller so the exchange is testable; [`fetch_tgt`] is the
/// convenience wrapper that supplies OS randomness and the current time.
///
/// Returns the TGT — a `krbtgt/REALM@REALM` [`Ticket`], opaque to the client
/// — and its session key.
pub fn get_tgt(
    stream: &mut TcpStream,
    realm: &str,
    user: &str,
    key: &AesKey,
    nonce: u32,
    confounder: [u8; 16],
    now: SystemTime,
) -> io::Result<(Ticket, AesKey)> {
    let padata = vec![PaData {
        padata_type: PA_ENC_TIMESTAMP,
        padata_value: EncryptedData {
            etype: key.etype(),
            kvno: None,
            cipher: key.encrypt(
                USAGE_AS_REQ_PA_ENC_TIMESTAMP,
                &pa_enc_ts_enc(now),
                &confounder,
            ),
        }
        .encode(),
    }];

    let req = KdcReq {
        msg_type: KRB_AS_REQ,
        padata,
        req_body: KdcReqBody {
            kdc_options: KDC_OPT_FORWARDABLE | KDC_OPT_RENEWABLE | KDC_OPT_CANONICALIZE,
            cname: Some(PrincipalName {
                name_type: NT_PRINCIPAL,
                name_string: vec![user.to_string()],
            }),
            realm: realm.to_string(),
            sname: Some(PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["krbtgt".to_string(), realm.to_string()],
            }),
            from: None,
            till: distant_till(),
            rtime: None,
            nonce,
            etypes: vec![key.etype()],
        },
    };

    let rep = kdc_exchange(stream, &req)?;
    if rep.msg_type != KRB_AS_REP {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected AS-REP, got msg-type {}", rep.msg_type),
        ));
    }

    let plain = key
        .decrypt(USAGE_AS_REP_ENCPART, &rep.enc_part.cipher)
        .map_err(to_io)?;
    let enc_part = EncKdcRepPart::decode(&plain).map_err(to_io)?;
    if enc_part.nonce != nonce {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "AS-REP nonce does not match the request",
        ));
    }

    let session_key =
        AesKey::from_key(enc_part.key.keytype, enc_part.key.keyvalue).map_err(to_io)?;
    Ok((rep.ticket, session_key))
}

/// Connect to `kdc_addr` (a `host:port` string, e.g. `"kdc.example.com:88"`)
/// and run [`get_tgt`] with OS-supplied randomness and the current time,
/// deriving `key` from `password` using the RFC 3961 default salt (`realm`
/// followed by `user`, e.g. `"EXAMPLE.COMalice"`).
pub fn fetch_tgt(
    kdc_addr: &str,
    realm: &str,
    user: &str,
    password: &str,
) -> io::Result<(Ticket, AesKey)> {
    let salt = format!("{realm}{user}");
    let key = AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, password, salt.as_bytes())
        .map_err(to_io)?;

    let mut seed = [0u8; 20];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let nonce = u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]]);
    let mut confounder = [0u8; 16];
    confounder.copy_from_slice(&seed[4..20]);

    let mut stream = TcpStream::connect(kdc_addr)?;
    get_tgt(
        &mut stream,
        realm,
        user,
        &key,
        nonce,
        confounder,
        SystemTime::now(),
    )
}

/// Trade a Ticket-Granting Ticket for a service ticket over an
/// already-connected `stream`, via the Ticket-Granting Service (TGS)
/// exchange (RFC 4120 3.3). `crealm`/`cname` identify the principal the TGT
/// was issued to (echo what [`get_tgt`] used); `tgt`/`tgt_session_key` are
/// what it returned. Only the same-realm case is handled (`service_realm`
/// should be `tgt`'s own realm) — no cross-realm referral chasing.
///
/// `nonce`/`confounder`/`now` are the exchange's nondeterministic inputs,
/// supplied by the caller so the exchange is testable; [`fetch_ap_req`] is
/// the convenience wrapper that supplies OS randomness and the current time.
///
/// Returns the service ticket and its session key.
#[allow(clippy::too_many_arguments)]
pub fn tgs_exchange(
    stream: &mut TcpStream,
    crealm: &str,
    cname: &PrincipalName,
    tgt: &Ticket,
    tgt_session_key: &AesKey,
    service_realm: &str,
    service: PrincipalName,
    nonce: u32,
    confounder: [u8; 16],
    now: SystemTime,
) -> io::Result<(Ticket, AesKey)> {
    let (ctime, cusec) = kerberos_time_and_usec(now);
    let authenticator = Authenticator {
        crealm: crealm.to_string(),
        cname: cname.clone(),
        cksum: None,
        cusec,
        ctime,
        subkey: None,
        seq_number: None,
    };
    let ap_req = ApReq {
        ap_options: 0,
        ticket: tgt.clone(),
        authenticator: EncryptedData {
            etype: tgt_session_key.etype(),
            kvno: None,
            cipher: tgt_session_key.encrypt(
                USAGE_TGS_REQ_AUTHENTICATOR,
                &authenticator.encode(),
                &confounder,
            ),
        },
    };
    let padata = vec![PaData {
        padata_type: PA_TGS_REQ,
        padata_value: ap_req.encode(),
    }];

    let req = KdcReq {
        msg_type: KRB_TGS_REQ,
        padata,
        req_body: KdcReqBody {
            kdc_options: KDC_OPT_FORWARDABLE | KDC_OPT_CANONICALIZE,
            cname: None,
            realm: service_realm.to_string(),
            sname: Some(service),
            from: None,
            till: distant_till(),
            rtime: None,
            nonce,
            etypes: vec![tgt_session_key.etype()],
        },
    };

    let rep = kdc_exchange(stream, &req)?;
    if rep.msg_type != KRB_TGS_REP {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected TGS-REP, got msg-type {}", rep.msg_type),
        ));
    }

    let plain = tgt_session_key
        .decrypt(USAGE_TGS_REP_ENCPART, &rep.enc_part.cipher)
        .map_err(to_io)?;
    let enc_part = EncKdcRepPart::decode(&plain).map_err(to_io)?;
    if enc_part.nonce != nonce {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TGS-REP nonce does not match the request",
        ));
    }

    let session_key =
        AesKey::from_key(enc_part.key.keytype, enc_part.key.keyvalue).map_err(to_io)?;
    Ok((rep.ticket, session_key))
}

/// Build a real AP-REQ from a service ticket and its session key — no
/// network access needed. `crealm`/`cname` identify the principal the
/// ticket belongs to; `confounder`/`now` are the Authenticator's
/// nondeterministic inputs.
///
/// This is the last step [`fetch_ap_req`] performs after the AS and TGS
/// exchanges; call it directly if a service ticket is already on hand from
/// elsewhere (e.g. a credential cache).
pub fn build_ap_req(
    ticket: &Ticket,
    session_key: &AesKey,
    crealm: &str,
    cname: PrincipalName,
    confounder: [u8; 16],
    now: SystemTime,
) -> ApReq {
    let (ctime, cusec) = kerberos_time_and_usec(now);
    let authenticator = Authenticator {
        crealm: crealm.to_string(),
        cname,
        cksum: None,
        cusec,
        ctime,
        subkey: None,
        seq_number: None,
    };
    ApReq {
        ap_options: 0,
        ticket: ticket.clone(),
        authenticator: EncryptedData {
            etype: session_key.etype(),
            kvno: None,
            cipher: session_key.encrypt(
                USAGE_AP_REQ_AUTHENTICATOR,
                &authenticator.encode(),
                &confounder,
            ),
        },
    }
}

/// Get an AP-REQ for `service` (e.g. `TERMSRV/host.example.com`) from just
/// a realm/username/password: [`fetch_tgt`], [`tgs_exchange`], then
/// [`build_ap_req`] — the two KDC round trips plus local assembly needed to
/// drive `crate::tls::connect_tls_kerberos`, which takes exactly this
/// function's return type. Opens a separate TCP connection per KDC request,
/// matching how most deployed KDCs expect one exchange per connection.
pub fn fetch_ap_req(
    kdc_addr: &str,
    realm: &str,
    user: &str,
    password: &str,
    service: PrincipalName,
) -> io::Result<(Vec<u8>, AesKey)> {
    let (tgt, tgt_session_key) = fetch_tgt(kdc_addr, realm, user, password)?;
    let cname = PrincipalName {
        name_type: NT_PRINCIPAL,
        name_string: vec![user.to_string()],
    };

    let mut seed = [0u8; 20];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let nonce = u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]]);
    let mut confounder = [0u8; 16];
    confounder.copy_from_slice(&seed[4..20]);

    let mut stream = TcpStream::connect(kdc_addr)?;
    let (service_ticket, service_key) = tgs_exchange(
        &mut stream,
        realm,
        &cname,
        &tgt,
        &tgt_session_key,
        realm,
        service,
        nonce,
        confounder,
        SystemTime::now(),
    )?;

    let mut ap_confounder = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut ap_confounder)?;
    let ap_req = build_ap_req(
        &service_ticket,
        &service_key,
        realm,
        cname,
        ap_confounder,
        SystemTime::now(),
    );
    Ok((ap_req.encode(), service_key))
}

/// Send a request and read back one framed response, or map a `KRB-ERROR`
/// reply to an [`io::Error`].
fn kdc_exchange(stream: &mut TcpStream, req: &KdcReq) -> io::Result<KdcRep> {
    let resp = send_and_receive(stream, &req.encode())?;
    match KdcRep::decode(&resp) {
        Ok(rep) => Ok(rep),
        Err(decode_err) => match KrbError::decode(&resp) {
            Ok(err) => Err(krb_error_to_io(&err)),
            Err(_) => Err(to_io(decode_err)),
        },
    }
}

/// RFC 4120 7.2.2 TCP framing: a 4-byte network-byte-order length prefix
/// precedes each message, both directions.
fn send_and_receive(stream: &mut TcpStream, msg: &[u8]) -> io::Result<Vec<u8>> {
    stream.write_all(&(msg.len() as u32).to_be_bytes())?;
    stream.write_all(msg)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let mut resp = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    stream.read_exact(&mut resp)?;
    Ok(resp)
}

/// `now` as a `(KerberosTime, microseconds)` pair, the shape both
/// `PA-ENC-TS-ENC` and `Authenticator.{ctime,cusec}` need.
fn kerberos_time_and_usec(now: SystemTime) -> (KerberosTime, i32) {
    let dur = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (y, mo, d, h, mi, s) = civil_from_unix(dur.as_secs() as i64);
    (
        KerberosTime::from_utc(y, mo, d, h, mi, s),
        dur.subsec_micros() as i32,
    )
}

/// `PA-ENC-TS-ENC ::= SEQUENCE { patimestamp [0] KerberosTime, pausec [1]
/// Microseconds OPTIONAL }` (RFC 4120 5.2.7.2), for `now`.
fn pa_enc_ts_enc(now: SystemTime) -> Vec<u8> {
    let (time, usec) = kerberos_time_and_usec(now);
    let mut body = Writer::new();
    asn1::write_context_time(&mut body, 0, &time);
    asn1::write_context_int32(&mut body, 1, usec);
    let mut out = Writer::new();
    crate::ber::write_tlv(&mut out, crate::ber::TAG_SEQUENCE, body.as_slice());
    out.into_vec()
}

/// Unix timestamp (seconds since epoch, UTC) to civil `(year, month, day,
/// hour, min, sec)`, via Howard Hinnant's `civil_from_days` (public domain;
/// <https://howardhinnant.github.io/date_algorithms.html>). Dependency-free
/// replacement for a calendar library, correct for the proleptic Gregorian
/// calendar (all dates this module deals with).
fn civil_from_unix(secs: i64) -> (u32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (hour, min, sec) = (
        (rem / 3600) as u32,
        ((rem / 60) % 60) as u32,
        (rem % 60) as u32,
    );

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = (if month <= 2 { y + 1 } else { y }) as u32;
    (year, month, day, hour, min, sec)
}

/// Turn a `KRB-ERROR` into an [`io::Error`], marking
/// [`KDC_ERR_PREAUTH_REQUIRED`] specifically since it means the client's
/// pre-authentication guess (salt, in particular) didn't match what the KDC
/// expects.
fn krb_error_to_io(err: &KrbError) -> io::Error {
    let kind = if err.error_code == KDC_ERR_PREAUTH_REQUIRED {
        io::ErrorKind::PermissionDenied
    } else {
        io::ErrorKind::Other
    };
    let text = err
        .e_text
        .as_deref()
        .map(|t| format!(": {t}"))
        .unwrap_or_default();
    io::Error::new(kind, format!("KRB-ERROR {}{text}", err.error_code))
}

/// Map a codec [`crate::Error`] into an [`io::Error`].
fn to_io(e: Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::krb5::asn1::EncryptionKey;
    use std::net::TcpListener;
    use std::thread;

    fn read_framed(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let mut buf = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        stream.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn write_framed(stream: &mut TcpStream, msg: &[u8]) -> io::Result<()> {
        stream.write_all(&(msg.len() as u32).to_be_bytes())?;
        stream.write_all(msg)?;
        stream.flush()
    }

    /// Hand-build a well-formed `EncASRepPart`/`EncTGSRepPart` (`app_tag` 25
    /// or 26), the one piece a mock KDC needs that [`EncKdcRepPart`]
    /// (deliberately decode-only, surfacing only the fields a client needs)
    /// doesn't provide an encoder for. `authtime` is arbitrary — real
    /// clients never see it.
    fn encode_enc_kdc_rep_part(
        app_tag: u8,
        key: &EncryptionKey,
        nonce: u32,
        srealm: &str,
        sname: &PrincipalName,
    ) -> Vec<u8> {
        let mut body = Writer::new();
        asn1::write_context(&mut body, 0, &key.encode());
        let mut empty_seq_of = Writer::new();
        crate::ber::write_tlv(&mut empty_seq_of, crate::ber::TAG_SEQUENCE, &[]);
        asn1::write_context(&mut body, 1, empty_seq_of.as_slice()); // last-req
        asn1::write_context_uint32(&mut body, 2, nonce);
        asn1::write_context_flags(&mut body, 4, 0);
        asn1::write_context_time(&mut body, 5, &KerberosTime::from_utc(2026, 1, 1, 0, 0, 0)); // authtime
        asn1::write_context_time(&mut body, 7, &distant_till()); // endtime
        asn1::write_context_general_string(&mut body, 9, srealm);
        asn1::write_context(&mut body, 10, &sname.encode());

        let mut seq = Writer::new();
        crate::ber::write_tlv(&mut seq, crate::ber::TAG_SEQUENCE, body.as_slice());
        let mut out = Writer::new();
        asn1::write_application(&mut out, app_tag, seq.as_slice());
        out.into_vec()
    }

    #[test]
    fn civil_from_unix_matches_known_dates() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_unix(86399), (1970, 1, 1, 23, 59, 59));
        assert_eq!(civil_from_unix(86400), (1970, 1, 2, 0, 0, 0));
        // 2000-03-01 00:00:00 UTC (951868800) -- crosses a leap-year boundary.
        assert_eq!(civil_from_unix(951_868_800), (2000, 3, 1, 0, 0, 0));
        // 2024-02-29 12:30:05 UTC (1709209805) -- a leap day.
        assert_eq!(civil_from_unix(1_709_209_805), (2024, 2, 29, 12, 30, 5));
    }

    #[test]
    fn get_tgt_completes_as_exchange_against_mock_kdc() {
        let realm = "EXAMPLE.COM";
        let user = "alice";
        let password = "s3cr3t";
        let key =
            AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, password, b"EXAMPLE.COMalice")
                .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let session_key_bytes = vec![0x42u8; 32];
        let session_key_bytes_for_server = session_key_bytes.clone();
        let realm_for_server = realm.to_string();

        let server = thread::spawn(move || {
            // AesKey isn't Clone; the mock KDC derives its own copy of the
            // same key from the same password/salt rather than sharing one.
            let server_key =
                AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, password, b"EXAMPLE.COMalice")
                    .unwrap();

            let (mut stream, _) = listener.accept().unwrap();
            let req_bytes = read_framed(&mut stream).unwrap();
            let req = KdcReq::decode(&req_bytes).unwrap();
            assert_eq!(req.msg_type, KRB_AS_REQ);
            assert_eq!(req.req_body.realm, realm_for_server);
            assert_eq!(
                req.req_body.cname.as_ref().unwrap().name_string,
                vec!["alice".to_string()]
            );

            // Verify the PA-ENC-TIMESTAMP decrypts and is well-formed --
            // this is the actual password check a real KDC performs.
            let pa = req
                .padata
                .iter()
                .find(|p| p.padata_type == PA_ENC_TIMESTAMP)
                .unwrap();
            let enc = EncryptedData::decode(&pa.padata_value).unwrap();
            assert!(server_key
                .decrypt(USAGE_AS_REQ_PA_ENC_TIMESTAMP, &enc.cipher)
                .is_ok());

            let sname = PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["krbtgt".to_string(), realm_for_server.clone()],
            };
            let enc_as_rep_part = encode_enc_kdc_rep_part(
                25,
                &EncryptionKey {
                    keytype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    keyvalue: session_key_bytes_for_server.clone(),
                },
                req.req_body.nonce,
                &realm_for_server,
                &sname,
            );

            let rep = KdcRep {
                msg_type: KRB_AS_REP,
                padata: Vec::new(),
                crealm: realm_for_server.clone(),
                cname: req.req_body.cname.clone().unwrap(),
                ticket: Ticket {
                    realm: realm_for_server.clone(),
                    sname: sname.clone(),
                    // Opaque to the client; arbitrary bytes are fine here.
                    enc_part: EncryptedData {
                        etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                        kvno: Some(1),
                        cipher: vec![0xAB; 64],
                    },
                },
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: None,
                    cipher: server_key.encrypt(
                        USAGE_AS_REP_ENCPART,
                        &enc_as_rep_part,
                        &[0x11u8; 16],
                    ),
                },
            };
            write_framed(&mut stream, &rep.encode()).unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        let (ticket, session_key) = get_tgt(
            &mut stream,
            realm,
            user,
            &key,
            0xDEAD_BEEF,
            [0x22u8; 16],
            SystemTime::now(),
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(ticket.realm, realm);
        assert_eq!(
            ticket.sname.name_string,
            vec!["krbtgt".to_string(), realm.to_string()]
        );
        assert_eq!(session_key.key(), session_key_bytes.as_slice());
        assert_eq!(session_key.etype(), ETYPE_AES256_CTS_HMAC_SHA1_96);
    }

    #[test]
    fn get_tgt_surfaces_krb_error() {
        let key =
            AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, "s3cr3t", b"EXAMPLE.COMalice")
                .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _req_bytes = read_framed(&mut stream).unwrap();
            let err = KrbError {
                stime: KerberosTime::from_utc(2026, 1, 1, 0, 0, 0),
                susec: 0,
                error_code: KDC_ERR_PREAUTH_REQUIRED,
                realm: "EXAMPLE.COM".to_string(),
                sname: PrincipalName {
                    name_type: NT_SRV_INST,
                    name_string: vec!["krbtgt".to_string(), "EXAMPLE.COM".to_string()],
                },
                e_text: Some("wrong salt".to_string()),
                e_data: None,
            };
            write_framed(&mut stream, &err.encode()).unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        let result = get_tgt(
            &mut stream,
            "EXAMPLE.COM",
            "alice",
            &key,
            1,
            [0u8; 16],
            SystemTime::now(),
        );
        server.join().unwrap();
        let err = match result {
            Ok(_) => panic!("expected get_tgt to fail on a KRB-ERROR reply"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn build_ap_req_produces_verifiable_authenticator() {
        let session_key =
            AesKey::from_key(ETYPE_AES256_CTS_HMAC_SHA1_96, vec![0x77u8; 32]).unwrap();
        let ticket = Ticket {
            realm: "EXAMPLE.COM".to_string(),
            sname: PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["TERMSRV".to_string(), "host.example.com".to_string()],
            },
            enc_part: EncryptedData {
                etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                kvno: Some(3),
                cipher: vec![0xCD; 64],
            },
        };
        let cname = PrincipalName {
            name_type: NT_PRINCIPAL,
            name_string: vec!["alice".to_string()],
        };

        let ap_req = build_ap_req(
            &ticket,
            &session_key,
            "EXAMPLE.COM",
            cname.clone(),
            [0x99u8; 16],
            SystemTime::now(),
        );

        assert_eq!(ap_req.ap_options, 0);
        assert_eq!(ap_req.ticket, ticket);
        assert_eq!(ap_req.authenticator.etype, ETYPE_AES256_CTS_HMAC_SHA1_96);

        // A verifier (the server side) can decrypt the Authenticator with usage 11.
        let plain = session_key
            .decrypt(USAGE_AP_REQ_AUTHENTICATOR, &ap_req.authenticator.cipher)
            .unwrap();
        let authenticator = Authenticator::decode(&plain).unwrap();
        assert_eq!(authenticator.crealm, "EXAMPLE.COM");
        assert_eq!(authenticator.cname, cname);
        assert!(authenticator.subkey.is_none());

        // Round-trips through the wire encoding too.
        let decoded = ApReq::decode(&ap_req.encode()).unwrap();
        assert_eq!(decoded, ap_req);
    }

    #[test]
    fn tgs_exchange_completes_against_mock_kdc() {
        let realm = "EXAMPLE.COM";
        let tgt_session_key =
            AesKey::from_key(ETYPE_AES256_CTS_HMAC_SHA1_96, vec![0x11u8; 32]).unwrap();
        let cname = PrincipalName {
            name_type: NT_PRINCIPAL,
            name_string: vec!["alice".to_string()],
        };
        let tgt = Ticket {
            realm: realm.to_string(),
            sname: PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["krbtgt".to_string(), realm.to_string()],
            },
            enc_part: EncryptedData {
                etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                kvno: Some(1),
                cipher: vec![0xAB; 64],
            },
        };
        let service = PrincipalName {
            name_type: NT_SRV_INST,
            name_string: vec!["TERMSRV".to_string(), "host.example.com".to_string()],
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let realm_for_server = realm.to_string();
        let tgt_for_server = tgt.clone();
        let cname_for_server = cname.clone();
        let service_session_key_bytes = vec![0x55u8; 32];
        let service_session_key_bytes_for_server = service_session_key_bytes.clone();
        let tgt_session_key_bytes = tgt_session_key.key().to_vec();

        let server = thread::spawn(move || {
            let server_tgt_key =
                AesKey::from_key(ETYPE_AES256_CTS_HMAC_SHA1_96, tgt_session_key_bytes).unwrap();

            let (mut stream, _) = listener.accept().unwrap();
            let req_bytes = read_framed(&mut stream).unwrap();
            let req = KdcReq::decode(&req_bytes).unwrap();
            assert_eq!(req.msg_type, KRB_TGS_REQ);
            assert_eq!(req.req_body.realm, realm_for_server);
            assert!(req.req_body.cname.is_none());

            let pa = req
                .padata
                .iter()
                .find(|p| p.padata_type == PA_TGS_REQ)
                .unwrap();
            let ap_req = ApReq::decode(&pa.padata_value).unwrap();
            assert_eq!(ap_req.ticket, tgt_for_server);

            let plain = server_tgt_key
                .decrypt(USAGE_TGS_REQ_AUTHENTICATOR, &ap_req.authenticator.cipher)
                .unwrap();
            let authenticator = Authenticator::decode(&plain).unwrap();
            assert_eq!(authenticator.crealm, realm_for_server);
            assert_eq!(authenticator.cname, cname_for_server);

            let sname = PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["TERMSRV".to_string(), "host.example.com".to_string()],
            };
            let enc_tgs_rep_part = encode_enc_kdc_rep_part(
                26,
                &EncryptionKey {
                    keytype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    keyvalue: service_session_key_bytes_for_server.clone(),
                },
                req.req_body.nonce,
                &realm_for_server,
                &sname,
            );

            let rep = KdcRep {
                msg_type: KRB_TGS_REP,
                padata: Vec::new(),
                crealm: realm_for_server.clone(),
                cname: cname_for_server.clone(),
                ticket: Ticket {
                    realm: realm_for_server.clone(),
                    sname: sname.clone(),
                    enc_part: EncryptedData {
                        etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                        kvno: Some(2),
                        cipher: vec![0xEF; 64],
                    },
                },
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: None,
                    cipher: server_tgt_key.encrypt(
                        USAGE_TGS_REP_ENCPART,
                        &enc_tgs_rep_part,
                        &[0x22u8; 16],
                    ),
                },
            };
            write_framed(&mut stream, &rep.encode()).unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        let (service_ticket, service_key) = tgs_exchange(
            &mut stream,
            realm,
            &cname,
            &tgt,
            &tgt_session_key,
            realm,
            service,
            0x00C0_FFEE,
            [0x33u8; 16],
            SystemTime::now(),
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(
            service_ticket.sname.name_string,
            vec!["TERMSRV".to_string(), "host.example.com".to_string()]
        );
        assert_eq!(service_key.key(), service_session_key_bytes.as_slice());
    }

    #[test]
    fn fetch_ap_req_completes_against_mock_kdc() {
        let realm = "EXAMPLE.COM";
        let user = "alice";
        let password = "s3cr3t";
        let service = PrincipalName {
            name_type: NT_SRV_INST,
            name_string: vec!["TERMSRV".to_string(), "host.example.com".to_string()],
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let kdc_addr = addr.to_string();

        let tgt_session_key_bytes = vec![0x44u8; 32];
        let tgt_session_key_bytes_for_server = tgt_session_key_bytes.clone();
        let service_session_key_bytes = vec![0x55u8; 32];
        let service_session_key_bytes_for_server = service_session_key_bytes.clone();
        let realm_for_server = realm.to_string();

        let server = thread::spawn(move || {
            let key =
                AesKey::from_password(ETYPE_AES256_CTS_HMAC_SHA1_96, password, b"EXAMPLE.COMalice")
                    .unwrap();

            // Leg 1: AS-REQ / AS-REP.
            let (mut stream, _) = listener.accept().unwrap();
            let req_bytes = read_framed(&mut stream).unwrap();
            let req = KdcReq::decode(&req_bytes).unwrap();
            assert_eq!(req.msg_type, KRB_AS_REQ);
            let cname = req.req_body.cname.clone().unwrap();

            let krbtgt_sname = PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["krbtgt".to_string(), realm_for_server.clone()],
            };
            let enc_as_rep_part = encode_enc_kdc_rep_part(
                25,
                &EncryptionKey {
                    keytype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    keyvalue: tgt_session_key_bytes_for_server.clone(),
                },
                req.req_body.nonce,
                &realm_for_server,
                &krbtgt_sname,
            );
            let tgt = Ticket {
                realm: realm_for_server.clone(),
                sname: krbtgt_sname,
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: Some(1),
                    cipher: vec![0xAB; 64],
                },
            };
            let as_rep = KdcRep {
                msg_type: KRB_AS_REP,
                padata: Vec::new(),
                crealm: realm_for_server.clone(),
                cname: cname.clone(),
                ticket: tgt.clone(),
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: None,
                    cipher: key.encrypt(USAGE_AS_REP_ENCPART, &enc_as_rep_part, &[0x11u8; 16]),
                },
            };
            write_framed(&mut stream, &as_rep.encode()).unwrap();
            drop(stream);

            // Leg 2: TGS-REQ / TGS-REP, on a fresh connection.
            let (mut stream, _) = listener.accept().unwrap();
            let req_bytes = read_framed(&mut stream).unwrap();
            let req = KdcReq::decode(&req_bytes).unwrap();
            assert_eq!(req.msg_type, KRB_TGS_REQ);

            let tgt_key = AesKey::from_key(
                ETYPE_AES256_CTS_HMAC_SHA1_96,
                tgt_session_key_bytes_for_server.clone(),
            )
            .unwrap();
            let pa = req
                .padata
                .iter()
                .find(|p| p.padata_type == PA_TGS_REQ)
                .unwrap();
            let ap_req = ApReq::decode(&pa.padata_value).unwrap();
            assert_eq!(ap_req.ticket, tgt);
            let plain = tgt_key
                .decrypt(USAGE_TGS_REQ_AUTHENTICATOR, &ap_req.authenticator.cipher)
                .unwrap();
            let authenticator = Authenticator::decode(&plain).unwrap();
            assert_eq!(authenticator.cname, cname);

            let service_sname = PrincipalName {
                name_type: NT_SRV_INST,
                name_string: vec!["TERMSRV".to_string(), "host.example.com".to_string()],
            };
            let enc_tgs_rep_part = encode_enc_kdc_rep_part(
                26,
                &EncryptionKey {
                    keytype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    keyvalue: service_session_key_bytes_for_server.clone(),
                },
                req.req_body.nonce,
                &realm_for_server,
                &service_sname,
            );
            let service_ticket = Ticket {
                realm: realm_for_server.clone(),
                sname: service_sname,
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: Some(2),
                    cipher: vec![0xEF; 64],
                },
            };
            let tgs_rep = KdcRep {
                msg_type: KRB_TGS_REP,
                padata: Vec::new(),
                crealm: realm_for_server.clone(),
                cname,
                ticket: service_ticket.clone(),
                enc_part: EncryptedData {
                    etype: ETYPE_AES256_CTS_HMAC_SHA1_96,
                    kvno: None,
                    cipher: tgt_key.encrypt(
                        USAGE_TGS_REP_ENCPART,
                        &enc_tgs_rep_part,
                        &[0x22u8; 16],
                    ),
                },
            };
            write_framed(&mut stream, &tgs_rep.encode()).unwrap();
            service_ticket
        });

        let (ap_req_bytes, service_key) =
            fetch_ap_req(&kdc_addr, realm, user, password, service).unwrap();
        let service_ticket = server.join().unwrap();

        assert_eq!(service_key.key(), service_session_key_bytes.as_slice());

        // The AP-REQ decodes and carries the service ticket.
        let ap_req = ApReq::decode(&ap_req_bytes).unwrap();
        assert_eq!(ap_req.ticket, service_ticket);

        // Its Authenticator decrypts with the service session key.
        let plain = service_key
            .decrypt(USAGE_AP_REQ_AUTHENTICATOR, &ap_req.authenticator.cipher)
            .unwrap();
        let authenticator = Authenticator::decode(&plain).unwrap();
        assert_eq!(authenticator.crealm, realm);
        assert_eq!(authenticator.cname.name_string, vec!["alice".to_string()]);
    }
}
