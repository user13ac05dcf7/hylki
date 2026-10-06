//! The People view: mail grouped by the person it was exchanged with, the
//! way a messenger lists conversations. Incoming mail belongs to its sender;
//! mail the user sent belongs to each of its recipients (To and Cc). Being
//! on Cc of someone else's mail does not put it in that person's view, and
//! the user's own addresses never make a person.

use std::collections::{HashMap, HashSet};

/// One person, as the People list shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// Lower-case address: the person's key.
    pub address: String,
    /// The best display name seen, or empty.
    pub name: String,
    /// Newest exchange (Unix seconds).
    pub latest: i64,
    /// Unread mail from them.
    pub unread: u32,
    /// Messages exchanged.
    pub total: u32,
}

impl Person {
    /// The name, or the address's local part when there is none.
    pub fn display_name(&self) -> &str {
        if !self.name.is_empty() {
            return &self.name;
        }
        match self.address.split_once('@') {
            Some((local, _)) if !local.is_empty() => local,
            _ => &self.address,
        }
    }
}

/// The header fields the People view needs of a cached message.
#[derive(Debug, Clone, Default)]
pub struct Header {
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub timestamp: i64,
    pub unread: bool,
    /// Message-ID, to count a message filed in several folders once.
    pub message_id: String,
}

impl From<&crate::models::Message> for Header {
    fn from(m: &crate::models::Message) -> Header {
        Header {
            from_name: m.from_name.clone(),
            from_addr: m.from_addr.clone(),
            to: m.to.clone(),
            cc: m.cc.clone(),
            timestamp: m.timestamp,
            unread: m.unread,
            message_id: m.message_id.clone(),
        }
    }
}

/// The user's own addresses (accounts and send-as aliases), lower case.
#[derive(Debug, Clone, Default)]
pub struct Own(HashSet<String>);

impl Own {
    pub fn new<I: IntoIterator<Item = S>, S: AsRef<str>>(addresses: I) -> Own {
        Own(addresses
            .into_iter()
            .map(|a| a.as_ref().trim().to_lowercase())
            .filter(|a| !a.is_empty())
            .collect())
    }

    pub fn contains(&self, address: &str) -> bool {
        self.0.contains(&address.trim().to_lowercase())
    }
}

/// The (name, lower-case address) pairs of a recipient field.
fn recipients(field: &str) -> impl Iterator<Item = (String, String)> + '_ {
    crate::worker::parse_recipients(field)
        .into_iter()
        .map(|(name, addr)| (name, addr.trim().to_lowercase()))
        .filter(|(_, addr)| addr.contains('@'))
}

/// The people a message belongs to, with the name it gives each: its sender
/// for incoming mail, every recipient but the user for mail the user sent.
pub fn counterparts(h: &Header, own: &Own) -> Vec<(String, String)> {
    let from = h.from_addr.trim().to_lowercase();
    let mut out: Vec<(String, String)> = Vec::new();
    if own.contains(&from) {
        for (name, addr) in recipients(&h.to).chain(recipients(&h.cc)) {
            if !own.contains(&addr) && !out.iter().any(|(a, _)| *a == addr) {
                out.push((addr, name));
            }
        }
    } else if from.contains('@') {
        out.push((from, h.from_name.trim().to_string()));
    }
    out
}

/// Whether a message belongs to `address`'s view.
pub fn involves(h: &Header, own: &Own, address: &str) -> bool {
    counterparts(h, own).iter().any(|(a, _)| a == address)
}

/// A name worth showing: not empty and not just an address.
fn usable_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('@')
}

