use anyhow::{bail, Result};
use url::Url;

pub fn destination(input: &str) -> Result<Url> {
    let input = input.trim();
    if input.is_empty() {
        bail!("Saisis une adresse ou quelques mots à rechercher.");
    }
    if input.contains("://") {
        return web_url(input);
    }
    if let Some((scheme, rest)) = input.split_once(':') {
        if scheme != "localhost"
            && !scheme.contains('.')
            && !scheme.starts_with('[')
            && !scheme.is_empty()
            && scheme.bytes().enumerate().all(|(i, c)| {
                c.is_ascii_alphabetic() || (i > 0 && (c.is_ascii_digit() || b"+-.".contains(&c)))
            })
            && !rest.starts_with(' ')
        {
            bail!("Seules les adresses HTTP et HTTPS sont prises en charge.");
        }
    }
    let host_like = !input.chars().any(char::is_whitespace)
        && (input.starts_with("localhost")
            || input.starts_with('[')
            || input
                .split('/')
                .next()
                .is_some_and(|host| host.contains('.')));
    if host_like && !input.contains("://") {
        let scheme = if input.starts_with("localhost")
            || input.starts_with("127.")
            || input.starts_with("[::1]")
        {
            "http"
        } else {
            "https"
        };
        return web_url(&format!("{scheme}://{input}"));
    }
    let mut url = Url::parse("https://duckduckgo.com/")?;
    url.query_pairs_mut().append_pair("q", input);
    Ok(url)
}

pub fn web_url(input: &str) -> Result<Url> {
    let url = Url::parse(input)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Seules les adresses HTTP et HTTPS sont prises en charge.");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_searches_have_distinct_destinations() {
        assert_eq!(
            destination(" example.org/docs ").unwrap().as_str(),
            "https://example.org/docs"
        );
        assert_eq!(
            destination("localhost:8080/a").unwrap().as_str(),
            "http://localhost:8080/a"
        );
        let query = destination("rust & arbres + web").unwrap();
        assert_eq!(query.query_pairs().next().unwrap().1, "rust & arbres + web");
    }

    #[test]
    fn unsupported_or_empty_destinations_do_not_navigate() {
        for input in [
            "",
            "   ",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "mailto:hello@example.org",
            "data:text/html,hello",
            "https://",
        ] {
            assert!(destination(input).is_err(), "{input}");
        }
    }
}
