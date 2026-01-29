#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(tag = "method")]
pub enum ServerNotification {
    #[serde(rename = "thread/event")]
    ThreadEvent { params: pm_protocol::ThreadEvent },
    #[serde(rename = "turn/started")]
    TurnStarted { params: pm_protocol::ThreadEvent },
    #[serde(rename = "turn/completed")]
    TurnCompleted { params: pm_protocol::ThreadEvent },
    #[serde(rename = "item/started")]
    ItemStarted { params: pm_protocol::ThreadEvent },
    #[serde(rename = "item/completed")]
    ItemCompleted { params: pm_protocol::ThreadEvent },
}
