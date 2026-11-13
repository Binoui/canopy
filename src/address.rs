use anyhow::{bail, Result};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Destination {
    Address(Url),
    Search(Url),
}

pub fn destination(input: &str) -> Result<Destination> {
    let input = input.trim();
    if input.is_empty() {
        bail!("Type an address or a few words to search for.");
    }
    if input.contains("://") {
        return web_url(input).map(Destination::Address);
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
            bail!("Only HTTP and HTTPS addresses are supported.");
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
        return web_url(&format!("{scheme}://{input}")).map(Destination::Address);
    }
    Ok(Destination::Search(web_search(input)))
}

pub fn web_search(query: &str) -> Url {
    let mut url = Url::parse("https://duckduckgo.com/").expect("a valid search URL");
    url.query_pairs_mut().append_pair("q", query.trim());
    url
}

pub fn web_url(input: &str) -> Result<Url> {
    let url = Url::parse(input)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Only HTTP and HTTPS addresses are supported.");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_searches_have_distinct_destinations() {
        assert_eq!(
            destination(" example.org/docs ").unwrap(),
            Destination::Address(Url::parse("https://example.org/docs").unwrap())
        );
        assert_eq!(
            destination("localhost:8080/a").unwrap(),
            Destination::Address(Url::parse("http://localhost:8080/a").unwrap())
        );
        let Destination::Search(query) = destination("rust & arbres + web").unwrap() else {
            panic!("words are a search");
        };
        assert_eq!(query.query_pairs().next().unwrap().1, "rust & arbres + web");
        assert_eq!(
            web_search(" example.org ").query_pairs().next().unwrap().1,
            "example.org"
        );
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
