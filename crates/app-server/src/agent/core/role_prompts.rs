const ROLE_PROMPT_ARCHITECT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/architect.md"
));
const ROLE_PROMPT_BUILDER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/builder.md"
));
const ROLE_PROMPT_CODER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/coder.md"
));
const ROLE_PROMPT_DEBUGGER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/debugger.md"
));
const ROLE_PROMPT_DESIGNER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/designer.md"
));
const ROLE_PROMPT_IDEATOR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/ideator.md"
));
const ROLE_PROMPT_LIBRARIAN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/librarian.md"
));
const ROLE_PROMPT_MERGER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/merger.md"
));
const ROLE_PROMPT_ORCHESTRATOR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/orchestrator.md"
));
const ROLE_PROMPT_REVIEWER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/reviewer.md"
));
const ROLE_PROMPT_SECURITY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/security.md"
));
const ROLE_PROMPT_SKEPTIC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/skeptic.md"
));

fn role_prompt_for_mode(mode: &str) -> Option<&'static str> {
    match mode {
        "architect" => Some(ROLE_PROMPT_ARCHITECT),
        "builder" => Some(ROLE_PROMPT_BUILDER),
        "coder" => Some(ROLE_PROMPT_CODER),
        "debugger" => Some(ROLE_PROMPT_DEBUGGER),
        "designer" => Some(ROLE_PROMPT_DESIGNER),
        "ideator" => Some(ROLE_PROMPT_IDEATOR),
        "librarian" => Some(ROLE_PROMPT_LIBRARIAN),
        "merger" => Some(ROLE_PROMPT_MERGER),
        "orchestrator" => Some(ROLE_PROMPT_ORCHESTRATOR),
        "reviewer" => Some(ROLE_PROMPT_REVIEWER),
        "security" => Some(ROLE_PROMPT_SECURITY),
        "skeptic" => Some(ROLE_PROMPT_SKEPTIC),
        _ => None,
    }
}

