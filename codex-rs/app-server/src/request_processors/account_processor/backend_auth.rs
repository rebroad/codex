use super::AccountRequestProcessor;
use super::JSONRPCErrorError;
use super::internal_error;
use codex_http_client::HttpClientFactory;
use codex_login::CodexAuth;

impl AccountRequestProcessor {
    pub(super) async fn backend_auth(
        &self,
    ) -> Result<Option<(CodexAuth, HttpClientFactory)>, JSONRPCErrorError> {
        // Persisted credentials can change between requests. Reload before policy
        // composition so the old account's policy cannot authorize the new one.
        if self.auth_manager.auth().await.is_none() {
            return Ok(None);
        }
        self.config_manager
            .refresh_application_network_policy()
            .await
            .map_err(|err| {
                internal_error(format!("failed to refresh account network policy: {err}"))
            })?;
        // Capture the account-bound factory after publication. A concurrent account
        // change still revokes this factory rather than reusing another account's policy.
        Ok(self.auth_manager.auth_with_http_client_factory().await)
    }
}
