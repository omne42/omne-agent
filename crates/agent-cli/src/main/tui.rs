// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

mod tui {
    include!("tui/runtime.rs");
    include!("tui/types.rs");
    include!("tui/ui_state_core.rs");
    include!("tui/ui_state_inline_palette.rs");
    include!("tui/ui_state_inline_execute.rs");
    include!("tui/ui_state_key_handling.rs");
    include!("tui/rpc_outcome.rs");
    include!("tui/overlays.rs");
    include!("tui/tool_format.rs");
    include!("tui/inline_context.rs");
    include!("tui/details.rs");
    include!("tui/key_helpers.rs");
    include!("tui/ui_state_overlays.rs");
    include!("tui/render_base.rs");
    include!("tui/render_thread_list.rs");
    include!("tui/render_thread_view.rs");
    include!("tui/render_overlay.rs");
    include!("tui/tests.rs");
}

async fn run_tui(app: &mut App, args: TuiArgs) -> anyhow::Result<()> {
    tui::run_tui(app, args).await
}
