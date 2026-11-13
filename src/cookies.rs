use crate::secret::Secret;

#[derive(Debug)]
pub struct Cookie {
    pub domain: CookieDomain,
    pub path: String,
    pub name: String,
    pub value: Secret,
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CookieDomain {
    Host(String),
    Domain(String),
}

impl CookieDomain {
    pub fn parse(dotted: &str) -> Option<Self> {
        let (domain, host) = match dotted.strip_prefix('.') {
            Some(host) => (true, host),
            None => (false, dotted),
        };
        if host.is_empty()
            || host.chars().any(|c| c.is_control() || c.is_whitespace())
            || url::Host::parse(host).is_err()
        {
            return None;
        }
        Some(if domain {
            Self::Domain(host.into())
        } else {
            Self::Host(host.into())
        })
    }

    pub fn dotted(&self) -> String {
        match self {
            Self::Host(host) => host.clone(),
            Self::Domain(host) => format!(".{host}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

impl Cookie {
    pub(crate) fn valid(&self) -> bool {
        !self.name.is_empty()
            && ![
                self.name.as_str(),
                self.path.as_str(),
                self.value.expose(),
                &self.domain.dotted(),
            ]
            .iter()
            .any(|s| s.chars().any(char::is_control))
            && (self.same_site != SameSite::None || self.secure)
            && (!self.name.starts_with("__Secure-") || self.secure)
            && (!self.name.starts_with("__Host-")
                || (self.secure
                    && matches!(self.domain, CookieDomain::Host(_))
                    && self.path == "/"))
    }
}
