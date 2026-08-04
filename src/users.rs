// SPDX-License-Identifier: GPL-3.0-or-later
//! Who the login screen offers.
//!
//! The list is the humans on the machine: `root`, plus the accounts in the range an
//! administrator set aside for people. That range comes from `/etc/login.defs`, the same
//! file `useradd` reads, so the greeter shows exactly the accounts the admin created and
//! nothing meant for the system's own use.
//!
//! Read from `/etc/passwd`, which is world-readable. Nothing here needs privilege, and
//! nothing here is a security boundary -- greetd decides who may actually log in. A user
//! typed by hand into the field works whether or not the list mentions them, so a system
//! with its accounts in LDAP rather than `/etc/passwd` still logs in; the list is just
//! empty.

// `list`/`list_from` read the real files; the UI reaches for them in a later stage.
#![allow(dead_code)]

use std::path::Path;

/// A person the login screen can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The login name, as typed into the field.
    pub name: String,
    /// The display name, if the account has one. `root` is shown as `Super-User`, the
    /// name IRIX gave it, in place of an empty one.
    pub full_name: Option<String>,
    pub uid: u32,
}

/// The UID range humans live in when `/etc/login.defs` says nothing.
///
/// The same defaults shadow-utils compiles in, so a machine without the setting still
/// lists what one with it would.
const DEFAULT_UID_MIN: u32 = 1000;
const DEFAULT_UID_MAX: u32 = 60000;

/// What IRIX displayed for the otherwise-nameless super-user.
const SUPER_USER: &str = "Super-User";

/// The users to offer, read from the usual files.
///
/// Missing or unreadable files yield an empty list rather than an error: a greeter with
/// no icons still lets a name be typed, which is the right outcome on an LDAP machine or
/// one where `/etc/passwd` cannot be read.
pub fn list() -> Vec<User> {
    list_from(Path::new("/etc/passwd"), Path::new("/etc/login.defs"))
}

fn list_from(passwd: &Path, login_defs: &Path) -> Vec<User> {
    let (uid_min, uid_max) = std::fs::read_to_string(login_defs)
        .map(|text| parse_uid_range(&text))
        .unwrap_or((DEFAULT_UID_MIN, DEFAULT_UID_MAX));

    std::fs::read_to_string(passwd)
        .map(|text| parse_passwd(&text, uid_min, uid_max))
        .unwrap_or_default()
}

/// The humans in a `passwd` file: uid 0, plus `uid_min..=uid_max`.
///
/// Accounts whose shell disables login (`nologin`, `/bin/false`) are left out -- they
/// exist to own files or run services, not to sit at a screen. Order is the file's, which
/// puts `root` first and is what a fixed system list should look like.
pub fn parse_passwd(text: &str, uid_min: u32, uid_max: u32) -> Vec<User> {
    text.lines()
        .filter_map(parse_entry)
        .filter(|entry| {
            (entry.uid == 0 || (entry.uid >= uid_min && entry.uid <= uid_max))
                && !shell_forbids_login(&entry.shell)
        })
        .map(|entry| User {
            full_name: full_name(&entry.gecos, entry.uid),
            name: entry.name,
            uid: entry.uid,
        })
        .collect()
}

/// The fields of a `passwd` line the greeter cares about.
struct Entry {
    name: String,
    uid: u32,
    gecos: String,
    shell: String,
}

/// One `passwd` line.
///
/// Anything malformed -- too few fields, a uid that will not parse -- is skipped, since a
/// broken line is not a reason to refuse the whole login screen.
fn parse_entry(line: &str) -> Option<Entry> {
    // name:passwd:uid:gid:gecos:home:shell
    let mut fields = line.split(':');
    let name = fields.next()?;
    let _passwd = fields.next()?;
    let uid: u32 = fields.next()?.parse().ok()?;
    let _gid = fields.next()?;
    let gecos = fields.next()?;
    let _home = fields.next()?;
    let shell = fields.next().unwrap_or("");

    if name.is_empty() {
        return None;
    }
    Some(Entry {
        name: name.to_string(),
        uid,
        gecos: gecos.to_string(),
        shell: shell.to_string(),
    })
}

/// The account's display name, from GECOS.
///
/// The first comma-separated part -- the rest is the office and phone fields nobody
/// fills in. Empty GECOS gives no name, except for the super-user, who is shown as
/// `Super-User` the way IRIX did rather than as a bare `root`.
fn full_name(gecos: &str, uid: u32) -> Option<String> {
    let name = gecos.split(',').next().unwrap_or("").trim();
    if !name.is_empty() {
        Some(name.to_string())
    } else if uid == 0 {
        Some(SUPER_USER.to_string())
    } else {
        None
    }
}

/// Whether a login shell means "no interactive login".
///
/// The conventional markers: any `nologin` (`/usr/sbin/nologin`, `/sbin/nologin`) and
/// `false`. An empty shell is *not* excluded -- historically that means `/bin/sh`.
fn shell_forbids_login(shell: &str) -> bool {
    shell.ends_with("nologin") || shell.ends_with("/false") || shell == "false"
}

