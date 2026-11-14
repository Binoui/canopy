//! Names a tree from its own pages. No model: the words the user searched for when there
//! are some, otherwise the root page's title without the site's name around it.

use url::Url;

use crate::browser::Visit;

/// Title separators that set a site's name apart from the page's own title.
const SEPARATORS: [&str; 7] = [" - ", " | ", " · ", " — ", " – ", " • ", " : "];
/// Query parameters search pages keep the typed words in.
const QUERY_KEYS: [&str; 7] = [
    "q",
    "query",
    "search_query",
    "k",
    "keywords",
    "field-keywords",
    "term",
];
const MAX_QUERY: usize = 60;

/// The name of the tree rooted at `root`, whose direct children are `children` in creation
/// order.
pub fn tree_name<'a>(root: &Visit, children: impl IntoIterator<Item = &'a Visit>) -> String {
    if let Some(query) = search_query(&root.url) {
        return query;
    }
    // A tree started on a site's front page is about what was searched for there.
    if root.url.path() == "/" {
        if let Some(query) = children.into_iter().find_map(|v| search_query(&v.url)) {
            return query;
        }
    }
    page_name(&root.title, &root.url)
}

/// The words typed into a search box, when `url` is a page of search results.
pub fn search_query(url: &Url) -> Option<String> {
    let path = url.path();
    let results_page = path == "/"
        || path
            .split('/')
            .any(|part| part.starts_with("search") || matches!(part, "results" | "s" | "scholar"));
    if !results_page {
        return None;
    }
    url.query_pairs()
        .filter(|(key, _)| QUERY_KEYS.contains(&key.as_ref()))
        .find_map(|(_, value)| {
            let words = value.split_whitespace().collect::<Vec<_>>().join(" ");
            // Redirects and sign-in pages reuse `q` for addresses and tokens.
            let typed = !words.is_empty()
                && !words.contains("://")
                && words.split(' ').all(|word| word.chars().count() <= 32);
            typed.then(|| match words.char_indices().nth(MAX_QUERY) {
                Some((end, _)) => format!("{}…", words[..end].trim_end()),
                None => words,
            })
        })
}

/// A page's title without its site's name, or the site's name alone for a front page.
pub fn page_name(title: &str, url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    let title = title.trim();
    // "(84) kanjicoder" counts unread items; the count is not part of the name.
    let title = title
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(") "))
        .filter(|(count, _)| !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()))
        .map_or(title, |(_, rest)| rest.trim_start());
    if title.is_empty() {
        return if host.is_empty() { url.as_str() } else { host }.to_owned();
    }
    let brands = brands(host);
    let is_brand = |piece: &str| {
        let key = key(piece);
        !key.is_empty() && brands.contains(&key)
    };

    let mut pieces = Vec::new();
    let mut start = 0;
    while let Some((at, separator)) = SEPARATORS
        .iter()
        .filter_map(|separator| Some((start + title[start..].find(separator)?, separator)))
        .min_by_key(|&(at, _)| at)
    {
        pieces.push((start, at));
        start = at + separator.len();
    }
    pieces.push((start, title.len()));
    // "Amazon.fr: books, DVDs…" leads with the site too.
    let (first_start, first_end) = pieces[0];
    if let Some(colon) = title[first_start..first_end].find(": ") {
        if is_brand(&title[first_start..first_start + colon]) {
            pieces[0] = (first_start + colon + 2, first_end);
            pieces.insert(0, (first_start, first_start + colon));
        }
    }

    let piece = |&(start, end): &(usize, usize)| title[start..end].trim();
    let brand = pieces.iter().map(piece).find(|piece| is_brand(piece));
    if let (Some(brand), "/") = (brand, url.path()) {
        return brand.to_owned();
    }
    let (mut first, mut last) = (0, pieces.len());
    while first < last && is_brand(piece(&pieces[first])) {
        first += 1;
    }
    while first < last && is_brand(piece(&pieces[last - 1])) {
        last -= 1;
    }
    if first == last {
        return piece(&pieces[0]).to_owned();
    }
    let name = title[pieces[first].0..pieces[last - 1].1].trim();
    // "owner/repository: what it does" is named by its path.
    match name.split_once(": ") {
        Some((path, _)) if path.contains('/') && !path.contains(' ') => path,
        _ => name,
    }
    .to_owned()
}

/// The ways a site may write its own name in a title: "battle", "battlenet", "accountbattlenet".
fn brands(host: &str) -> Vec<String> {
    let labels: Vec<&str> = host.split('.').collect();
    let mut end = labels.len().saturating_sub(1).max(1);
    if end > 1 && matches!(labels[end - 1], "co" | "com" | "org" | "net" | "ac" | "gov") {
        end -= 1;
    }
    vec![
        key(labels[end - 1]),
        key(&labels[end - 1..].concat()),
        key(host),
    ]
}

