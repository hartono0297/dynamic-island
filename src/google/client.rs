//! Which OAuth client a Google login uses: the user's own (Settings →
//! Integrasi → Lanjutan) when one is set, else the one built into this copy
//! of Vale, else none.
//!
//! The built-in client is compiled in from `VALE_GOOGLE_CLIENT_ID` and
//! `VALE_GOOGLE_CLIENT_SECRET` at build time (a "Desktop app" client, whose
//! secret Google does not treat as secret). A build without them has no
//! built-in client, and Settings shows only the own-client fields.

use crate::config::Config;

/// The built-in client: ID and secret, empty when the build set none.
pub const BUILT_IN: (&str, &str) = (
    or_empty(option_env!("VALE_GOOGLE_CLIENT_ID")),
    or_empty(option_env!("VALE_GOOGLE_CLIENT_SECRET")),
);

const fn or_empty(value: Option<&'static str>) -> &'static str {
    match value {
        Some(v) => v,
        None => "",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub secret: String,
    /// The user's own client, not the built-in one.
    pub own: bool,
}

/// The client to use: `own` (ID, secret) when its ID is set, else `built_in`
/// when its ID is set, else none. Both are trimmed.
pub fn choose(own: (&str, &str), built_in: (&str, &str)) -> Option<Client> {
    [(own, true), (built_in, false)]
        .into_iter()
        .find(|((id, _), _)| !id.trim().is_empty())
        .map(|((id, secret), own)| Client {
            id: id.trim().into(),
            secret: secret.trim().into(),
            own,
        })
}

/// The client `cfg` and this build give.
pub fn for_config(cfg: &Config) -> Option<Client> {
    choose((&cfg.google_client_id, &cfg.google_client_secret), BUILT_IN)
}

/// This build has a built-in client: Settings offers "Masuk dengan Google".
pub fn has_built_in() -> bool {
    !BUILT_IN.0.trim().is_empty()
}

/// Tokens whose stored client ID is `issued_by` came from `client`. Tokens
/// saved before Vale recorded the client (an empty ID) came from the user's
/// own client, the only kind there was.
pub fn issued_to(issued_by: &str, client: &Client) -> bool {
    if issued_by.is_empty() {
        client.own
    } else {
        issued_by == client.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILT: (&str, &str) = ("vale.apps.googleusercontent.com", "vale-secret");

    fn client(id: &str, secret: &str, own: bool) -> Client {
        Client {
            id: id.into(),
            secret: secret.into(),
            own,
        }
    }

    #[test]
    fn own_client_wins_then_built_in_then_none() {
        // Own ID set: it is used, whatever the build carries.
        assert_eq!(
            choose((" mine.apps.googleusercontent.com ", " s "), BUILT),
            Some(client("mine.apps.googleusercontent.com", "s", true))
        );
        assert_eq!(
            choose(("mine.apps.googleusercontent.com", "s"), ("", "")),
            Some(client("mine.apps.googleusercontent.com", "s", true))
        );
        // No own ID: the built-in client, even with a stray own secret.
        assert_eq!(
            choose(("  ", "left-over"), BUILT),
            Some(client(BUILT.0, BUILT.1, false))
        );
        // Neither: not configured.
        assert_eq!(choose(("", ""), ("", "")), None);
        assert_eq!(choose(("", ""), (" ", "secret")), None);
    }

    #[test]
    fn tokens_belong_to_the_client_that_issued_them() {
        let own = client("mine", "s", true);
        let built = client("vale", "v", false);
        assert!(issued_to("mine", &own));
        assert!(issued_to("vale", &built));
        // Switching between the two asks for a new login.
        assert!(!issued_to("vale", &own));
        assert!(!issued_to("mine", &built));
        // Tokens from before the client was recorded came from an own client.
        assert!(issued_to("", &own));
        assert!(!issued_to("", &built));
    }
}
