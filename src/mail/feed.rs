//! Gmail's Atom format and the HTTP answers around it.

use super::{Account, AccountNumber, Message, MessageId, Origin, Trouble, SIGN_IN_HOST};
use roxmltree::Node;

/// Gmail's feed lists at most this many unread mails, newest first.
pub(super) const PAGE: usize = 20;

pub(super) struct Answer<'a> {
    pub status: u16,
    pub location: Option<&'a str>,
    pub body: &'a [u8],
}

#[derive(Debug)]
pub(super) enum Outcome {
    Inbox(Account),
    SignedOut,
    Trouble(Trouble),
}

pub(super) fn read(answer: Option<&Answer<'_>>, origin: &Origin, number: AccountNumber) -> Outcome {
    let Some(answer) = answer else {
        return Outcome::Trouble(Trouble::Unreachable);
    };
    match answer.status {
        200 => {
            let account = (answer.body.len() <= 512 * 1024)
                .then(|| std::str::from_utf8(answer.body).ok())
                .flatten()
                .and_then(|xml| parse(xml, origin, number));
            account.map_or(Outcome::Trouble(Trouble::Unreadable), Outcome::Inbox)
        }
        300..=399
            if answer
                .location
                .and_then(|s| origin.feed(number).join(s).ok())
                .is_some_and(|to| leaves_account(origin, number, &to)) =>
        {
            Outcome::SignedOut
        }
        401 => Outcome::SignedOut,
        403 | 408 | 429 | 500..=599 => Outcome::Trouble(Trouble::Unreachable),
        _ => Outcome::Trouble(Trouble::Unreadable),
    }
}

/// Gmail sends a number that is not signed in to Google's sign-in page, or, while another
/// account is signed in, to that account's feed (measured 2026-11-17: `u/3` to `u/0`).
fn leaves_account(origin: &Origin, number: AccountNumber, to: &url::Url) -> bool {
    to.host_str() == Some(SIGN_IN_HOST)
        || (origin.serves(to)
            && to
                .path()
                .strip_prefix("/mail/u/")
                .and_then(|rest| rest.strip_suffix("/feed/atom"))
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| n != number.get()))
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn text<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    child(node, name)?
        .text()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse(xml: &str, origin: &Origin, number: AccountNumber) -> Option<Account> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let feed = document.root_element();
    if feed.tag_name().name() != "feed" {
        return None;
    }
    let address = text(feed, "title")?
        .split_whitespace()
        .rev()
        .find(|s| s.contains('@'))?
        .to_owned();
    let unread = text(feed, "fullcount")?.parse().ok()?;
    let inbox = origin.inbox(&address);
    let messages = feed
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let id = text(entry, "id")?.rsplit(':').next()?.trim();
            if id.is_empty() {
                return None;
            }
            let author = child(entry, "author");
            let from_address = author.and_then(|a| text(a, "email"));
            let from = author
                .and_then(|a| text(a, "name"))
                .or(from_address)
                .unwrap_or("Unknown sender")
                .to_owned();
            let link = entry
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "link")
                .filter_map(|n| n.attribute("href").and_then(|s| url::Url::parse(s).ok()))
                .find(|u| origin.serves(u))
                .unwrap_or_else(|| inbox.clone());
            Some(Message {
                unmarked: None,
                id: MessageId(id.to_owned()),
                from,
                from_address: from_address.map(str::to_owned),
                subject: text(entry, "title").unwrap_or("(no subject)").to_owned(),
                snippet: text(entry, "summary")
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(190)
                    .collect(),
                issued: text(entry, "issued")
                    .and_then(utc_ms)
                    .or_else(|| text(entry, "modified").and_then(utc_ms)),
                link,
            })
        })
        .take(PAGE)
        .collect();
    Some(Account {
        number,
        address,
        unread,
        messages,
        inbox,
    })
}

