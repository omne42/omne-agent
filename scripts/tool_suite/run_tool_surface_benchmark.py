#!/usr/bin/env python3
"""Tool-surface benchmark for model tool-calling ability.

Goals:
- Cover all current tool names from crates/app-server/src/agent/tools/spec.rs.
- Measure per-provider single-use pass and recovery pass.
- Keep full traces (system/user prompt, request/response, tool I/O, usage, reasoning).
- Support both /responses and /chat/completions endpoints and easy model/provider switching.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import copy
import dataclasses
import datetime as dt
import json
import os
import re
import statistics
import time
from pathlib import Path
from typing import Any

import requests


@dataclasses.dataclass(frozen=True)
class ProviderCase:
    name: str
    endpoint: str  # responses | chat
    base_url: str
    model: str
    api_key_env: str
    api_key: str


@dataclasses.dataclass(frozen=True)
class ToolCase:
    tool_name: str
    description: str
    schema: dict[str, Any]
    single_args: dict[str, Any]
    recovery_initial_args: dict[str, Any]
    recovery_fix_args: dict[str, Any]
    task: str
    case_id: str = ""
    feature: str = "default"
    difficulty: str = "normal"


ALLOWED_DIFFICULTIES = ("simple", "normal", "complex", "advanced")


def parse_env_file(path: Path) -> dict[str, str]:
    env: dict[str, str] = {}
    if not path.exists():
        return env
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        env[k.strip()] = v.strip()
    return env


def load_env(path: Path) -> dict[str, str]:
    env = dict(os.environ)
    env.update(parse_env_file(path))
    return env


def parse_usage(usage: dict[str, Any] | None) -> tuple[int, int, int]:
    usage = usage or {}
    input_tokens = usage.get("input_tokens")
    if not isinstance(input_tokens, int) or input_tokens <= 0:
        pt = usage.get("prompt_tokens")
        input_tokens = int(pt) if isinstance(pt, int) else 0

    output_tokens = usage.get("output_tokens")
    if not isinstance(output_tokens, int) or output_tokens < 0:
        ct = usage.get("completion_tokens")
        output_tokens = int(ct) if isinstance(ct, int) else 0

    cached_tokens = 0
    itd = usage.get("input_tokens_details")
    if isinstance(itd, dict) and isinstance(itd.get("cached_tokens"), int):
        cached_tokens = int(itd["cached_tokens"])
    else:
        ptd = usage.get("prompt_tokens_details")
        if isinstance(ptd, dict) and isinstance(ptd.get("cached_tokens"), int):
            cached_tokens = int(ptd["cached_tokens"])

    return input_tokens, output_tokens, cached_tokens


def normalize_json_text(text: str) -> str:
    t = text.strip()
    if t.startswith("```"):
        t = re.sub(r"^```(?:json)?\\s*", "", t, flags=re.IGNORECASE)
        t = re.sub(r"\\s*```$", "", t)
    return t.strip()


def parse_json_object(text: str) -> dict[str, Any] | None:
    t = normalize_json_text(text)
    try:
        value = json.loads(t)
        if isinstance(value, dict):
            return value
    except Exception:
        pass

    start = t.find("{")
    end = t.rfind("}")
    if start >= 0 and end > start:
        try:
            value = json.loads(t[start : end + 1])
            if isinstance(value, dict):
                return value
        except Exception:
            return None
    return None


def extract_responses_text(output_items: list[dict[str, Any]]) -> tuple[str, str]:
    texts: list[str] = []
    reasoning: list[str] = []
    for item in output_items:
        if item.get("type") == "message":
            for content in item.get("content", []):
                if content.get("type") in {"output_text", "text"} and isinstance(content.get("text"), str):
                    texts.append(content["text"])
        elif item.get("type") == "reasoning":
            summary = item.get("summary")
            if isinstance(summary, list):
                for chunk in summary:
                    if isinstance(chunk, dict) and isinstance(chunk.get("text"), str):
                        reasoning.append(chunk["text"])
            if isinstance(item.get("text"), str):
                reasoning.append(item["text"])
    return "\n".join(texts).strip(), "\n".join(reasoning).strip()


def sanitize_headers(headers: dict[str, str]) -> dict[str, str]:
    out = dict(headers)
    if "Authorization" in out:
        out["Authorization"] = "Bearer ***"
    return out


def load_provider_cases(path: Path, env: dict[str, str], selected: set[str] | None) -> list[ProviderCase]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, list):
        raise RuntimeError(f"provider config must be JSON list: {path}")

    out: list[ProviderCase] = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name", "")).strip()
        if not name:
            continue
        if selected and name not in selected:
            continue
        endpoint = str(item.get("endpoint", "")).strip().lower()
        base_url = str(item.get("base_url", "")).strip()
        model = str(item.get("model", "")).strip()
        api_key_env = str(item.get("api_key_env", "")).strip()
        if endpoint not in {"responses", "chat"}:
            raise RuntimeError(f"provider={name} has invalid endpoint={endpoint}")
        if not base_url or not model or not api_key_env:
            raise RuntimeError(f"provider={name} missing base_url/model/api_key_env")
        api_key = env.get(api_key_env, "").strip()
        if not api_key:
            raise RuntimeError(f"provider={name} requires missing env key: {api_key_env}")
        out.append(
            ProviderCase(
                name=name,
                endpoint=endpoint,
                base_url=base_url,
                model=model,
                api_key_env=api_key_env,
                api_key=api_key,
            )
        )

    if not out:
        raise RuntimeError("no provider selected")

    return out


def load_tool_names_from_spec(spec_rs_path: Path) -> list[str]:
    text = spec_rs_path.read_text(encoding="utf-8")
    marker = "const AGENT_TOOL_SPECS"
    start = text.find(marker)
    if start < 0:
        raise RuntimeError(f"failed to find AGENT_TOOL_SPECS in {spec_rs_path}")
    end = text.find("];", start)
    if end < 0:
        raise RuntimeError(f"failed to find AGENT_TOOL_SPECS end marker in {spec_rs_path}")
    seg = text[start:end]
    names = re.findall(r'name:\s*"([^"]+)"', seg)
    out: list[str] = []
    seen: set[str] = set()
    for n in names:
        if n not in seen:
            seen.add(n)
            out.append(n)
    return out


def schema_obj(properties: dict[str, Any], required: list[str]) -> dict[str, Any]:
    return {
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": False,
    }


def case_for_tool(tool: str) -> ToolCase:
    string = {"type": "string"}
    integer = {"type": "integer"}
    boolean = {"type": "boolean"}

    if tool == "workspace":
        schema = schema_obj(
            {
                "op": {"type": "string", "enum": ["read", "glob", "grep", "write", "patch", "edit", "delete", "mkdir"]},
                "path": string,
                "pattern": string,
                "query": string,
                "content": string,
            },
            ["op"],
        )
        return ToolCase(tool, "Workspace facade synthetic op check.", schema, {"op": "read", "path": "README.md"}, {"op": "read", "path": "MISSING.md"}, {"op": "glob", "pattern": "**/*.md"}, "Use workspace facade op to inspect project files.")

    if tool == "process":
        schema = schema_obj(
            {
                "op": {"type": "string", "enum": ["start", "inspect", "tail", "follow", "kill"]},
                "process_id": string,
                "command": {"type": "array", "items": string},
                "lines": integer,
            },
            ["op"],
        )
        return ToolCase(tool, "Process facade synthetic op check.", schema, {"op": "inspect", "process_id": "proc_001"}, {"op": "tail", "process_id": "proc_404", "lines": 20}, {"op": "inspect", "process_id": "proc_001"}, "Use process facade to inspect process metadata.")

    if tool == "thread":
        schema = schema_obj(
            {
                "op": {"type": "string", "enum": ["state", "diff", "events", "usage", "hook_run", "request_input", "spawn_agent", "send_input", "wait", "close"]},
                "hook": string,
                "max_events": integer,
                "subagent_id": string,
                "input": string,
            },
            ["op"],
        )
        return ToolCase(tool, "Thread facade synthetic op check.", schema, {"op": "state"}, {"op": "events", "max_events": 3}, {"op": "state"}, "Use thread facade to read thread state.")

    if tool == "artifact":
        schema = schema_obj(
            {
                "op": {"type": "string", "enum": ["write", "update_plan", "list", "read", "delete"]},
                "artifact_id": string,
                "artifact_type": string,
                "summary": string,
                "text": string,
                "limit": integer,
            },
            ["op"],
        )
        return ToolCase(tool, "Artifact facade synthetic op check.", schema, {"op": "list", "limit": 5}, {"op": "read", "artifact_id": "missing"}, {"op": "list", "limit": 5}, "Use artifact facade to list artifacts.")

    if tool == "integration":
        schema = schema_obj(
            {
                "op": {"type": "string", "enum": ["mcp_list_servers", "mcp_list_tools", "mcp_list_resources", "mcp_call", "web_search", "web_fetch", "view_image"]},
                "server": string,
                "tool": string,
                "arguments": {
                    "type": "object",
                    "description": "Required. The exact payload for the underlying MCP tool. Must be a JSON object containing required fields for that MCP tool. Even when empty, provide {}. Do not flatten nested MCP arguments to root.",
                },
                "query": string,
                "url": string,
                "path": string,
            },
            ["op"],
        )
        return ToolCase(tool, "Integration facade synthetic op check.", schema, {"op": "web_search", "query": "omne-agent architecture"}, {"op": "mcp_list_tools", "server": "missing"}, {"op": "web_search", "query": "omne-agent architecture"}, "Use integration facade to run a web search.")

    if tool == "file_read":
        schema = schema_obj({"path": string, "root": {"type": "string", "enum": ["workspace", "reference"]}, "max_bytes": integer}, ["path"])
        return ToolCase(tool, "Read file content.", schema, {"path": "README.md"}, {"path": "NOT_FOUND.md"}, {"path": "README.md"}, "Read README.md via file_read.")

    if tool == "file_glob":
        schema = schema_obj({"pattern": string, "root": {"type": "string", "enum": ["workspace", "reference"]}, "max_results": integer}, ["pattern"])
        return ToolCase(tool, "Glob file list.", schema, {"pattern": "**/*.md"}, {"pattern": "**/*.doesnotexist"}, {"pattern": "**/*.md"}, "Find markdown files via file_glob.")

    if tool == "file_grep":
        schema = schema_obj({"query": string, "root": {"type": "string", "enum": ["workspace", "reference"]}, "is_regex": boolean, "include_glob": string}, ["query"])
        return ToolCase(tool, "Grep text.", schema, {"query": "OmneAgent"}, {"query": "THIS_WILL_NOT_MATCH_123456"}, {"query": "OmneAgent"}, "Search keyword OmneAgent via file_grep.")

    if tool == "repo_search":
        schema = schema_obj({"query": string, "root": {"type": "string", "enum": ["workspace", "reference"]}, "is_regex": boolean, "include_glob": string}, ["query"])
        return ToolCase(tool, "Repo search.", schema, {"query": "thread_state"}, {"query": "__UNMATCHABLE_TOKEN__"}, {"query": "thread_state"}, "Run repo_search for thread_state.")

    if tool == "repo_index":
        schema = schema_obj({"root": {"type": "string", "enum": ["workspace", "reference"]}, "include_glob": string, "max_files": integer}, [])
        return ToolCase(tool, "Repo index.", schema, {}, {"include_glob": "**/*.doesnotexist"}, {}, "Generate repo index.")

    if tool == "repo_symbols":
        schema = schema_obj({"root": {"type": "string", "enum": ["workspace", "reference"]}, "path": string, "max_symbols": integer}, ["path"])
        return ToolCase(tool, "Repo symbols.", schema, {"path": "crates/app-server/src/main.rs"}, {"path": "missing.rs"}, {"path": "crates/app-server/src/main.rs"}, "Extract symbols from main.rs.")

    if tool == "repo_goto_definition":
        schema = schema_obj(
            {
                "root": {"type": "string", "enum": ["workspace", "reference"]},
                "symbol": string,
                "path": string,
                "include_glob": string,
                "max_results": integer,
            },
            ["symbol"],
        )
        return ToolCase(
            tool,
            "Repo go-to-definition.",
            schema,
            {"symbol": "handle_repo_search", "path": "crates/app-server/src/main/repo_index_search/search.rs"},
            {"symbol": ""},
            {"symbol": "handle_repo_search", "path": "crates/app-server/src/main/repo_index_search/search.rs"},
            "Locate definitions for handle_repo_search in this repo.",
        )

    if tool == "repo_find_references":
        schema = schema_obj(
            {
                "root": {"type": "string", "enum": ["workspace", "reference"]},
                "symbol": string,
                "path": string,
                "include_glob": string,
                "max_matches": integer,
            },
            ["symbol"],
        )
        return ToolCase(
            tool,
            "Repo find references.",
            schema,
            {"symbol": "handle_repo_search", "include_glob": "crates/**/*.rs"},
            {"symbol": ""},
            {"symbol": "handle_repo_search", "include_glob": "crates/**/*.rs"},
            "Find references of handle_repo_search in this repo.",
        )

    if tool == "mcp_list_servers":
        schema = schema_obj({}, [])
        return ToolCase(tool, "List MCP servers.", schema, {}, {}, {}, "Call mcp_list_servers.")

    if tool == "mcp_list_tools":
        schema = schema_obj({"server": string}, ["server"])
        return ToolCase(tool, "List MCP tools.", schema, {"server": "default"}, {"server": "missing"}, {"server": "default"}, "Call mcp_list_tools for server default.")

    if tool == "mcp_list_resources":
        schema = schema_obj({"server": string}, ["server"])
        return ToolCase(tool, "List MCP resources.", schema, {"server": "default"}, {"server": "missing"}, {"server": "default"}, "Call mcp_list_resources for server default.")

    if tool == "mcp_call":
        schema = schema_obj(
            {
                "server": string,
                "tool": string,
                "arguments": {
                    "type": "object",
                    "description": "Required. The exact payload for the underlying MCP tool. Must be a JSON object containing required fields for that MCP tool. Even when empty, provide {}. Do not flatten nested MCP arguments to root.",
                },
            },
            ["server", "tool", "arguments"],
        )
        return ToolCase(tool, "Call MCP tool.", schema, {"server": "default", "tool": "echo", "arguments": {"text": "hello"}}, {"server": "missing", "tool": "echo", "arguments": {}}, {"server": "default", "tool": "echo", "arguments": {"text": "hello"}}, "Call mcp_call with echo.")

    if tool == "file_write":
        schema = schema_obj({"path": string, "content": string}, ["path", "content"])
        return ToolCase(tool, "Write file.", schema, {"path": "tmp/tool_suite.txt", "content": "hello"}, {"path": "tmp/denied.txt", "content": "x"}, {"path": "tmp/tool_suite.txt", "content": "hello"}, "Write a temp file.")

    if tool == "file_patch":
        schema = schema_obj({"path": string, "patch": string}, ["path", "patch"])
        return ToolCase(tool, "Patch file.", schema, {"path": "README.md", "patch": "@@\n-foo\n+bar"}, {"path": "missing.md", "patch": "@@\n-a\n+b"}, {"path": "README.md", "patch": "@@\n-foo\n+bar"}, "Apply a synthetic patch.")

    if tool == "file_edit":
        schema = schema_obj({"path": string, "old_text": string, "new_text": string}, ["path", "old_text", "new_text"])
        return ToolCase(tool, "Edit file.", schema, {"path": "README.md", "old_text": "OmneAgent", "new_text": "Omne Agent"}, {"path": "README.md", "old_text": "NO_MATCH", "new_text": "X"}, {"path": "README.md", "old_text": "OmneAgent", "new_text": "Omne Agent"}, "Run file_edit with deterministic replacement.")

    if tool == "file_delete":
        schema = schema_obj({"path": string}, ["path"])
        return ToolCase(tool, "Delete file.", schema, {"path": "tmp/old.log"}, {"path": "tmp/missing.log"}, {"path": "tmp/old.log"}, "Delete target file path.")

    if tool == "fs_mkdir":
        schema = schema_obj({"path": string}, ["path"])
        return ToolCase(tool, "Create directory.", schema, {"path": "tmp/new_dir"}, {"path": "tmp/forbidden_dir"}, {"path": "tmp/new_dir"}, "Create directory with fs_mkdir.")

    if tool == "process_start":
        schema = schema_obj({"command": {"type": "array", "items": string}, "cwd": string}, ["command"])
        return ToolCase(tool, "Start process.", schema, {"command": ["echo", "hello"]}, {"command": ["invalid_cmd"]}, {"command": ["echo", "hello"]}, "Start a small process command.")

    if tool == "process_inspect":
        schema = schema_obj({"process_id": string}, ["process_id"])
        return ToolCase(tool, "Inspect process.", schema, {"process_id": "proc_001"}, {"process_id": "proc_missing"}, {"process_id": "proc_001"}, "Inspect process proc_001.")

    if tool == "process_tail":
        schema = schema_obj({"process_id": string, "lines": integer}, ["process_id"])
        return ToolCase(tool, "Tail process output.", schema, {"process_id": "proc_001", "lines": 20}, {"process_id": "proc_missing", "lines": 20}, {"process_id": "proc_001", "lines": 20}, "Tail process logs.")

    if tool == "process_follow":
        schema = schema_obj({"process_id": string, "max_chunks": integer}, ["process_id"])
        return ToolCase(tool, "Follow process output.", schema, {"process_id": "proc_001", "max_chunks": 2}, {"process_id": "proc_missing", "max_chunks": 2}, {"process_id": "proc_001", "max_chunks": 2}, "Follow process output.")

    if tool == "process_kill":
        schema = schema_obj({"process_id": string}, ["process_id"])
        return ToolCase(tool, "Kill process.", schema, {"process_id": "proc_001"}, {"process_id": "proc_missing"}, {"process_id": "proc_001"}, "Kill process proc_001.")

    if tool == "artifact_write":
        schema = schema_obj({"artifact_type": string, "summary": string, "text": string}, ["artifact_type", "summary", "text"])
        return ToolCase(tool, "Write artifact.", schema, {"artifact_type": "note", "summary": "s", "text": "hello"}, {"artifact_type": "", "summary": "", "text": ""}, {"artifact_type": "note", "summary": "s", "text": "hello"}, "Write a note artifact.")

    if tool == "update_plan":
        schema = schema_obj(
            {
                "explanation": string,
                "plan": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"step": string, "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}},
                        "required": ["step", "status"],
                        "additionalProperties": False,
                    },
                },
            },
            ["plan"],
        )
        return ToolCase(tool, "Update plan.", schema, {"plan": [{"step": "check", "status": "in_progress"}]}, {"plan": []}, {"plan": [{"step": "check", "status": "in_progress"}]}, "Update plan with one in_progress step.")

    if tool == "request_user_input":
        schema = schema_obj(
            {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"id": string, "question": string},
                        "required": ["id", "question"],
                        "additionalProperties": False,
                    },
                }
            },
            ["questions"],
        )
        return ToolCase(tool, "Request user input.", schema, {"questions": [{"id": "q1", "question": "continue?"}]}, {"questions": []}, {"questions": [{"id": "q1", "question": "continue?"}]}, "Request one user choice question.")

    if tool == "web_search":
        schema = schema_obj({"q": string, "recency": integer}, ["q"])
        return ToolCase(tool, "Web search.", schema, {"q": "omne agent"}, {"q": ""}, {"q": "omne agent"}, "Run web search query.")

    if tool == "webfetch":
        schema = schema_obj({"url": string}, ["url"])
        return ToolCase(tool, "Web fetch.", schema, {"url": "https://example.com"}, {"url": "https://invalid.invalid"}, {"url": "https://example.com"}, "Fetch one URL.")

    if tool == "view_image":
        schema = schema_obj({"path": string}, ["path"])
        return ToolCase(tool, "View image.", schema, {"path": "/tmp/demo.png"}, {"path": "/tmp/missing.png"}, {"path": "/tmp/demo.png"}, "View an image path.")

    if tool == "artifact_list":
        schema = schema_obj({"limit": integer}, [])
        return ToolCase(tool, "List artifacts.", schema, {"limit": 10}, {"limit": -1}, {"limit": 10}, "List artifacts with limit.")

    if tool == "artifact_read":
        schema = schema_obj({"artifact_id": string}, ["artifact_id"])
        return ToolCase(tool, "Read artifact.", schema, {"artifact_id": "art_001"}, {"artifact_id": "missing"}, {"artifact_id": "art_001"}, "Read artifact art_001.")

    if tool == "artifact_delete":
        schema = schema_obj({"artifact_id": string}, ["artifact_id"])
        return ToolCase(tool, "Delete artifact.", schema, {"artifact_id": "art_001"}, {"artifact_id": "missing"}, {"artifact_id": "art_001"}, "Delete artifact art_001.")

    if tool == "thread_diff":
        schema = schema_obj({}, [])
        return ToolCase(tool, "Thread diff.", schema, {}, {}, {}, "Generate thread diff.")

    if tool == "thread_state":
        schema = schema_obj({}, [])
        return ToolCase(tool, "Thread state.", schema, {}, {}, {}, "Read thread state.")

    if tool == "thread_usage":
        schema = schema_obj({}, [])
        return ToolCase(tool, "Thread usage.", schema, {}, {}, {}, "Read thread usage.")

    if tool == "thread_events":
        schema = schema_obj({"max_events": integer}, [])
        return ToolCase(tool, "Thread events.", schema, {"max_events": 20}, {"max_events": -1}, {"max_events": 20}, "Read thread events.")

    if tool == "thread_hook_run":
        schema = schema_obj({"hook": {"type": "string", "enum": ["setup", "run", "archive"]}}, ["hook"])
        return ToolCase(tool, "Thread hook run.", schema, {"hook": "setup"}, {"hook": "invalid"}, {"hook": "setup"}, "Run thread setup hook.")

    if tool == "agent_spawn":
        schema = schema_obj({"goal": string}, ["goal"])
        return ToolCase(tool, "Spawn subagent.", schema, {"goal": "check TODOs"}, {"goal": ""}, {"goal": "check TODOs"}, "Spawn one subagent.")

    if tool == "subagent_send_input":
        schema = schema_obj({"subagent_id": string, "input": string}, ["subagent_id", "input"])
        return ToolCase(tool, "Send input to subagent.", schema, {"subagent_id": "sa_001", "input": "continue"}, {"subagent_id": "missing", "input": "continue"}, {"subagent_id": "sa_001", "input": "continue"}, "Send input to subagent sa_001.")

    if tool == "subagent_wait":
        schema = schema_obj({"subagent_id": string}, ["subagent_id"])
        return ToolCase(tool, "Wait subagent.", schema, {"subagent_id": "sa_001"}, {"subagent_id": "missing"}, {"subagent_id": "sa_001"}, "Wait for subagent sa_001.")

    if tool == "subagent_close":
        schema = schema_obj({"subagent_id": string}, ["subagent_id"])
        return ToolCase(tool, "Close subagent.", schema, {"subagent_id": "sa_001"}, {"subagent_id": "missing"}, {"subagent_id": "sa_001"}, "Close subagent sa_001.")

    schema = schema_obj({"request": string}, ["request"])
    return ToolCase(tool, "Generic synthetic tool case.", schema, {"request": f"do_{tool}"}, {"request": "bad"}, {"request": f"do_{tool}"}, f"Call {tool} with request payload.")


def finalize_case(case: ToolCase, feature: str) -> ToolCase:
    case_id = case.case_id or f"{case.tool_name}__{feature}"
    return dataclasses.replace(case, case_id=case_id, feature=feature)


def feature_case(
    base: ToolCase,
    *,
    feature: str,
    task: str,
    single_args: dict[str, Any],
    recovery_initial_args: dict[str, Any],
    recovery_fix_args: dict[str, Any],
) -> ToolCase:
    return ToolCase(
        tool_name=base.tool_name,
        description=base.description,
        schema=base.schema,
        single_args=single_args,
        recovery_initial_args=recovery_initial_args,
        recovery_fix_args=recovery_fix_args,
        task=task,
        case_id=f"{base.tool_name}__{feature}",
        feature=feature,
    )


def expand_feature_cases(base: ToolCase) -> list[ToolCase]:
    tool = base.tool_name
    cases = [finalize_case(base, "default")]

    if tool == "workspace":
        cases.extend(
            [
                feature_case(
                    base,
                    feature="op_glob",
                    task="Use workspace facade with op=glob to list markdown files.",
                    single_args={"op": "glob", "pattern": "**/*.md"},
                    recovery_initial_args={"op": "glob", "pattern": "**/*.none"},
                    recovery_fix_args={"op": "glob", "pattern": "**/*.md"},
                ),
                feature_case(
                    base,
                    feature="op_grep",
                    task="Use workspace facade with op=grep to search OmneAgent.",
                    single_args={"op": "grep", "query": "OmneAgent"},
                    recovery_initial_args={"op": "grep", "query": "UNMATCHABLE__TOKEN"},
                    recovery_fix_args={"op": "grep", "query": "OmneAgent"},
                ),
                feature_case(
                    base,
                    feature="op_write",
                    task="Use workspace facade with op=write to write one file.",
                    single_args={"op": "write", "path": "tmp/ws.txt", "content": "hello"},
                    recovery_initial_args={"op": "write", "path": "tmp/denied.txt", "content": "x"},
                    recovery_fix_args={"op": "write", "path": "tmp/ws.txt", "content": "hello"},
                ),
                feature_case(
                    base,
                    feature="op_patch",
                    task="Use workspace facade with op=patch to apply a patch payload.",
                    single_args={"op": "patch", "path": "README.md", "content": "@@\n-a\n+b"},
                    recovery_initial_args={"op": "patch", "path": "missing.md", "content": "@@\n-a\n+b"},
                    recovery_fix_args={"op": "patch", "path": "README.md", "content": "@@\n-a\n+b"},
                ),
                feature_case(
                    base,
                    feature="op_edit",
                    task="Use workspace facade with op=edit for deterministic replacement.",
                    single_args={"op": "edit", "path": "README.md", "content": "replace OmneAgent -> Omne Agent"},
                    recovery_initial_args={"op": "edit", "path": "README.md", "content": "replace NO_MATCH -> X"},
                    recovery_fix_args={"op": "edit", "path": "README.md", "content": "replace OmneAgent -> Omne Agent"},
                ),
                feature_case(
                    base,
                    feature="op_delete",
                    task="Use workspace facade with op=delete to remove one file path.",
                    single_args={"op": "delete", "path": "tmp/old.log"},
                    recovery_initial_args={"op": "delete", "path": "tmp/missing.log"},
                    recovery_fix_args={"op": "delete", "path": "tmp/old.log"},
                ),
                feature_case(
                    base,
                    feature="op_mkdir",
                    task="Use workspace facade with op=mkdir to create one directory.",
                    single_args={"op": "mkdir", "path": "tmp/new_dir"},
                    recovery_initial_args={"op": "mkdir", "path": "tmp/forbidden_dir"},
                    recovery_fix_args={"op": "mkdir", "path": "tmp/new_dir"},
                ),
            ]
        )

    elif tool == "process":
        cases.extend(
            [
                feature_case(
                    base,
                    feature="op_start",
                    task="Use process facade with op=start to run a tiny command.",
                    single_args={"op": "start", "command": ["echo", "hello"]},
                    recovery_initial_args={"op": "start", "command": ["invalid_cmd"]},
                    recovery_fix_args={"op": "start", "command": ["echo", "hello"]},
                ),
                feature_case(
                    base,
                    feature="op_tail",
                    task="Use process facade with op=tail to read logs.",
                    single_args={"op": "tail", "process_id": "proc_001", "lines": 20},
                    recovery_initial_args={"op": "tail", "process_id": "proc_missing", "lines": 20},
                    recovery_fix_args={"op": "tail", "process_id": "proc_001", "lines": 20},
                ),
                feature_case(
                    base,
                    feature="op_follow",
                    task="Use process facade with op=follow for streaming output.",
                    single_args={"op": "follow", "process_id": "proc_001"},
                    recovery_initial_args={"op": "follow", "process_id": "proc_missing"},
                    recovery_fix_args={"op": "follow", "process_id": "proc_001"},
                ),
                feature_case(
                    base,
                    feature="op_kill",
                    task="Use process facade with op=kill for one process id.",
                    single_args={"op": "kill", "process_id": "proc_001"},
                    recovery_initial_args={"op": "kill", "process_id": "proc_missing"},
                    recovery_fix_args={"op": "kill", "process_id": "proc_001"},
                ),
            ]
        )

    elif tool == "thread":
        cases.extend(
            [
                feature_case(
                    base,
                    feature="op_diff",
                    task="Use thread facade with op=diff.",
                    single_args={"op": "diff"},
                    recovery_initial_args={"op": "diff"},
                    recovery_fix_args={"op": "diff"},
                ),
                feature_case(
                    base,
                    feature="op_events",
                    task="Use thread facade with op=events.",
                    single_args={"op": "events", "max_events": 20},
                    recovery_initial_args={"op": "events", "max_events": -1},
                    recovery_fix_args={"op": "events", "max_events": 20},
                ),
                feature_case(
                    base,
                    feature="op_usage",
                    task="Use thread facade with op=usage.",
                    single_args={"op": "usage"},
                    recovery_initial_args={"op": "usage"},
                    recovery_fix_args={"op": "usage"},
                ),
                feature_case(
                    base,
                    feature="op_hook_run",
                    task="Use thread facade with op=hook_run.",
                    single_args={"op": "hook_run", "hook": "setup"},
                    recovery_initial_args={"op": "hook_run", "hook": "invalid"},
                    recovery_fix_args={"op": "hook_run", "hook": "setup"},
                ),
                feature_case(
                    base,
                    feature="op_request_input",
                    task="Use thread facade with op=request_input.",
                    single_args={"op": "request_input", "input": "continue?"},
                    recovery_initial_args={"op": "request_input", "input": ""},
                    recovery_fix_args={"op": "request_input", "input": "continue?"},
                ),
                feature_case(
                    base,
                    feature="op_spawn_agent",
                    task="Use thread facade with op=spawn_agent.",
                    single_args={"op": "spawn_agent", "input": "check TODO"},
                    recovery_initial_args={"op": "spawn_agent", "input": ""},
                    recovery_fix_args={"op": "spawn_agent", "input": "check TODO"},
                ),
                feature_case(
                    base,
                    feature="op_send_input",
                    task="Use thread facade with op=send_input.",
                    single_args={"op": "send_input", "subagent_id": "sa_001", "input": "continue"},
                    recovery_initial_args={"op": "send_input", "subagent_id": "missing", "input": "continue"},
                    recovery_fix_args={"op": "send_input", "subagent_id": "sa_001", "input": "continue"},
                ),
                feature_case(
                    base,
                    feature="op_wait",
                    task="Use thread facade with op=wait.",
                    single_args={"op": "wait", "subagent_id": "sa_001"},
                    recovery_initial_args={"op": "wait", "subagent_id": "missing"},
                    recovery_fix_args={"op": "wait", "subagent_id": "sa_001"},
                ),
                feature_case(
                    base,
                    feature="op_close",
                    task="Use thread facade with op=close.",
                    single_args={"op": "close", "subagent_id": "sa_001"},
                    recovery_initial_args={"op": "close", "subagent_id": "missing"},
                    recovery_fix_args={"op": "close", "subagent_id": "sa_001"},
                ),
            ]
        )

    elif tool == "artifact":
        cases.extend(
            [
                feature_case(
                    base,
                    feature="op_write",
                    task="Use artifact facade with op=write.",
                    single_args={"op": "write", "artifact_type": "note", "summary": "s", "text": "hello"},
                    recovery_initial_args={"op": "write", "artifact_type": "", "summary": "", "text": ""},
                    recovery_fix_args={"op": "write", "artifact_type": "note", "summary": "s", "text": "hello"},
                ),
                feature_case(
                    base,
                    feature="op_update_plan",
                    task="Use artifact facade with op=update_plan.",
                    single_args={"op": "update_plan", "summary": "plan", "text": "1. step"},
                    recovery_initial_args={"op": "update_plan", "summary": "", "text": ""},
                    recovery_fix_args={"op": "update_plan", "summary": "plan", "text": "1. step"},
                ),
                feature_case(
                    base,
                    feature="op_read",
                    task="Use artifact facade with op=read.",
                    single_args={"op": "read", "artifact_id": "art_001"},
                    recovery_initial_args={"op": "read", "artifact_id": "missing"},
                    recovery_fix_args={"op": "read", "artifact_id": "art_001"},
                ),
                feature_case(
                    base,
                    feature="op_delete",
                    task="Use artifact facade with op=delete.",
                    single_args={"op": "delete", "artifact_id": "art_001"},
                    recovery_initial_args={"op": "delete", "artifact_id": "missing"},
                    recovery_fix_args={"op": "delete", "artifact_id": "art_001"},
                ),
            ]
        )

    elif tool == "integration":
        cases.extend(
            [
                feature_case(
                    base,
                    feature="op_mcp_list_servers",
                    task="Use integration facade with op=mcp_list_servers.",
                    single_args={"op": "mcp_list_servers"},
                    recovery_initial_args={"op": "mcp_list_servers"},
                    recovery_fix_args={"op": "mcp_list_servers"},
                ),
                feature_case(
                    base,
                    feature="op_mcp_list_tools",
                    task="Use integration facade with op=mcp_list_tools.",
                    single_args={"op": "mcp_list_tools", "server": "default"},
                    recovery_initial_args={"op": "mcp_list_tools", "server": "missing"},
                    recovery_fix_args={"op": "mcp_list_tools", "server": "default"},
                ),
                feature_case(
                    base,
                    feature="op_mcp_list_resources",
                    task="Use integration facade with op=mcp_list_resources.",
                    single_args={"op": "mcp_list_resources", "server": "default"},
                    recovery_initial_args={"op": "mcp_list_resources", "server": "missing"},
                    recovery_fix_args={"op": "mcp_list_resources", "server": "default"},
                ),
                feature_case(
                    base,
                    feature="op_mcp_call",
                    task="Use integration facade with op=mcp_call.",
                    single_args={"op": "mcp_call", "server": "default", "tool": "echo", "arguments": {"text": "hello"}},
                    recovery_initial_args={"op": "mcp_call", "server": "missing", "tool": "echo", "arguments": {}},
                    recovery_fix_args={"op": "mcp_call", "server": "default", "tool": "echo", "arguments": {"text": "hello"}},
                ),
                feature_case(
                    base,
                    feature="op_web_fetch",
                    task="Use integration facade with op=web_fetch.",
                    single_args={"op": "web_fetch", "url": "https://example.com"},
                    recovery_initial_args={"op": "web_fetch", "url": "https://invalid.invalid"},
                    recovery_fix_args={"op": "web_fetch", "url": "https://example.com"},
                ),
                feature_case(
                    base,
                    feature="op_view_image",
                    task="Use integration facade with op=view_image.",
                    single_args={"op": "view_image", "path": "/tmp/demo.png"},
                    recovery_initial_args={"op": "view_image", "path": "/tmp/missing.png"},
                    recovery_fix_args={"op": "view_image", "path": "/tmp/demo.png"},
                ),
            ]
        )

    return cases


def task_for_difficulty(task: str, difficulty: str) -> str:
    if difficulty == "simple":
        return f"{task} Keep steps minimal and execute one clear action."
    if difficulty == "complex":
        return (
            f"{task} Validate argument hierarchy carefully and recover from one tool error if needed."
        )
    if difficulty == "advanced":
        return (
            f"{task} Handle argument shape strictly; if parameters fail, inspect help and retry before finishing."
        )
    return task


def apply_case_difficulty(case: ToolCase, difficulty: str) -> ToolCase:
    if difficulty not in ALLOWED_DIFFICULTIES:
        raise RuntimeError(f"unsupported difficulty: {difficulty}")
    case_id = f"{case.case_id}__{difficulty}"
    return dataclasses.replace(
        case,
        case_id=case_id,
        difficulty=difficulty,
        task=task_for_difficulty(case.task, difficulty),
    )


def build_tool_cases(tool_names: list[str], difficulties: list[str]) -> list[ToolCase]:
    out: list[ToolCase] = []
    for tool in tool_names:
        base = case_for_tool(tool)
        feature_cases = expand_feature_cases(base)
        for feature_case_item in feature_cases:
            for difficulty in difficulties:
                out.append(apply_case_difficulty(feature_case_item, difficulty))
    return out


def args_match_expected(expected: dict[str, Any], received: dict[str, Any]) -> bool:
    # Models often include optional fields with empty strings/defaults.
    # For benchmark correctness we only require expected key-values to match.
    for key, expected_value in expected.items():
        if received.get(key) != expected_value:
            return False
    return True


def help_retry_hint(case: ToolCase) -> dict[str, Any] | None:
    if case.tool_name not in {"workspace", "process", "thread", "artifact", "integration"}:
        return None
    hint: dict[str, Any] = {"op": "help"}
    op = case.single_args.get("op")
    if isinstance(op, str) and op:
        hint["topic"] = op
    return hint


def retry_instruction(case: ToolCase) -> str:
    base = "You MUST call the tool again with the suggested_args to fix this. DO NOT finish the task yet."
    help_hint = help_retry_hint(case)
    if help_hint is None:
        return base
    return (
        f"{base} If argument shape is still unclear, call the tool with help args: "
        f"{json.dumps(help_hint, ensure_ascii=False)}"
    )


class ToolRuntime:
    def __init__(self, case: ToolCase, mode: str) -> None:
        self.case = case
        self.mode = mode
        self.call_index = 0

    def execute(self, args: dict[str, Any]) -> dict[str, Any]:
        self.call_index += 1

        if self.mode == "single":
            if args_match_expected(self.case.single_args, args):
                return {
                    "ok": True,
                    "tool": self.case.tool_name,
                    "mode": self.mode,
                    "call_index": self.call_index,
                    "result": "single_ok",
                    "echo_args": args,
                }
            return {
                "ok": False,
                "tool": self.case.tool_name,
                "mode": self.mode,
                "call_index": self.call_index,
                "error_code": "ARGS_MISMATCH",
                "error": "arguments mismatch",
                "message": "arguments mismatch; call again with expected_args or suggested_args",
                "expected_args": self.case.single_args,
                "suggested_args": self.case.single_args,
                "instruction": retry_instruction(self.case),
                "help_hint": help_retry_hint(self.case),
                "received_args": args,
            }

        # recovery mode: always fail first, then require recovery_fix_args.
        if self.call_index == 1:
            return {
                "ok": False,
                "tool": self.case.tool_name,
                "mode": self.mode,
                "call_index": self.call_index,
                "error_code": "INJECTED_ERROR",
                "error": "synthetic injected failure for recovery benchmark",
                "message": "synthetic injected failure; call again with suggested_args",
                "suggested_args": self.case.recovery_fix_args,
                "instruction": retry_instruction(self.case),
                "help_hint": help_retry_hint(self.case),
                "received_args": args,
            }

        if args_match_expected(self.case.recovery_fix_args, args):
            return {
                "ok": True,
                "tool": self.case.tool_name,
                "mode": self.mode,
                "call_index": self.call_index,
                "result": "recovery_ok",
                "echo_args": args,
            }

        return {
            "ok": False,
            "tool": self.case.tool_name,
            "mode": self.mode,
            "call_index": self.call_index,
            "error_code": "RECOVERY_ARGS_MISMATCH",
            "error": "recovery args mismatch",
            "message": "recovery args mismatch; use suggested_args",
            "suggested_args": self.case.recovery_fix_args,
            "instruction": retry_instruction(self.case),
            "help_hint": help_retry_hint(self.case),
            "received_args": args,
        }


def build_responses_tool_spec(case: ToolCase) -> dict[str, Any]:
    return {
        "type": "function",
        "name": case.tool_name,
        "description": case.description,
        "parameters": case.schema,
    }


def build_chat_tool_spec(case: ToolCase) -> dict[str, Any]:
    return {
        "type": "function",
        "function": {
            "name": case.tool_name,
            "description": case.description,
            "parameters": case.schema,
        },
    }


def difficulty_prompt_guidance(difficulty: str) -> str:
    if difficulty == "simple":
        return "Difficulty guidance: keep the plan short and execute direct arguments first."
    if difficulty == "complex":
        return (
            "Difficulty guidance: pay attention to exact parameter names, nesting, and type consistency."
        )
    if difficulty == "advanced":
        return (
            "Difficulty guidance: if tool args fail, actively recover using suggested_args and help before finalizing."
        )
    return "Difficulty guidance: complete the task normally with correct tool usage."


def case_prompt_hint(case: ToolCase) -> str:
    if case.tool_name == "mcp_call":
        return (
            "Parameter shape hint: call mcp_call with nested arguments, e.g. "
            "{'server':'default','tool':'echo','arguments':{'text':'hello'}}. "
            "Do not flatten nested fields."
        )
    if case.tool_name == "integration" and case.single_args.get("op") == "mcp_call":
        return (
            "Parameter shape hint: for integration op=mcp_call, keep MCP params nested under "
            "'arguments', not flattened at top level."
        )
    return ""


def build_user_prompt(case: ToolCase, mode: str) -> str:
    initial_args = case.single_args if mode == "single" else case.recovery_initial_args
    expected_keys = {"tool": case.tool_name, "mode": mode, "success": True}
    help_hint = help_retry_hint(case)
    help_line = (
        f"If parameter errors persist, call help with args JSON: {json.dumps(help_hint, ensure_ascii=False)}."
        if help_hint is not None
        else "If parameter errors persist, use expected_args/suggested_args exactly from tool output."
    )
    hint = case_prompt_hint(case)
    hint_line = f"{hint}\\n" if hint else ""
    return (
        f"Tool benchmark target: `{case.tool_name}`\\n"
        f"Mode: `{mode}`\\n"
        f"Difficulty: `{case.difficulty}`\\n"
        f"Task: {case.task}\\n"
        "You have one tool available (the target tool). Solve the task in a realistic way.\\n"
        f"Recommended starting args JSON: {json.dumps(initial_args, ensure_ascii=False)}\\n"
        "If the tool returns error and includes `expected_args` or `suggested_args`, retry using that payload.\\n"
        f"{help_line}\\n"
        f"{difficulty_prompt_guidance(case.difficulty)}\\n"
        f"{hint_line}"
        "When tool returns `ok=true`, output strict JSON only (no markdown, no prose) with at least keys: "
        f"{json.dumps(expected_keys, ensure_ascii=False)}"
    )


def evaluate_final_output(parsed_output: dict[str, Any] | None, case: ToolCase, mode: str) -> bool:
    if not isinstance(parsed_output, dict):
        return False
    if parsed_output.get("tool") != case.tool_name:
        return False
    if parsed_output.get("mode") != mode:
        return False
    if parsed_output.get("success") is not True:
        return False
    return True


def run_one_case(
    provider: ProviderCase,
    case: ToolCase,
    mode: str,
    timeout_sec: int,
    max_steps: int,
) -> dict[str, Any]:
    system_prompt = (
        "You are a deterministic tool benchmark agent. "
        "Follow instructions exactly. "
        "Use the provided tool when needed and do not fabricate tool results. "
        "Return strict JSON as the final answer."
    )
    user_prompt = build_user_prompt(case, mode)

    runtime = ToolRuntime(case, mode)
    tool_events: list[dict[str, Any]] = []
    step_traces: list[dict[str, Any]] = []
    final_text = ""
    reasoning_text = ""
    started = time.perf_counter()

    try:
        if provider.endpoint == "responses":
            history: list[dict[str, Any]] = [
                {
                    "role": "system",
                    "content": [{"type": "input_text", "text": system_prompt}],
                },
                {
                    "role": "user",
                    "content": [{"type": "input_text", "text": user_prompt}],
                },
            ]

            url = provider.base_url.rstrip("/") + "/responses"
            headers = {
                "Authorization": f"Bearer {provider.api_key}",
                "Content-Type": "application/json",
            }

            with requests.Session() as session:
                for step in range(1, max_steps + 1):
                    body = {
                        "model": provider.model,
                        "input": history,
                        "tools": [build_responses_tool_spec(case)],
                        "tool_choice": "auto",
                        "temperature": 0,
                    }
                    req_started = time.perf_counter()
                    resp = session.post(url, headers=headers, json=body, timeout=timeout_sec)
                    latency_ms = int((time.perf_counter() - req_started) * 1000)
                    data = resp.json()
                    if resp.status_code >= 400:
                        raise RuntimeError(
                            f"HTTP {resp.status_code}: {json.dumps(data, ensure_ascii=False)[:1200]}"
                        )

                    in_tok, out_tok, cached_tok = parse_usage(data.get("usage"))
                    step_trace = {
                        "step": step,
                        "request": {
                            "url": url,
                            "headers": sanitize_headers(headers),
                            "body": copy.deepcopy(body),
                        },
                        "response": copy.deepcopy(data),
                        "latency_ms": latency_ms,
                        "usage_parsed": {
                            "input_tokens": in_tok,
                            "output_tokens": out_tok,
                            "cached_tokens": cached_tok,
                        },
                        "tool_events": [],
                    }

                    output_items = data.get("output") if isinstance(data.get("output"), list) else []
                    function_calls = [item for item in output_items if item.get("type") == "function_call"]
                    if function_calls:
                        for fc in function_calls:
                            tool_name = str(fc.get("name", ""))
                            args_raw = fc.get("arguments", "{}")
                            try:
                                parsed_args = json.loads(args_raw)
                                if not isinstance(parsed_args, dict):
                                    parsed_args = {"_raw": parsed_args}
                            except Exception:
                                parsed_args = {"_raw": args_raw, "_error": "invalid_json"}

                            t0 = time.perf_counter()
                            tool_result = runtime.execute(parsed_args)
                            exec_ms = int((time.perf_counter() - t0) * 1000)
                            tool_output_text = json.dumps(tool_result, ensure_ascii=False, separators=(",", ":"))

                            history.append(
                                {
                                    "type": "function_call",
                                    "id": fc.get("id"),
                                    "call_id": fc.get("call_id"),
                                    "name": tool_name,
                                    "arguments": args_raw,
                                }
                            )
                            history.append(
                                {
                                    "type": "function_call_output",
                                    "call_id": fc.get("call_id"),
                                    "output": tool_output_text,
                                }
                            )

                            event = {
                                "tool": tool_name,
                                "call_id": fc.get("call_id"),
                                "arguments_raw": args_raw,
                                "arguments_parsed": parsed_args,
                                "tool_result": tool_result,
                                "tool_output_text": tool_output_text,
                                "execution_ms": exec_ms,
                            }
                            step_trace["tool_events"].append(event)
                            tool_events.append(event)

                        step_traces.append(step_trace)
                        continue

                    final_text, reasoning_text = extract_responses_text(output_items)
                    step_traces.append(step_trace)
                    break

        else:
            messages: list[dict[str, Any]] = [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ]
            url = provider.base_url.rstrip("/") + "/chat/completions"
            headers = {
                "Authorization": f"Bearer {provider.api_key}",
                "Content-Type": "application/json",
            }

            with requests.Session() as session:
                for step in range(1, max_steps + 1):
                    body = {
                        "model": provider.model,
                        "messages": messages,
                        "tools": [build_chat_tool_spec(case)],
                        "tool_choice": "auto",
                        "temperature": 0,
                        "stream": False,
                    }
                    req_started = time.perf_counter()
                    resp = session.post(url, headers=headers, json=body, timeout=timeout_sec)
                    latency_ms = int((time.perf_counter() - req_started) * 1000)
                    data = resp.json()
                    if resp.status_code >= 400:
                        raise RuntimeError(
                            f"HTTP {resp.status_code}: {json.dumps(data, ensure_ascii=False)[:1200]}"
                        )

                    in_tok, out_tok, cached_tok = parse_usage(data.get("usage"))
                    step_trace = {
                        "step": step,
                        "request": {
                            "url": url,
                            "headers": sanitize_headers(headers),
                            "body": copy.deepcopy(body),
                        },
                        "response": copy.deepcopy(data),
                        "latency_ms": latency_ms,
                        "usage_parsed": {
                            "input_tokens": in_tok,
                            "output_tokens": out_tok,
                            "cached_tokens": cached_tok,
                        },
                        "tool_events": [],
                    }

                    choices = data.get("choices") if isinstance(data.get("choices"), list) else []
                    if not choices:
                        raise RuntimeError("empty choices")
                    msg = choices[0].get("message") if isinstance(choices[0], dict) else None
                    if not isinstance(msg, dict):
                        raise RuntimeError("invalid message in choices[0]")

                    tool_calls = msg.get("tool_calls") if isinstance(msg.get("tool_calls"), list) else []
                    if tool_calls:
                        messages.append(
                            {
                                "role": "assistant",
                                "content": msg.get("content") or "",
                                "tool_calls": tool_calls,
                            }
                        )
                        for tc in tool_calls:
                            fn = tc.get("function") if isinstance(tc.get("function"), dict) else {}
                            tool_name = str(fn.get("name", ""))
                            args_raw = fn.get("arguments", "{}")
                            try:
                                parsed_args = json.loads(args_raw)
                                if not isinstance(parsed_args, dict):
                                    parsed_args = {"_raw": parsed_args}
                            except Exception:
                                parsed_args = {"_raw": args_raw, "_error": "invalid_json"}

                            t0 = time.perf_counter()
                            tool_result = runtime.execute(parsed_args)
                            exec_ms = int((time.perf_counter() - t0) * 1000)
                            tool_output_text = json.dumps(tool_result, ensure_ascii=False, separators=(",", ":"))

                            call_id = tc.get("id")
                            messages.append(
                                {
                                    "role": "tool",
                                    "tool_call_id": call_id,
                                    "content": tool_output_text,
                                }
                            )

                            event = {
                                "tool": tool_name,
                                "call_id": call_id,
                                "arguments_raw": args_raw,
                                "arguments_parsed": parsed_args,
                                "tool_result": tool_result,
                                "tool_output_text": tool_output_text,
                                "execution_ms": exec_ms,
                            }
                            step_trace["tool_events"].append(event)
                            tool_events.append(event)

                        step_traces.append(step_trace)
                        continue

                    final_text = str(msg.get("content") or "").strip()
                    reasoning_text = str(msg.get("reasoning_content") or "").strip()
                    step_traces.append(step_trace)
                    break

        parsed_output = parse_json_object(final_text)
        final_output_ok = evaluate_final_output(parsed_output, case, mode)
        target_events = [e for e in tool_events if e.get("tool") == case.tool_name]
        tool_call_count = len(target_events)
        first_call_ok = bool(target_events and target_events[0].get("tool_result", {}).get("ok") is True)
        encountered_error = any(e.get("tool_result", {}).get("ok") is False for e in target_events)
        eventual_ok = any(e.get("tool_result", {}).get("ok") is True for e in target_events)
        successful_event = next((e for e in target_events if e.get("tool_result", {}).get("ok") is True), None)
        adjusted = False
        if mode == "recovery" and successful_event is not None and len(target_events) >= 2:
            adjusted = (
                target_events[0].get("arguments_parsed") != successful_event.get("arguments_parsed")
            ) or (case.recovery_initial_args == case.recovery_fix_args)

        single_first_pass = mode == "single" and first_call_ok and eventual_ok and final_output_ok
        single_eventual_pass = mode == "single" and eventual_ok and final_output_ok
        recovery_pass = (
            mode == "recovery"
            and encountered_error
            and eventual_ok
            and final_output_ok
            and tool_call_count >= 2
        )

        usage_total = {
            "input_tokens": sum(s["usage_parsed"]["input_tokens"] for s in step_traces),
            "output_tokens": sum(s["usage_parsed"]["output_tokens"] for s in step_traces),
            "cached_tokens": sum(s["usage_parsed"]["cached_tokens"] for s in step_traces),
            "latency_ms": sum(s["latency_ms"] for s in step_traces),
        }

        return {
            "provider": provider.name,
            "endpoint": provider.endpoint,
            "base_url": provider.base_url,
            "model": provider.model,
            "tool": case.tool_name,
            "case_id": case.case_id,
            "feature": case.feature,
            "difficulty": case.difficulty,
            "mode": mode,
            "system_prompt": system_prompt,
            "user_prompt": user_prompt,
            "tool_case": dataclasses.asdict(case),
            "tool_events": tool_events,
            "step_traces": step_traces,
            "tool_call_count": tool_call_count,
            "first_call_ok": first_call_ok,
            "encountered_error": encountered_error,
            "eventual_ok": eventual_ok,
            "adjusted_after_error": adjusted,
            "final_text": final_text,
            "reasoning_text": reasoning_text,
            "parsed_output": parsed_output,
            "final_output_ok": final_output_ok,
            "single_first_pass": single_first_pass,
            "single_eventual_pass": single_eventual_pass,
            "recovery_pass": recovery_pass,
            "usage_total": usage_total,
            "duration_ms": int((time.perf_counter() - started) * 1000),
            "error": None,
        }
    except Exception as exc:
        return {
            "provider": provider.name,
            "endpoint": provider.endpoint,
            "base_url": provider.base_url,
            "model": provider.model,
            "tool": case.tool_name,
            "case_id": case.case_id,
            "feature": case.feature,
            "difficulty": case.difficulty,
            "mode": mode,
            "system_prompt": system_prompt,
            "user_prompt": user_prompt,
            "tool_case": dataclasses.asdict(case),
            "tool_events": tool_events,
            "step_traces": step_traces,
            "tool_call_count": len([e for e in tool_events if e.get("tool") == case.tool_name]),
            "first_call_ok": False,
            "encountered_error": False,
            "eventual_ok": False,
            "adjusted_after_error": False,
            "final_text": final_text,
            "reasoning_text": reasoning_text,
            "parsed_output": None,
            "final_output_ok": False,
            "single_first_pass": False,
            "single_eventual_pass": False,
            "recovery_pass": False,
            "usage_total": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0, "latency_ms": 0},
            "duration_ms": int((time.perf_counter() - started) * 1000),
            "error": str(exc),
        }


def safe_rate(numer: int, denom: int) -> float:
    if denom <= 0:
        return 0.0
    return numer / denom


def average(values: list[float]) -> float:
    if not values:
        return 0.0
    return float(statistics.fmean(values))


def render_report(results: list[dict[str, Any]], out_dir: Path, started_at: str, elapsed_ms: int) -> str:
    providers = sorted({r["provider"] for r in results})
    lines: list[str] = []
    lines.append("# Tool Surface Benchmark")
    lines.append("")
    lines.append(f"- Generated at: {started_at}")
    lines.append(f"- Total wall time: {elapsed_ms} ms")
    lines.append(f"- Result rows: {len(results)}")
    lines.append("")

    lines.append("## Provider Summary")
    lines.append("")
    lines.append("| provider | total_tools | total_cases | single_first_pass_rate | single_eventual_pass_rate | recovery_pass_rate | avg_latency_ms | avg_input_tokens | avg_cached_tokens |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")

    for provider in providers:
        rows = [r for r in results if r["provider"] == provider]
        single_rows = [r for r in rows if r["mode"] == "single"]
        recovery_rows = [r for r in rows if r["mode"] == "recovery"]

        total_tools = len({r["tool"] for r in rows})
        total_cases = len({r["case_id"] for r in rows})
        single_first = sum(1 for r in single_rows if r["single_first_pass"])
        single_eventual = sum(1 for r in single_rows if r["single_eventual_pass"])
        recovery_pass = sum(1 for r in recovery_rows if r["recovery_pass"])

        lat = average([float(r["usage_total"]["latency_ms"]) for r in rows])
        inp = average([float(r["usage_total"]["input_tokens"]) for r in rows])
        cac = average([float(r["usage_total"]["cached_tokens"]) for r in rows])

        lines.append(
            f"| {provider} | {total_tools} | {total_cases} | {safe_rate(single_first, len(single_rows))*100:.2f}% | "
            f"{safe_rate(single_eventual, len(single_rows))*100:.2f}% | {safe_rate(recovery_pass, len(recovery_rows))*100:.2f}% | "
            f"{lat:.1f} | {inp:.1f} | {cac:.1f} |"
        )

    lines.append("")
    lines.append("## Per Feature Case")
    lines.append("")
    lines.append("| provider | tool | feature | difficulty | case_id | single_first | single_eventual | recovery | tool_calls_single | tool_calls_recovery | error_single | error_recovery |")
    lines.append("|---|---|---|---|---|---|---|---|---:|---:|---|---|")

    for provider in providers:
        provider_rows = [r for r in results if r["provider"] == provider]
        case_ids = sorted({r["case_id"] for r in provider_rows})
        for case_id in case_ids:
            sr = next((r for r in provider_rows if r["case_id"] == case_id and r["mode"] == "single"), None)
            rr = next((r for r in provider_rows if r["case_id"] == case_id and r["mode"] == "recovery"), None)
            tool = (sr or rr or {}).get("tool", "")
            feature = (sr or rr or {}).get("feature", "")
            difficulty = (sr or rr or {}).get("difficulty", "")
            lines.append(
                f"| {provider} | {tool} | {feature} | {difficulty} | {case_id} | {'PASS' if sr and sr['single_first_pass'] else 'FAIL'} | "
                f"{'PASS' if sr and sr['single_eventual_pass'] else 'FAIL'} | "
                f"{'PASS' if rr and rr['recovery_pass'] else 'FAIL'} | "
                f"{(sr or {}).get('tool_call_count', 0)} | {(rr or {}).get('tool_call_count', 0)} | "
                f"{(sr or {}).get('error') or ''} | {(rr or {}).get('error') or ''} |"
            )

    lines.append("")
    lines.append("## Full Trace Files")
    lines.append("")
    lines.append("- Full per-case traces are under `details/<provider>/<mode>/<case_id>.json`.")
    lines.append("- Raw aggregate payload: `raw_results.json`.")

    text = "\n".join(lines)
    (out_dir / "report.md").write_text(text, encoding="utf-8")
    return text


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Benchmark all Omne tools on responses/chat interfaces.")
    p.add_argument("--env-file", type=Path, default=Path(".omne_data/.env"))
    p.add_argument(
        "--providers-file",
        type=Path,
        default=Path("scripts/tool_suite/providers.example.json"),
        help="JSON provider list",
    )
    p.add_argument(
        "--spec-rs",
        type=Path,
        default=Path("crates/app-server/src/agent/tools/spec.rs"),
        help="Path to tool spec source",
    )
    p.add_argument(
        "--providers",
        default="",
        help="Comma-separated provider names to run (default: all from providers-file).",
    )
    p.add_argument(
        "--modes",
        default="single,recovery",
        help="Comma-separated modes: single,recovery",
    )
    p.add_argument(
        "--tools",
        default="",
        help="Comma-separated tool names to include (default: all current tools).",
    )
    p.add_argument(
        "--exclude-tools",
        default="",
        help="Comma-separated tool names to exclude.",
    )
    p.add_argument(
        "--difficulties",
        default="simple,normal,complex,advanced",
        help="Comma-separated case difficulties: simple,normal,complex,advanced",
    )
    p.add_argument(
        "--list-cases-only",
        action="store_true",
        help="Only materialize the resolved case matrix (no model API calls).",
    )
    p.add_argument("--timeout-sec", type=int, default=180)
    p.add_argument("--max-steps", type=int, default=8)
    p.add_argument("--parallel-providers", type=int, default=1)
    p.add_argument("--out-dir", type=Path, default=None)
    return p.parse_args()


def main() -> int:
    args = parse_args()
    env = load_env(args.env_file)

    selected_providers = {
        x.strip() for x in args.providers.split(",") if x.strip()
    } or None
    providers = load_provider_cases(args.providers_file, env, selected_providers)

    all_tool_names = load_tool_names_from_spec(args.spec_rs)
    include_tools = {x.strip() for x in args.tools.split(",") if x.strip()}
    exclude_tools = {x.strip() for x in args.exclude_tools.split(",") if x.strip()}

    tool_names = [
        t
        for t in all_tool_names
        if (not include_tools or t in include_tools) and t not in exclude_tools
    ]
    if not tool_names:
        raise RuntimeError("no tools selected")

    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    for m in modes:
        if m not in {"single", "recovery"}:
            raise RuntimeError(f"unsupported mode: {m}")

    difficulties = [d.strip().lower() for d in args.difficulties.split(",") if d.strip()]
    if not difficulties:
        raise RuntimeError("no difficulties selected")
    for difficulty in difficulties:
        if difficulty not in ALLOWED_DIFFICULTIES:
            raise RuntimeError(f"unsupported difficulty: {difficulty}")

    tool_cases = build_tool_cases(tool_names, difficulties)

    if args.list_cases_only:
        ts = dt.datetime.now().strftime("%Y%m%d_%H%M%S")
        out_dir = args.out_dir or Path("docs/reports") / f"tool-surface-cases-{ts}"
        out_dir.mkdir(parents=True, exist_ok=True)
        payload = {
            "generated_at": dt.datetime.now().isoformat(timespec="seconds"),
            "tool_count": len(tool_names),
            "case_count": len(tool_cases),
            "difficulties": difficulties,
            "tools": tool_names,
            "cases": [dataclasses.asdict(c) for c in tool_cases],
        }
        out = out_dir / "cases.json"
        out.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"[done] cases={out}")
        return 0

    started_at = dt.datetime.now().isoformat(timespec="seconds")
    run_start = time.perf_counter()
    ts = dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    out_dir = args.out_dir or Path("docs/reports") / f"tool-surface-benchmark-{ts}"
    out_dir.mkdir(parents=True, exist_ok=True)

    results: list[dict[str, Any]] = []

    def run_provider(provider: ProviderCase) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        for mode in modes:
            for case in tool_cases:
                print(
                    f"[run] provider={provider.name} mode={mode} case={case.case_id}",
                    flush=True,
                )
                row = run_one_case(
                    provider=provider,
                    case=case,
                    mode=mode,
                    timeout_sec=args.timeout_sec,
                    max_steps=args.max_steps,
                )
                rows.append(row)
        return rows

    if args.parallel_providers > 1 and len(providers) > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel_providers) as ex:
            futs = [ex.submit(run_provider, p) for p in providers]
            for fut in concurrent.futures.as_completed(futs):
                results.extend(fut.result())
    else:
        for provider in providers:
            results.extend(run_provider(provider))

    results.sort(key=lambda r: (r["provider"], r["mode"], r["case_id"]))

    details_root = out_dir / "details"
    for row in results:
        detail_dir = details_root / row["provider"] / row["mode"]
        detail_dir.mkdir(parents=True, exist_ok=True)
        (detail_dir / f"{row['case_id']}.json").write_text(
            json.dumps(row, ensure_ascii=False, indent=2), encoding="utf-8"
        )

    meta = {
        "generated_at": started_at,
        "elapsed_ms": int((time.perf_counter() - run_start) * 1000),
        "providers": [dataclasses.asdict(p) | {"api_key": "***"} for p in providers],
        "modes": modes,
        "difficulties": difficulties,
        "tool_count": len(tool_names),
        "case_count": len(tool_cases),
        "tools": tool_names,
        "args": {
            "timeout_sec": args.timeout_sec,
            "max_steps": args.max_steps,
            "parallel_providers": args.parallel_providers,
            "providers_file": str(args.providers_file),
            "spec_rs": str(args.spec_rs),
            "difficulties": difficulties,
        },
    }

    raw = {
        "meta": meta,
        "tool_cases": [dataclasses.asdict(c) for c in tool_cases],
        "results": results,
    }
    raw_path = out_dir / "raw_results.json"
    raw_path.write_text(json.dumps(raw, ensure_ascii=False, indent=2), encoding="utf-8")

    render_report(results, out_dir, started_at, meta["elapsed_ms"])

    print(f"[done] report={out_dir / 'report.md'}")
    print(f"[done] raw={raw_path}")
    print(f"[done] details={details_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
