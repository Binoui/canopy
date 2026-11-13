use zeroize::Zeroizing;

pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn from_utf8(bytes: Zeroizing<Vec<u8>>) -> Option<Self> {
        std::str::from_utf8(&bytes)
            .ok()
            .map(|s| Self::new(s.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}
