#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(tag = "method")]
pub enum ServerNotification {
    #[serde(rename = "thread/event")]
    ThreadEvent { params: omne_protocol::ThreadEvent },
    #[serde(rename = "turn/started")]
    TurnStarted { params: omne_protocol::ThreadEvent },
    #[serde(rename = "turn/completed")]
    TurnCompleted { params: omne_protocol::ThreadEvent },
    #[serde(rename = "item/started")]
    ItemStarted { params: omne_protocol::ThreadEvent },
    #[serde(rename = "item/completed")]
    ItemCompleted { params: omne_protocol::ThreadEvent },
}
