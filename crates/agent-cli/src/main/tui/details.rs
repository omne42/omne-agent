    fn build_process_inspect_text(resp: &ProcessInspectResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("process_id: {}\n", resp.process.process_id));
        out.push_str(&format!("thread_id: {}\n", resp.process.thread_id));
        out.push_str(&format!(
            "status: {}\n",
            process_status_str(resp.process.status)
        ));
        if let Some(turn_id) = resp.process.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str(&format!("started_at: {}\n", resp.process.started_at));
        out.push_str(&format!("last_update_at: {}\n", resp.process.last_update_at));
        if let Some(exit_code) = resp.process.exit_code {
            out.push_str(&format!("exit_code: {exit_code}\n"));
        }
        out.push_str(&format!("cwd: {}\n", resp.process.cwd));
        out.push_str(&format!("argv: {}\n", resp.process.argv.join(" ")));
        out.push_str(&format!("stdout_path: {}\n", resp.process.stdout_path));
        out.push_str(&format!("stderr_path: {}\n", resp.process.stderr_path));

        out.push_str("\n# stdout\n\n");
        out.push_str(resp.stdout_tail.trim_end());
        out.push_str("\n\n# stderr\n\n");
        out.push_str(resp.stderr_tail.trim_end());
        out
    }

    fn build_artifact_read_text(resp: &ArtifactReadResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("artifact_id: {}\n", resp.metadata.artifact_id));
        out.push_str(&format!("artifact_type: {}\n", resp.metadata.artifact_type));
        out.push_str(&format!("summary: {}\n", resp.metadata.summary));
        out.push_str(&format!("version: {}\n", resp.metadata.version));
        out.push_str(&format!("bytes: {}\n", resp.bytes));
        out.push_str(&format!("truncated: {}\n", resp.truncated));
        out.push_str("\n# Content\n\n");
        out.push_str(resp.text.trim_end());
        out
    }

    fn process_status_str(value: ProcessStatus) -> &'static str {
        match value {
            ProcessStatus::Running => "running",
            ProcessStatus::Exited => "exited",
            ProcessStatus::Abandoned => "abandoned",
        }
    }

