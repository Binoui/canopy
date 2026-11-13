use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use zeroize::Zeroizing;

use super::ImportError;

pub(super) struct ChromeKeys {
    v10: Zeroizing<[u8; 16]>,
    v11: std::cell::OnceCell<Result<Zeroizing<[u8; 16]>, ImportError>>,
}

impl ChromeKeys {
    pub(super) fn new() -> Self {
        Self {
            v10: derive(b"peanuts"),
            v11: std::cell::OnceCell::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_v11_password(password: &[u8]) -> Self {
        let keys = Self::new();
        let _ = keys.v11.set(Ok(derive(password)));
        keys
    }

    pub(super) fn decrypt(&self, value: &[u8]) -> Result<Zeroizing<Vec<u8>>, ImportError> {
        let key = match value.get(..3) {
            Some(b"v10") => &self.v10,
            Some(b"v11") => self
                .v11
                .get_or_init(|| {
                    let password = crate::key_store::read(&[("application", "chrome")])
                        .map_err(|_| ImportError::KeyringUnavailable)?
                        .ok_or(ImportError::KeyringUnavailable)?;
                    Ok(derive(&password))
                })
                .as_ref()
                .map_err(|e| *e)?,
            _ => return Err(ImportError::Undecryptable),
        };
        let mut bytes = Zeroizing::new(value[3..].to_vec());
        let plain = cbc::Decryptor::<aes::Aes128>::new((&**key).into(), (&[b' '; 16]).into())
            .decrypt_padded_mut::<Pkcs7>(&mut bytes)
            .map_err(|_| ImportError::Undecryptable)?;
        let len = plain.len();
        bytes.truncate(len);
        Ok(bytes)
    }
}

fn derive(password: &[u8]) -> Zeroizing<[u8; 16]> {
    let mut key = Zeroizing::new([0; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1, &mut *key);
    key
}

#[cfg(test)]
pub(super) fn encrypt(password: &[u8], tag: &[u8], plaintext: &[u8]) -> Vec<u8> {
    use aes::cipher::BlockEncryptMut;
    let key = derive(password);
    let mut out = tag.to_vec();
    out.extend(
        cbc::Encryptor::<aes::Aes128>::new((&*key).into(), (&[b' '; 16]).into())
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_versions_and_redaction() {
        let keys = ChromeKeys::with_v11_password(b"fake-storage");
        for (password, tag) in [
            (b"peanuts".as_slice(), b"v10"),
            (b"fake-storage".as_slice(), b"v11"),
        ] {
            let encrypted = encrypt(password, tag, b"fake-password");
            let secret =
                crate::secret::Secret::from_utf8(keys.decrypt(&encrypted).unwrap()).unwrap();
            assert!(secret.expose() == "fake-password");
            assert!(!format!("{secret:?}").contains("fake-password"));
        }
    }
}