/// Letters and digits only, lowercase and unaccented, so "Wikipédia" matches wikipedia.org.
fn key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' => 'a',
            'ç' => 'c',
            'è'..='ë' => 'e',
            'ì'..='ï' => 'i',
            'ñ' => 'n',
            'ò'..='ö' => 'o',
            'ù'..='ü' => 'u',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::VisitState;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn visit(address: &str, title: &str) -> Visit {
        Visit {
            id: ulid::Ulid::new().to_string().parse().unwrap(),
            parent: None,
            url: url(address),
            title: title.into(),
            created_at: 0,
            opened_at: Some(0),
            state: VisitState::Opened,
            source: None,
        }
    }

    #[test]
    fn search_pages_give_the_typed_words() {
        for (address, query) in [
            ("https://duckduckgo.com/?q=tauri", Some("tauri")),
            ("https://www.google.com/search?q=Le+passage++%C3%A0+niveau&hl=fr", Some("Le passage à niveau")),
            ("https://www.youtube.com/results?search_query=rust+gtk", Some("rust gtk")),
            ("https://www.amazon.fr/s/ref=nb_sb_noss_2?url=search-alias%3Daps&field-keywords=lampe", Some("lampe")),
            ("https://www.amazon.fr/s?k=lagostina+pan", Some("lagostina pan")),
            ("https://www.reddit.com/r/starcraft/search/?q=harmstem", Some("harmstem")),
            // Not searches: a redirect, a token, a filter, a product reached from a search.
            ("https://www.google.com/url?q=http://example.org", None),
            ("https://example.org/?q=http://example.org", None),
            ("https://auth.example.org/search?q=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9eyJhbGci", None),
            ("https://github.com/Binoui/canopy/issues?q=is%3Aissue", None),
            ("https://www.amazon.fr/-/en/Lamp/dp/B0?keywords=lampe", None),
            ("https://duckduckgo.com/?q=+", None),
        ] {
            assert_eq!(search_query(&url(address)).as_deref(), query, "{address}");
        }
        let long = search_query(&url(&format!(
            "https://duckduckgo.com/?q={}",
            "word ".repeat(30)
        )));
        assert_eq!(long.unwrap().chars().count(), MAX_QUERY);
    }

    #[test]
    fn page_names_drop_the_site_around_the_title() {
        for (title, address, name) in [
            (
                "Reddit - The heart of the internet",
                "https://www.reddit.com/",
                "Reddit",
            ),
            (
                "Amazon.fr: books, DVDs, video games",
                "https://www.amazon.fr/",
                "Amazon.fr",
            ),
            ("Tauri 2.0 | Tauri", "https://tauri.app/", "Tauri"),
            (
                "Battle.net Account",
                "https://account.battle.net/?utm_source=app",
                "Battle.net Account",
            ),
            (
                "GitHub - tauri-apps/tauri: Build smaller apps",
                "https://github.com/tauri-apps/tauri",
                "tauri-apps/tauri",
            ),
            (
                "(84) kanjicoder - Twitch",
                "https://www.twitch.tv/kanjicoder",
                "kanjicoder",
            ),
            (
                "Compare · tauri-apps/tauri · GitHub",
                "https://github.com/tauri-apps/tauri/compare",
                "Compare · tauri-apps/tauri",
            ),
            (
                "Rust — Wikipédia",
                "https://fr.wikipedia.org/wiki/Rust",
                "Rust",
            ),
            ("Amazon.fr : lampe", "https://www.amazon.fr/gp/x", "lampe"),
            (
                "Shop | Amazon.co.uk",
                "https://www.amazon.co.uk/shop",
                "Shop",
            ),
            // A trailing name that is not the host's stays: it may be part of the title.
            (
                "Catseye Draught - Item - WoW Forever",
                "https://www.wowhead.com/forever/item=1",
                "Catseye Draught - Item - WoW Forever",
            ),
            (
                "Incendie Paris XIII ? : r/paris",
                "https://www.reddit.com/r/paris/comments/1",
                "Incendie Paris XIII ? : r/paris",
            ),
            ("GitHub", "https://github.com/login", "GitHub"),
            ("  ", "https://www.example.org/a", "example.org"),
            (
                "Notes: day one",
                "http://localhost:8080/notes",
                "Notes: day one",
            ),
        ] {
            assert_eq!(page_name(title, &url(address)), name, "{title}");
        }
    }

    #[test]
    fn trees_are_named_after_a_search_before_the_root_title() {
        let search = visit("https://duckduckgo.com/?q=tauri", "tauri at DuckDuckGo");
        let page = visit("https://tauri.app/", "Tauri 2.0 | Tauri");
        assert_eq!(tree_name(&search, [&page]), "tauri");

        let front = visit("https://www.amazon.fr/", "Amazon.fr: books, DVDs");
        let results = visit("https://www.amazon.fr/s?k=lampe", "Amazon.fr : lampe");
        assert_eq!(tree_name(&front, [&page, &results]), "lampe");
        assert_eq!(tree_name(&front, [&page]), "Amazon.fr");

        // Only a front page takes its name from a later search.
        let repository = visit(
            "https://github.com/tauri-apps/tauri",
            "GitHub - tauri-apps/tauri",
        );
        assert_eq!(tree_name(&repository, [&results]), "tauri-apps/tauri");
    }
}