/// Everyone the headers were exchanged with, most recent first. A name
/// the person gives themselves (as sender) wins over one the user typed.
pub fn people(headers: &[Header], own: &Own) -> Vec<Person> {
    let mut by_addr: HashMap<String, (Person, bool)> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for h in headers {
        if !h.message_id.is_empty() && !seen.insert(h.message_id.as_str()) {
            continue;
        }
        let incoming = !own.contains(&h.from_addr);
        for (addr, name) in counterparts(h, own) {
            let (p, own_name) = by_addr.entry(addr.clone()).or_insert_with(|| {
                (Person { address: addr, name: String::new(), latest: 0, unread: 0, total: 0 }, false)
            });
            p.total += 1;
            if incoming && h.unread {
                p.unread += 1;
            }
            if usable_name(&name) && (p.name.is_empty() || (incoming && !*own_name)) {
                p.name = name;
                *own_name = incoming;
            }
            p.latest = p.latest.max(h.timestamp);
        }
    }
    let mut out: Vec<Person> = by_addr.into_values().map(|(p, _)| p).collect();
    out.sort_by(|a, b| b.latest.cmp(&a.latest).then_with(|| a.address.cmp(&b.address)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(from: &str, to: &str, cc: &str, ts: i64, unread: bool, id: &str) -> Header {
        let (from_name, from_addr) = crate::config::split_identity(from);
        Header {
            from_name,
            from_addr,
            to: to.into(),
            cc: cc.into(),
            timestamp: ts,
            unread,
            message_id: id.into(),
        }
    }

    fn own() -> Own {
        Own::new(["me@example.com", "Alias@Example.org"])
    }

    #[test]
    fn incoming_mail_belongs_to_its_sender_only() {
        let m = h("Ada <Ada@x.com>", "me@example.com", "bob@x.com", 1, false, "a");
        assert_eq!(counterparts(&m, &own()), vec![("ada@x.com".into(), "Ada".into())]);
        // Bob was only on Cc of Ada's mail: not his conversation.
        assert!(!involves(&m, &own(), "bob@x.com"));
    }

    #[test]
    fn sent_mail_belongs_to_every_recipient_but_me() {
        let m = h(
            "Me <me@example.com>",
            "Ada <ada@x.com>, alias@example.org",
            "\"Smith, Bob\" <bob@x.com>",
            1,
            false,
            "a",
        );
        let c = counterparts(&m, &own());
        assert_eq!(c, vec![
            ("ada@x.com".into(), "Ada".into()),
            ("bob@x.com".into(), "Smith, Bob".into()),
        ]);
    }

    #[test]
    fn mail_from_an_alias_counts_as_sent() {
        let m = h("alias@example.org", "carol@x.com", "", 1, false, "a");
        assert!(involves(&m, &own(), "carol@x.com"));
        assert!(!involves(&m, &own(), "alias@example.org"));
    }

    #[test]
    fn people_sorted_by_latest_with_unread_and_names() {
        let headers = vec![
            h("Ada <ada@x.com>", "me@example.com", "", 10, true, "1"),
            h("me@example.com", "Ada L. <ada@x.com>", "", 30, false, "2"),
            h("Bob <bob@x.com>", "me@example.com", "", 20, true, "3"),
            // The same message filed twice (Gmail labels) counts once.
            h("Bob <bob@x.com>", "me@example.com", "", 20, true, "3"),
            h("me@example.com", "dave@x.com", "", 5, false, "4"),
        ];
        let p = people(&headers, &own());
        let addrs: Vec<&str> = p.iter().map(|p| p.address.as_str()).collect();
        assert_eq!(addrs, ["ada@x.com", "bob@x.com", "dave@x.com"]);
        assert_eq!((p[0].total, p[0].unread), (2, 1));
        // Ada's own name wins over the one typed when writing to her.
        assert_eq!(p[0].name, "Ada");
        assert_eq!((p[1].total, p[1].unread), (1, 1));
        assert_eq!(p[2].display_name(), "dave");
    }

    #[test]
    fn unread_mail_i_sent_is_not_counted_as_unread() {
        let headers = vec![h("me@example.com", "ada@x.com", "", 1, true, "1")];
        assert_eq!(people(&headers, &own())[0].unread, 0);
    }
}
