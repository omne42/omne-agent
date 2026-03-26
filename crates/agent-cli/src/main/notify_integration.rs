fn watch_inbox_notify_env_options() -> omne_notify_env::NotifyEnvOptions {
    omne_notify_env::NotifyEnvOptions {
        default_sound_enabled: true,
        require_sink: true,
    }
}

fn build_watch_inbox_notify_hub_from_env() -> anyhow::Result<notify_kit::Hub> {
    omne_notify_env::build_notify_hub_from_env(watch_inbox_notify_env_options())?
        .context("expected notification hub when require_sink=true")
}

#[cfg(test)]
fn build_watch_inbox_notify_hub_from_reader<F>(get: &F) -> anyhow::Result<notify_kit::Hub>
where
    F: Fn(&str) -> Option<String>,
{
    omne_notify_env::build_notify_hub_from_reader(get, watch_inbox_notify_env_options())?
        .context("expected notification hub when require_sink=true")
}

#[cfg(test)]
mod notify_integration_tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn notify_env_reader_uses_default_sound_for_watch_inbox() {
        let env = HashMap::<String, String>::new();

        let hub = build_watch_inbox_notify_hub_from_reader(&|key| env.get(key).cloned())
            .expect("build hub");

        assert_eq!(
            hub.try_notify(notify_kit::Event::new(
                "attention_state",
                notify_kit::Severity::Info,
                "title",
            )),
            Err(notify_kit::TryNotifyError::NoTokioRuntime)
        );
    }

    #[test]
    fn notify_env_reader_errors_when_require_sink_and_sound_disabled() {
        let env = HashMap::from([(String::from("OMNE_NOTIFY_SOUND"), String::from("0"))]);

        let result = build_watch_inbox_notify_hub_from_reader(&|key| env.get(key).cloned());
        let err = match result {
            Ok(_) => panic!("expected missing sink error"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("no notification sinks configured"),
            "{err:#}"
        );
    }

    #[test]
    fn notify_env_reader_preserves_enabled_kinds_filter() {
        let env = HashMap::from([(
            String::from("OMNE_NOTIFY_EVENTS"),
            String::from("attention_state"),
        )]);

        let hub = build_watch_inbox_notify_hub_from_reader(&|key| env.get(key).cloned())
            .expect("build hub");

        assert_eq!(
            hub.try_notify(notify_kit::Event::new(
                "other_kind",
                notify_kit::Severity::Info,
                "title",
            )),
            Ok(())
        );
    }
}
