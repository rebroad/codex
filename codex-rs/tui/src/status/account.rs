#[derive(Debug, Clone)]
pub(crate) enum StatusAccountDisplay {
    ChatGpt {
        email_prefix_emoji: Option<String>,
        email: Option<String>,
        plan: Option<String>,
    },
    ApiKey,
}

pub(crate) fn truncate_status_email_local_part(email: Option<String>) -> Option<String> {
    email.map(|email| {
        let Some((local_part, domain)) = email.split_once('@') else {
            return email;
        };
        let truncated_local_part: String = local_part
            .chars()
            .rev()
            .take(14)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{truncated_local_part}@{domain}")
    })
}

#[cfg(test)]
mod tests {
    use super::truncate_status_email_local_part;
    use pretty_assertions::assert_eq;

    #[test]
    fn truncates_only_local_part_to_last_14_chars() {
        let email = "abcdefghijklmnop@example.com".to_string();
        let truncated = truncate_status_email_local_part(Some(email));
        assert_eq!(truncated, Some("cdefghijklmnop@example.com".to_string()));
    }

    #[test]
    fn keeps_domain_unchanged() {
        let email = "x@sub.domain.example.com".to_string();
        let truncated = truncate_status_email_local_part(Some(email));
        assert_eq!(truncated, Some("x@sub.domain.example.com".to_string()));
    }
}