/// `UID_MIN` and `UID_MAX` from a `login.defs`, defaulting each independently.
///
/// The format is `KEY<whitespace>VALUE`, with `#` comments. An absent or unparseable key
/// falls back, so a file that sets only one of the two still works.
pub fn parse_uid_range(text: &str) -> (u32, u32) {
    let mut min = DEFAULT_UID_MIN;
    let mut max = DEFAULT_UID_MAX;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("UID_MIN"), Some(value)) => {
                if let Ok(v) = value.parse() {
                    min = v;
                }
            }
            (Some("UID_MAX"), Some(value)) => {
                if let Ok(v) = value.parse() {
                    max = v;
                }
            }
            _ => {}
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A passwd covering the cases that matter: root with no GECOS, a human, a system
    /// account below the range, a nologin service, `nobody` above the range, and a
    /// human whose GECOS carries the office/phone fields.
    const PASSWD: &str = "\
root:x:0:0::/root:/usr/bin/bash
bin:x:1:1::/:/usr/bin/nologin
daemon:x:2:2::/:/usr/bin/nologin
mail:x:8:12::/var/spool/mail:/usr/bin/nologin
ftp:x:14:11::/srv/ftp:/usr/bin/nologin
sysbuild:x:500:500:System Build:/var/empty:/bin/bash
vic485:x:1000:1000:Victor Fugazzotto:/home/vic485:/bin/fish
ada:x:1001:1001:Ada Lovelace,Room 1,555-0100,,:/home/ada:/bin/bash
noshell:x:1002:1002:No Shell User:/home/noshell:/bin/false
nobody:x:65534:65534:Nobody:/:/usr/bin/nologin
";

    fn names(users: &[User]) -> Vec<&str> {
        users.iter().map(|u| u.name.as_str()).collect()
    }

    #[test]
    fn offers_root_and_humans_only() {
        let users = parse_passwd(PASSWD, 1000, 60000);
        // No nologin services (bin/daemon/mail/ftp/nobody), no below-range system
        // account (sysbuild, uid 500), no login-disabled human (noshell).
        assert_eq!(names(&users), ["root", "vic485", "ada"]);
    }

    #[test]
    fn root_comes_first_regardless_of_range() {
        // uid 0 is included by identity, not by the range, and keeps the file's order.
        let users = parse_passwd(PASSWD, 1000, 60000);
        assert_eq!(users[0].name, "root");
        assert_eq!(users[0].uid, 0);
    }

    #[test]
    fn empty_gecos_root_shows_super_user() {
        let users = parse_passwd(PASSWD, 1000, 60000);
        let root = &users[0];
        assert_eq!(root.full_name.as_deref(), Some("Super-User"));
    }

    #[test]
    fn full_name_is_the_first_gecos_field() {
        let users = parse_passwd(PASSWD, 1000, 60000);
        let ada = users.iter().find(|u| u.name == "ada").unwrap();
        // The office and phone fields after the first comma are dropped.
        assert_eq!(ada.full_name.as_deref(), Some("Ada Lovelace"));
    }

    #[test]
    fn a_human_without_gecos_has_no_full_name() {
        let passwd = "vic:x:1000:1000::/home/vic:/bin/bash\n";
        let users = parse_passwd(passwd, 1000, 60000);
        assert_eq!(users[0].full_name, None);
    }

    #[test]
    fn the_range_is_honored() {
        // Widen the range and the below-range system account appears; narrow it and the
        // humans drop out. root stays either way.
        let wide = parse_passwd(PASSWD, 500, 60000);
        assert!(names(&wide).contains(&"sysbuild"));

        let narrow = parse_passwd(PASSWD, 2000, 3000);
        assert_eq!(names(&narrow), ["root"]);
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let passwd = "\
root:x:0:0::/root:/bin/bash
this is not a passwd line
half:x:notanumber:1000::/home/half:/bin/bash
vic:x:1000:1000:Vic:/home/vic:/bin/bash
";
        let users = parse_passwd(passwd, 1000, 60000);
        assert_eq!(names(&users), ["root", "vic"]);
    }

    #[test]
    fn empty_or_missing_yields_no_users() {
        assert!(parse_passwd("", 1000, 60000).is_empty());
        assert!(parse_passwd("\n\n", 1000, 60000).is_empty());
    }

    #[test]
    fn nologin_variants_are_all_excluded() {
        for shell in ["/usr/sbin/nologin", "/sbin/nologin", "/bin/false", "false"] {
            let passwd = format!("svc:x:1000:1000:Service:/:{shell}\n");
            assert!(
                parse_passwd(&passwd, 1000, 60000).is_empty(),
                "shell {shell} should exclude the account"
            );
        }
        // An empty shell is historically /bin/sh, so it is a real login.
        let passwd = "u:x:1000:1000:U:/home/u:\n";
        assert_eq!(parse_passwd(passwd, 1000, 60000).len(), 1);
    }

    #[test]
    fn uid_range_parses_and_defaults_each_key_alone() {
        let (min, max) = parse_uid_range("UID_MIN 1000\nUID_MAX 60000\n");
        assert_eq!((min, max), (1000, 60000));

        // Comments and odd whitespace.
        let (min, max) = parse_uid_range("# people\nUID_MIN\t\t2000  # start\nUID_MAX 50000\n");
        assert_eq!((min, max), (2000, 50000));

        // Only one key set: the other keeps its default.
        let (min, max) = parse_uid_range("UID_MIN 5000\n");
        assert_eq!((min, max), (5000, DEFAULT_UID_MAX));

        // Nothing usable: both default.
        assert_eq!(
            parse_uid_range("nonsense\n"),
            (DEFAULT_UID_MIN, DEFAULT_UID_MAX)
        );
    }
}
