//! Startup-only email allowlist. Addresses authorize a grant after provider token
//! verification, or a matching development-login subject; they never merge accounts.
use std::ffi::OsString;

pub const HELP: &str = "Usage: pinvoyager-api [--initial-admin-email EMAIL]...\n\nRepeat --initial-admin-email for each initial administrator.\nProvider sign-in requires eligible, verified Google or Apple ID-token emails.\nDevelopment login matches the supplied subject without email verification.\nRemove these arguments after bootstrap to stop future automatic grants.\n";

#[derive(Default)]
pub struct InitialAdmins {
    emails: Vec<String>,
}

impl std::fmt::Debug for InitialAdmins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitialAdmins")
            .field("count", &self.emails.len())
            .finish()
    }
}

impl InitialAdmins {
    /// Parse repeatable --initial-admin-email EMAIL or --initial-admin-email=EMAIL.
    /// Returns None for help. Unknown flags, non-Unicode values, and malformed
    /// addresses fail before startup; error messages do not echo the arguments.
    pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Self>, String> {
        let invalid = || {
            "Expected --initial-admin-email EMAIL (repeatable), or --help; use a nonempty ASCII email address.".to_owned()
        };
        let mut args = args.into_iter();
        let mut emails = Vec::new();
        while let Some(arg) = args.next() {
            let arg = arg.into_string().map_err(|_| invalid())?;
            if arg == "--help" || arg == "-h" {
                return Ok(None);
            }
            let email = if arg == "--initial-admin-email" {
                args.next()
                    .ok_or_else(invalid)?
                    .into_string()
                    .map_err(|_| invalid())?
            } else if let Some(email) = arg.strip_prefix("--initial-admin-email=") {
                email.to_owned()
            } else {
                return Err(invalid());
            };
            let email = normalize(&email).ok_or_else(invalid)?;
            if !emails.contains(&email) {
                emails.push(email);
            }
        }
        Ok(Some(Self { emails }))
    }

    pub fn is_empty(&self) -> bool {
        self.emails.is_empty()
    }

    /// Ignore domain case, preserving local-part case, dots, and plus suffixes. Callers
    /// must verify token/email ownership first, except for the development-login bypass.
    pub fn matches(&self, email: &str) -> bool {
        normalize(email).is_some_and(|email| self.emails.contains(&email))
    }
}

fn normalize(email: &str) -> Option<String> {
    let email = email.trim();
    let (local, domain) = email.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !email.is_ascii()
        || email
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return None;
    }
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}
#[cfg(test)]
#[path = "initial_admin_tests.rs"]
mod tests;
