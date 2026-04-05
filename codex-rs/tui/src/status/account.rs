#[derive(Debug, Clone)]
pub(crate) enum StatusAccountDisplay {
    ChatGpt {
        email_prefix_emoji: Option<String>,
        email: Option<String>,
        plan: Option<String>,
    },
    ApiKey,
}
