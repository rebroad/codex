use super::turn_settings_update_error_message;

#[test]
fn turn_settings_update_error_shows_wrapped_server_rejection() {
    let error =
        color_eyre::eyre::eyre!("turn settings updates require the step_model_switching feature")
            .wrap_err("turn/settings/update failed in TUI")
            .wrap_err("turn/settings/update failed");

    insta::assert_snapshot!(turn_settings_update_error_message(&error));
}