fn utc_ms(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    if date.len() != 10 {
        return None;
    }
    let date: Vec<i64> = date
        .split('-')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [year, month, day] = date[..] else {
        return None;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month) || !(1..=days[(month - 1) as usize]).contains(&day) {
        return None;
    }
    let (time, fraction) = time.split_once('.').map_or((time, ""), |p| p);
    if !fraction.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<i64> = time
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [hour, minute, second] = parts[..] else {
        return None;
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    let millis = fraction
        .chars()
        .chain(std::iter::repeat('0'))
        .take(3)
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    days.checked_mul(86400000)?
        .checked_add((hour * 3600 + minute * 60 + second) * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FEED: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?><feed version="0.3" xmlns="http://purl.org/atom/ns#">"#,
        r#"<title>Gmail - Inbox for ana@example.test</title><tagline>New messages in your Gmail Inbox</tagline>"#,
        r#"<fullcount>3</fullcount><link rel="alternate" href="https://mail.google.com/mail" type="text/html" />"#,
        r#"<modified>2026-11-17T09:12:33Z</modified>"#,
        r#"<entry><title>Tom &amp; Jerry&#39;s lunch</title><summary>Are you free   on Friday?</summary>"#,
        r#"<link rel="alternate" href="https://mail.google.com/mail/u/0?account_id=ana@example.test&amp;message_id=111&amp;view=conv&amp;extsrc=atom" type="text/html" />"#,
        r#"<modified>2026-11-17T09:12:33Z</modified><issued>2026-11-17T09:12:33Z</issued>"#,
        r#"<id>tag:gmail.google.com,2004:1782938475629384756</id>"#,
        r#"<author><name>Jo Smith</name><email>jo@example.test</email></author></entry>"#,
        r#"<entry><title></title><summary></summary><link rel="alternate" href="https://elsewhere.example/x" type="text/html" />"#,
        r#"<issued>not a time</issued><id>tag:gmail.google.com,2004:1782938475629384757</id>"#,
        r#"<author><email>bo@example.test</email></author></entry>"#,
        "</feed>",
    );

    #[test]
    fn reads_atom_and_falls_back_per_entry() {
        let account = parse(FEED, &Origin::gmail(), AccountNumber(0)).unwrap();
        assert_eq!(account.address, "ana@example.test");
        assert_eq!(account.unread, 3);
        assert_eq!(account.messages.len(), 2);
        let first = &account.messages[0];
        assert_eq!(first.subject, "Tom & Jerry's lunch");
        assert_eq!(first.snippet, "Are you free on Friday?");
        assert_eq!(first.from, "Jo Smith");
        assert_eq!(first.from_address.as_deref(), Some("jo@example.test"));
        assert_eq!(first.id.as_str(), "1782938475629384756");
        assert_eq!(first.issued, Some(1_794_906_753_000));
        assert!(first.link.as_str().contains("message_id=111&view=conv"));
        let second = &account.messages[1];
        assert_eq!(second.subject, "(no subject)");
        assert_eq!(second.from, "bo@example.test");
        assert_eq!(second.from_address.as_deref(), Some("bo@example.test"));
        assert_eq!(second.issued, None);
        assert_eq!(second.link, account.inbox);
    }

    #[test]
    fn requires_count_and_address_but_accepts_empty_inboxes_and_translated_titles() {
        let xml = "<feed><title>Boîte de ana@example.test</title><fullcount>0</fullcount></feed>";
        let a = parse(xml, &Origin::gmail(), AccountNumber(0)).unwrap();
        assert_eq!(a.unread, 0);
        assert!(a.messages.is_empty());
        for bad in [
            xml.replace("<fullcount>0</fullcount>", ""),
            xml.replace(">0<", ">bad<"),
            xml.replace("ana@example.test", "no address"),
            "<html/>".into(),
            "junk".into(),
        ] {
            assert!(
                parse(&bad, &Origin::gmail(), AccountNumber(0)).is_none(),
                "reject {bad}"
            );
        }
        let fallback = xml.replace("</feed>", "<entry><id>x</id></entry></feed>");
        let a = parse(&fallback, &Origin::gmail(), AccountNumber(0)).unwrap();
        assert_eq!(a.messages[0].from, "Unknown sender");
        assert_eq!(a.messages[0].from_address, None);
        assert_eq!(a.messages[0].snippet, "");
    }

    #[test]
    fn answers_follow_the_outcome_table() {
        let origin = Origin::gmail();
        assert!(matches!(
            read(None, &origin, AccountNumber(0)),
            Outcome::Trouble(Trouble::Unreachable)
        ));
        for (status, location, expected) in [
            (200, None, 0),
            (302, Some("https://accounts.google.com/ServiceLogin"), 1),
            (401, None, 1),
            (302, Some("https://mail.google.com/mail/u/1/feed/atom"), 1),
            (302, Some("/mail/u/2/feed/atom"), 1),
            (302, Some("https://mail.google.com/mail/u/0/feed/atom"), 2),
            (302, Some("https://mail.google.com/mail/u/0/"), 2),
            (302, Some("https://elsewhere.test/mail/u/1/feed/atom"), 2),
            (302, Some("https://elsewhere.test/"), 2),
            (302, None, 2),
            (403, None, 3),
            (408, None, 3),
            (429, None, 3),
            (503, None, 3),
            (404, None, 2),
        ] {
            let result = read(
                Some(&Answer {
                    status,
                    location,
                    body: FEED.as_bytes(),
                }),
                &origin,
                AccountNumber(0),
            );
            let kind = match result {
                Outcome::Inbox(_) => 0,
                Outcome::SignedOut => 1,
                Outcome::Trouble(Trouble::Unreadable) => 2,
                Outcome::Trouble(Trouble::Unreachable) => 3,
            };
            assert_eq!(kind, expected, "status {status}, location {location:?}");
        }
        for body in [vec![b' '; 512 * 1024 + 1], vec![255], b"<html/>".to_vec()] {
            assert!(matches!(
                read(
                    Some(&Answer {
                        status: 200,
                        location: None,
                        body: &body
                    }),
                    &origin,
                    AccountNumber(0)
                ),
                Outcome::Trouble(Trouble::Unreadable)
            ));
        }
    }

    #[test]
    fn reads_utc_times_and_rejects_invalid_dates() {
        for (text, expected) in [
            ("2026-11-17T09:12:33Z", 1_794_906_753_000),
            ("2026-11-17T09:12:33.250Z", 1_794_906_753_250),
            ("2000-02-29T23:59:59Z", 951_868_799_000),
            ("1970-01-01T00:00:00Z", 0),
        ] {
            assert_eq!(utc_ms(text), Some(expected), "{text}");
        }
        for text in [
            "",
            "junk",
            "2026-13-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-11-17T25:00:00Z",
        ] {
            assert_eq!(utc_ms(text), None, "{text}");
        }
    }
}
