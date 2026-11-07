use reqwest::{Certificate, Client, ClientBuilder};
use std::ffi::OsStr;
use std::path::Path;

pub(super) fn client_builder() -> Result<ClientBuilder, String> {
    let file = std::env::var_os("SSL_CERT_FILE");
    let dirs = std::env::var_os("SSL_CERT_DIR");
    client_builder_with_sources(file.as_deref(), dirs.as_deref())
}

fn client_builder_with_sources(
    file: Option<&OsStr>,
    dirs: Option<&OsStr>,
) -> Result<ClientBuilder, String> {
    let file = file.filter(|value| !value.is_empty());
    let dirs = dirs.filter(|value| !value.is_empty());
    if file.is_none() && dirs.is_none() {
        return Ok(Client::builder().use_native_tls());
    }

    let mut certs = Vec::new();
    if let Some(file) = file {
        load_certs(Some(Path::new(file)), None, &mut certs)?;
    }
    if let Some(dirs) = dirs {
        for dir in std::env::split_paths(dirs).filter(|path| !path.as_os_str().is_empty()) {
            load_certs(None, Some(&dir), &mut certs)?;
        }
    }
    if certs.is_empty() {
        return Err("SSL_CERT_FILE / SSL_CERT_DIR: no certificates found".into());
    }
    let mut builder = Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false);
    for cert in certs {
        builder = builder.add_root_certificate(cert);
    }
    Ok(builder)
}

fn load_certs(
    file: Option<&Path>,
    dir: Option<&Path>,
    certs: &mut Vec<Certificate>,
) -> Result<(), String> {
    let source = if file.is_some() {
        "SSL_CERT_FILE"
    } else {
        "SSL_CERT_DIR"
    };
    let result = rustls_native_certs::load_certs_from_paths(file, dir);
    if !result.errors.is_empty() {
        return Err(format!("{source}: {:?}", result.errors));
    }
    for cert in result.certs {
        certs.push(
            Certificate::from_der(cert.as_ref())
                .map_err(|e| format!("{source}: invalid certificate: {e:?}"))?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn client_builds_without_explicit_sources() {
        client_builder_with_sources(None, None)
            .unwrap()
            .build()
            .unwrap();
        client_builder_with_sources(Some(OsStr::new("")), Some(OsStr::new("")))
            .unwrap()
            .build()
            .unwrap();
    }

    #[test]
    fn loads_all_certificates_from_file_and_multiple_directories() {
        let temp = tempfile::tempdir().unwrap();
        let first = rcgen::generate_simple_self_signed(vec!["first.example".into()]).unwrap();
        let second = rcgen::generate_simple_self_signed(vec!["second.example".into()]).unwrap();
        let third = rcgen::generate_simple_self_signed(vec!["third.example".into()]).unwrap();
        let file = temp.path().join("bundle");
        fs::write(&file, format!("{}{}", first.cert.pem(), second.cert.pem())).unwrap();
        let dir1 = temp.path().join("one");
        let dir2 = temp.path().join("two");
        fs::create_dir(&dir1).unwrap();
        fs::create_dir(&dir2).unwrap();
        fs::write(dir1.join("ca"), third.cert.pem()).unwrap();
        fs::write(dir2.join("ca"), first.cert.pem()).unwrap();

        let mut certs = Vec::new();
        load_certs(Some(&file), None, &mut certs).unwrap();
        assert_eq!(certs.len(), 2);
        load_certs(None, Some(&dir1), &mut certs).unwrap();
        load_certs(None, Some(&dir2), &mut certs).unwrap();
        assert_eq!(certs.len(), 4);
        let dirs = std::env::join_paths([&dir1, &dir2]).unwrap();
        client_builder_with_sources(Some(file.as_os_str()), Some(&dirs))
            .unwrap()
            .build()
            .unwrap();
    }

    #[test]
    fn invalid_sources_fail_instead_of_trusting_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        assert!(client_builder_with_sources(Some(missing.as_os_str()), None)
            .unwrap_err()
            .contains("SSL_CERT_FILE"));
        assert!(client_builder_with_sources(None, Some(missing.as_os_str()))
            .unwrap_err()
            .contains("SSL_CERT_DIR"));
        let empty = temp.path().join("empty");
        fs::write(&empty, "").unwrap();
        assert!(client_builder_with_sources(Some(empty.as_os_str()), None)
            .unwrap_err()
            .contains("no certificates"));
    }
}
