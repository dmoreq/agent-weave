//! Suspected-secret detection for memory note fields.
//!
//! Best-effort, deterministic, dependency-free: provider prefixes with length
//! and token-boundary rules. It blocks ordinary words such as `task-1` that a
//! bare `sk-` substring check rejected, and it catches common token shapes.
//! It is not an entropy detector; unknown formats can pass.

/// One matched pattern label. The label is internal; callers emit the shared
/// `SUSPECTED_SECRET_MSG` and never echo the matched text.
pub(crate) fn suspected_secret(text: &str) -> Option<&'static str> {
    if has_openai_key(text) {
        return Some("openai-key");
    }
    if has_github_token(text) {
        return Some("github-token");
    }
    if has_aws_access_key(text) {
        return Some("aws-access-key");
    }
    if has_jwt(text) {
        return Some("jwt");
    }
    if has_private_key(text) {
        return Some("private-key");
    }
    if has_slack_token(text) {
        return Some("slack-token");
    }
    if has_bearer_token(text) {
        return Some("bearer-token");
    }
    if has_database_url(text) {
        return Some("database-url");
    }
    None
}

/// Thin wrapper used by every validation call site.
pub(crate) fn contains_suspected_secret(text: &str) -> bool {
    suspected_secret(text).is_some()
}

fn boundary(text: &str, byte_idx: usize) -> bool {
    byte_idx == 0
        || !text[..byte_idx]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric())
}

fn run_len(text: &str, start: usize, allowed: impl Fn(char) -> bool, min: usize) -> bool {
    text[start..].chars().take_while(|c| allowed(*c)).count() >= min
}

fn has_openai_key(text: &str) -> bool {
    text.match_indices("sk-").any(|(idx, _)| {
        boundary(text, idx)
            && run_len(
                text,
                idx + 3,
                |c| c.is_ascii_alphanumeric() || c == '-' || c == '_',
                16,
            )
    })
}

fn has_github_token(text: &str) -> bool {
    const CLASSIC: [&str; 5] = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    for prefix in CLASSIC {
        if text.match_indices(prefix).any(|(idx, _)| {
            boundary(text, idx)
                && run_len(text, idx + prefix.len(), |c| c.is_ascii_alphanumeric(), 20)
        }) {
            return true;
        }
    }
    text.match_indices("github_pat_").any(|(idx, _)| {
        boundary(text, idx)
            && run_len(
                text,
                idx + "github_pat_".len(),
                |c| c.is_ascii_alphanumeric() || c == '_',
                22,
            )
    })
}

fn has_aws_access_key(text: &str) -> bool {
    for prefix in ["AKIA", "ASIA"] {
        for (idx, _) in text.match_indices(prefix) {
            if !boundary(text, idx) {
                continue;
            }
            let rest = &text[idx + prefix.len()..];
            let mut chars = rest.chars();
            let key: String = chars.by_ref().take(16).collect();
            if key.len() != 16
                || !key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                continue;
            }
            let next_is_key = chars
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
            if !next_is_key {
                return true;
            }
        }
    }
    false
}

fn has_jwt(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        if !token.starts_with("eyJ") {
            return false;
        }
        let parts: Vec<&str> = token.split('.').collect();
        parts.len() >= 3
            && parts.iter().all(|p| {
                p.len() >= 10
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
    })
}

fn has_private_key(text: &str) -> bool {
    let Some(begin) = text.find("-----BEGIN ") else {
        return false;
    };
    text[begin..].contains("PRIVATE KEY-----")
}

fn has_slack_token(text: &str) -> bool {
    for prefix in ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"] {
        if text.match_indices(prefix).any(|(idx, _)| {
            boundary(text, idx)
                && run_len(
                    text,
                    idx + prefix.len(),
                    |c| c.is_ascii_alphanumeric() || c == '-',
                    10,
                )
        }) {
            return true;
        }
    }
    false
}

fn has_bearer_token(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("bearer ").any(|(idx, _)| {
        boundary(&lower, idx)
            && run_len(
                &lower,
                idx + "bearer ".len(),
                |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '+' | '/' | '-'),
                20,
            )
    })
}

fn has_database_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for scheme in ["postgres://", "postgresql://", "mysql://", "mongodb://"] {
        for (idx, _) in lower.match_indices(scheme) {
            if !boundary(&lower, idx) {
                continue;
            }
            let rest = &lower[idx + scheme.len()..];
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let Some((userinfo, _host)) = authority.rsplit_once('@') else {
                continue;
            };
            if let Some((user, pass)) = userinfo.split_once(':') {
                if !user.is_empty() && !pass.is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_words() {
        for text in [
            "task-1 pending",
            "risk-averse reviewer",
            "desk-lamp on the shelf",
            "sk-learn pipeline",
            "application.configuration.properties",
            "https://deploy@example.com:8443",
        ] {
            assert_eq!(suspected_secret(text), None, "{text}");
        }
    }

    #[test]
    fn accepts_short_prefixes() {
        for text in ["sk-12345", "ghp_abcdef", "eyJhbGciOi"] {
            assert_eq!(suspected_secret(text), None, "{text}");
        }
    }

    #[test]
    fn rejects_each_pattern() {
        let openai = format!("sk-{}", "a".repeat(24));
        let github = format!("ghp_{}", "a".repeat(36));
        let github_pat = format!("github_pat_{}", "a".repeat(22));
        let aws = format!("AKIA{}", "A".repeat(16));
        let jwt = format!(
            "eyJ{}.eyJ{}.{}",
            "a".repeat(10),
            "b".repeat(10),
            "c".repeat(10)
        );
        let pem = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");
        let slack = format!("xoxb-{}", "1".repeat(12));
        let bearer = format!("Bearer {}", "a".repeat(24));
        let database = "postgres://user:pass@localhost/db";

        for (text, label) in [
            (openai, "openai-key"),
            (github, "github-token"),
            (github_pat, "github-token"),
            (aws, "aws-access-key"),
            (jwt, "jwt"),
            (pem, "private-key"),
            (slack, "slack-token"),
            (bearer, "bearer-token"),
            (database.to_string(), "database-url"),
        ] {
            assert_eq!(suspected_secret(&text), Some(label), "{text}");
        }
    }

    #[test]
    fn boundary_rules() {
        let embedded = format!("task-{}", "a".repeat(24));
        assert_eq!(suspected_secret(&embedded), None);
        let at_boundary = format!("-- sk-{}", "a".repeat(24));
        assert_eq!(suspected_secret(&at_boundary), Some("openai-key"));
    }

    #[test]
    fn database_url_requires_a_password() {
        assert_eq!(suspected_secret("postgres://user:@host/db"), None);
        assert_eq!(
            suspected_secret("mysql://user:pass@host/db"),
            Some("database-url")
        );
    }

    #[test]
    fn bearer_is_case_insensitive() {
        let text = format!("bearer {}", "a".repeat(24));
        assert_eq!(suspected_secret(&text), Some("bearer-token"));
    }
}
