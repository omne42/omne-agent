use omne_notify_env::NotifyEnvOptions as OmneNotifyHubOptions;

fn build_omne_notify_hub_from_env(
    options: OmneNotifyHubOptions,
) -> anyhow::Result<Option<notify_kit::Hub>> {
    omne_notify_env::build_notify_hub_from_env(options)
}

#[cfg(test)]
fn build_omne_notify_hub_from_reader<F>(
    options: OmneNotifyHubOptions,
    get: &F,
) -> anyhow::Result<Option<notify_kit::Hub>>
where
    F: Fn(&str) -> Option<String>,
{
    omne_notify_env::build_notify_hub_from_reader(get, options)
}

#[cfg(test)]
mod notify_integration_tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn build_omne_notify_hub_from_reader_supports_sound_only() {
        let env = HashMap::from([(String::from("OMNE_NOTIFY_SOUND"), String::from("1"))]);
        let hub = build_omne_notify_hub_from_reader(
            OmneNotifyHubOptions {
                default_sound_enabled: false,
                require_sink: true,
            },
            &|key| env.get(key).cloned(),
        )
        .expect("build hub")
        .expect("hub present");

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
    fn build_omne_notify_hub_from_reader_errors_when_sink_required() {
        let env = HashMap::<String, String>::new();
        let err = match build_omne_notify_hub_from_reader(
            OmneNotifyHubOptions {
                default_sound_enabled: false,
                require_sink: true,
            },
            &|key| env.get(key).cloned(),
        ) {
            Ok(_) => panic!("expected error"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("no notification sinks configured"));
    }
}
