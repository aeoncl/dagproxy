use rcgen::{Issuer, SanType, SigningKey};
use std::time::Duration;
use rcgen::{BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rcgen::string::Ia5String;
use time::OffsetDateTime;

fn generate_root_ca() -> Result<(Certificate, KeyPair), rcgen::Error> {
    // Create custom parameters for the root CA
    let mut params = CertificateParams::default();

    // Set the CA certificate to be valid for 10 years
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::from_secs(10 * 365 * 24 * 60 * 60);

    // Set this as a CA certificate
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);

    // Set key usages appropriate for a CA
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];

    // Set CA name information
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, "DagProxy Root CA");
    distinguished_name.push(DnType::OrganizationName, "DagProxy");
    distinguished_name.push(DnType::CountryName, "US");
    params.distinguished_name = distinguished_name;

    // Generate a key pair for the CA
    let key_pair = KeyPair::generate()?;

    // Create and self-sign the CA certificate
    let cert = params.self_signed(&key_pair).unwrap();

    Ok((cert, key_pair))
}

fn issue_certificate(
    ca_cert: &Certificate,
    ca_key_pair: &KeyPair,
    domain_name: &str
) -> Result<(Certificate), rcgen::Error> {
    // Create parameters for the leaf certificate
    let mut params = CertificateParams::default();

    // Set the certificate to be valid for 1 year
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::from_secs(365 * 24 * 60 * 60);

    // Add the domain name as both Common Name and Subject Alternative Name
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, domain_name);
    params.distinguished_name = distinguished_name;

    // Add Subject Alternative Name
    params.subject_alt_names.push(SanType::DnsName(Ia5String::try_from(domain_name).unwrap()));

    // This is NOT a CA certificate
    params.is_ca = IsCa::NoCa;

    // Set key usages for a server certificate
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];


    // Create an issuer from the CA certificate and private key
    let issuer = Issuer::from_ca_cert_der(ca_cert.der(), &ca_key_pair)?;

    // Create and sign the certificate with the CA
    let cert = params.signed_by(&ca_key_pair, &issuer)?;
    Ok(cert)
}

fn get_issuer_from_pem<'a>(pem_str: &str, private_key_pem: &str) -> Result<Issuer<'a, KeyPair>, rcgen::Error> {
    let signing_key = KeyPair::from_pem(&private_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(&pem_str, signing_key)?;
    Ok(issuer)
}



#[cfg(test)]
mod tests{
    use rcgen::{generate_simple_self_signed, CertificateParams, CertifiedKey};
    use crate::cert::{generate_root_ca, issue_certificate};

    #[test]
    fn test_cert(){
        let (cert, signing_key) = generate_root_ca().unwrap();


        println!("{}", cert.pem());
        println!("{}", signing_key.serialize_pem());
    }

    #[test]
    fn test_cert_2(){
        let (cert, signing_key) = generate_root_ca().unwrap();
        let test = issue_certificate(&cert, &signing_key, "shlasouf.com").unwrap();
        println!("{}", test.pem());
    }


}