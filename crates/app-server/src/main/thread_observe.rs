#[path = "thread_observe/attention_and_subscribe.rs"]
mod attention_and_subscribe;
#[path = "thread_observe/disk_git_diff.rs"]
mod disk_git_diff;

pub(crate) use attention_and_subscribe::{
    handle_thread_attention, handle_thread_list_meta, handle_thread_subscribe,
    looks_like_test_command, maybe_write_stuck_report, parse_thread_approval_action_id,
    process_command_label, summarize_pending_approval, ThreadObservationCacheEntry,
};
pub(crate) use disk_git_diff::{
    handle_thread_diff, handle_thread_disk_report, handle_thread_disk_usage, handle_thread_patch,
    maybe_emit_thread_disk_warning,
};

#[cfg(test)]
pub(crate) use disk_git_diff::{handle_thread_git_snapshot, ThreadGitSnapshotSpec};
