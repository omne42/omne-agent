#!/usr/bin/env python3
"""Realistic tool benchmark v2.

Design goals:
- User prompts are natural task intents (no prompt-coded control flow / JSON success contract).
- Inject the default full tool surface (all tools from spec.rs by default).
- Judge success via external evidence emitted from tool runtime, not self-declared model JSON.
- Keep full traces for system/user prompt, requests/responses, tool args/results, and usage.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import datetime as dt
import fnmatch
import hashlib
import json
import os
import random
import re
import secrets
import shlex
import shutil
import statistics
import subprocess
import time
from pathlib import Path
from typing import Any

import requests

import run_tool_surface_benchmark as legacy

MAX_TOOL_STRING_CHARS = 2000
MAX_TOOL_LIST_ITEMS = 40
MAX_TOOL_DICT_ITEMS = 80
MAX_TOOL_DEPTH = 6
MAX_FILE_READ_CHARS = 2000
MAX_GREP_MATCHES = 30
MAX_GREP_LINE_CHARS = 180
MAX_REPO_SYMBOLS = 40
MAX_WEB_RESULTS = 5
MAX_WEB_SNIPPET_CHARS = 320


@dataclasses.dataclass(frozen=True)
class CaseV2:
    case_id: str
    user_prompt: str
    target_tool: str
    success_args: dict[str, Any]
    capability_hint: str


@dataclasses.dataclass(frozen=True)
class RuntimeConfig:
    runtime_mode: str
    workspace_root: Path
    process_timeout_sec: int
    process_allowlist: set[str]
    web_timeout_sec: int
    web_max_bytes: int


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Realistic full-tool benchmark (v2).")
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
        "--case-source",
        choices=["file", "auto"],
        default="file",
        help="Case source: file (cases-file) or auto (one case per selected tool).",
    )
    p.add_argument(
        "--cases-file",
        type=Path,
        default=Path("scripts/tool_suite/cases.information_gap.v1.json"),
        help="Path to case file (JSON list).",
    )
    p.add_argument(
        "--providers",
        default="",
        help="Comma-separated provider names to run (default: all from providers-file).",
    )
    p.add_argument(
        "--modes",
        default="direct,recovery",
        help="Comma-separated modes: direct,recovery",
    )
    p.add_argument(
        "--tools",
        default="",
        help="Comma-separated tool names to include in full-tool injection (default: all current tools).",
    )
    p.add_argument(
        "--exclude-tools",
        default="",
        help="Comma-separated tool names to exclude from full-tool injection.",
    )
    p.add_argument("--timeout-sec", type=int, default=180)
    p.add_argument("--max-steps", type=int, default=8)
    p.add_argument("--parallel-providers", type=int, default=1)
    p.add_argument(
        "--parallel-cases",
        type=int,
        default=1,
        help="Per-provider parallelism across case/mode rows.",
    )
    p.add_argument(
        "--runtime-mode",
        choices=["mock", "real-sandbox"],
        default="real-sandbox",
        help="Tool runtime mode for benchmark execution.",
    )
    p.add_argument(
        "--runtime-workspace-root",
        type=Path,
        default=Path("tmp/tool_suite_runtime"),
        help="Workspace root for real-sandbox runtime.",
    )
    p.add_argument(
        "--real-process-timeout-sec",
        type=int,
        default=20,
        help="Per-command timeout in real-sandbox process tools.",
    )
    p.add_argument(
        "--real-process-allowlist",
        default="echo,python,python3,ls,pwd,date,whoami,cat,grep,head,tail,uname",
        help="Comma-separated allowlist for process_start commands in real-sandbox mode.",
    )
    p.add_argument(
        "--real-web-timeout-sec",
        type=int,
        default=12,
        help="HTTP timeout for web tools in real-sandbox mode.",
    )
    p.add_argument(
        "--real-web-max-bytes",
        type=int,
        default=20000,
        help="Max response bytes returned by webfetch in real-sandbox mode.",
    )
    p.add_argument(
        "--no-tool-conclusion-guard",
        choices=["off", "on"],
        default="off",
        help="Benchmark-only guard for no-tool direct conclusion (default: off).",
    )
    p.add_argument(
        "--benchmark-version",
        default="v2",
        help="Version label shown in reports and raw metadata (e.g. v3).",
    )
    p.add_argument("--out-dir", type=Path, default=None)
    p.add_argument(
        "--shuffle-cases",
        action="store_true",
        help="Shuffle case order before execution.",
    )
    p.add_argument(
        "--list-cases-only",
        action="store_true",
        help="Only materialize selected cases and tool surface (no model API calls).",
    )
    return p.parse_args()


def build_full_tool_surface(tool_names: list[str]) -> list[dict[str, Any]]:
    facade_ops: dict[str, list[str]] = {
        "workspace": [
            "help",
            "read",
            "glob",
            "grep",
            "repo_search",
            "repo_index",
            "repo_symbols",
            "repo_goto_definition",
            "repo_find_references",
            "write",
            "patch",
            "edit",
            "delete",
            "mkdir",
        ],
        "process": ["help", "start", "inspect", "tail", "follow", "kill"],
        "thread": [
            "help",
            "diff",
            "state",
            "usage",
            "events",
            "hook_run",
            "request_input",
            "spawn_agent",
            "send_input",
            "wait",
            "close",
            "close_agent",
        ],
        "artifact": ["help", "write", "update_plan", "list", "read", "delete"],
        "integration": [
            "help",
            "mcp_list_servers",
            "mcp_list_tools",
            "mcp_list_resources",
            "mcp_call",
            "web_search",
            "web_fetch",
            "view_image",
        ],
    }

    specs: list[dict[str, Any]] = []
    for name in tool_names:
        base_case = legacy.case_for_tool(name)
        description = base_case.description
        schema = base_case.schema

        if name in facade_ops:
            schema = {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": facade_ops[name]},
                    "help": {"type": "boolean"},
                    "topic": {"type": "string"},
                },
                "required": ["op"],
                "additionalProperties": True,
            }
            description = (
                f"{name} facade. For op!=help, all operation parameters MUST be flat root-level fields "
                "(alongside `op`). Do not nest parameters under `args`. "
                "Use op=help for quickstart and advanced usage."
            )
        elif name == "mcp_call":
            # Flatten mcp_call input for model ergonomics; runtime will auto-pack extras into `arguments`.
            schema = {
                "type": "object",
                "properties": {
                    "server": {"type": "string"},
                    "tool": {"type": "string"},
                    "arguments": {"type": "object"},
                },
                "required": ["tool"],
                "additionalProperties": True,
            }
            description += (
                " Prefer flat root-level MCP arguments (for example "
                "`{\"server\":\"default\",\"tool\":\"echo\",\"text\":\"hello\"}`) "
                "instead of nesting everything under `arguments`."
            )

        if name == "file_grep":
            description += (
                " Use exact text/regex line matching and return concrete line-level hits. "
                " Always narrow scope with `include_glob` (for example `crates/**/*.rs`) "
                "to avoid oversized result sets."
            )
        elif name == "repo_search":
            description += (
                " Use broad repository-level discovery when users ask for higher-level location hints. "
                " Prefer include_glob when scope is clear."
            )
        elif name == "webfetch":
            description += (
                " Use concise facts from `title` and `content_preview`; "
                "do not copy raw HTML into later tool arguments."
            )
        specs.append(
            {
                "name": name,
                "description": description,
                "schema": schema,
            }
        )

    ask_user_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "question": {"type": "string"},
            "purpose": {"type": "string"},
            "choices": {
                "type": "array",
                "items": {"type": "string"},
            },
        },
        "required": ["question"],
        "additionalProperties": True,
    }
    ask_user_desc = (
        "Ask user for missing information or confirmation required to complete the task. "
        "Use only when blocked on user input; after receiving user reply, continue and finish the task."
    )
    if not any(s.get("name") == "ask_user" for s in specs):
        specs.append({"name": "ask_user", "description": ask_user_desc, "schema": ask_user_schema})
    return specs


def build_responses_tools(surface: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "type": "function",
            "name": spec["name"],
            "description": spec["description"],
            "parameters": spec["schema"],
        }
        for spec in surface
    ]


def build_chat_tools(surface: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "type": "function",
            "function": {
                "name": spec["name"],
                "description": spec["description"],
                "parameters": spec["schema"],
            },
        }
        for spec in surface
    ]


def is_user_query_tool_call(tool_name: str, args: dict[str, Any]) -> bool:
    if tool_name in {"ask_user", "request_user_input"}:
        return True
    if tool_name == "thread":
        op = _first_string_arg(args, ["op"]).lower()
        return op in {"request_input", "request_user_input"}
    return False


def simulated_user_reply_text(tool_name: str, args: dict[str, Any]) -> str:
    purpose = _first_string_arg(args, ["purpose", "objective", "reason"]).strip()
    if not purpose:
        purpose = "确认继续执行并完整完成当前请求"

    question = _first_string_arg(args, ["question", "prompt", "input"]).strip()
    if not question:
        questions = args.get("questions")
        if isinstance(questions, list) and questions:
            first = questions[0]
            if isinstance(first, dict):
                question = safe_text(first.get("question")).strip()

    if question:
        return (
            f"用户回复（针对你刚才的问题“{question}”）：已确认，请继续执行并完整完成原始请求。"
            f" 目的：{purpose}。"
        )
    return f"用户回复：已确认，请继续执行并完整完成原始请求。目的：{purpose}。"


def normalize_string_for_compare(value: str) -> str:
    return value.replace("\r\n", "\n")


def value_match(expected: Any, received: Any) -> bool:
    if isinstance(expected, str) and isinstance(received, str):
        e = normalize_string_for_compare(expected)
        r = normalize_string_for_compare(received)
        if e == r:
            return True
        # Allow common escaped-newline variant.
        if e.replace("\\n", "\n") == r.replace("\\n", "\n"):
            return True
        # Allow harmless trailing newline differences for text payloads.
        return e.rstrip("\n") == r.rstrip("\n")
    if isinstance(expected, dict) and isinstance(received, dict):
        for key, expected_value in expected.items():
            if key not in received:
                return False
            if not value_match(expected_value, received[key]):
                return False
        return True
    if isinstance(expected, list) and isinstance(received, list):
        if len(expected) != len(received):
            return False
        return all(value_match(e, r) for e, r in zip(expected, received))
    return expected == received


def args_match_expected(expected: dict[str, Any], received: dict[str, Any]) -> bool:
    for key, expected_value in expected.items():
        if key not in received:
            return False
        if not value_match(expected_value, received[key]):
            return False
    return True


def parse_cases(path: Path, allowed_tools: set[str]) -> list[CaseV2]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, list):
        raise RuntimeError(f"case file must be JSON list: {path}")

    cases: list[CaseV2] = []
    seen: set[str] = set()
    for item in payload:
        if not isinstance(item, dict):
            continue
        case_id = str(item.get("id", "")).strip()
        user_prompt = str(item.get("user_prompt", "")).strip()
        target_tool = str(item.get("target_tool", "")).strip()
        success_args = item.get("success_args")
        capability_hint = str(item.get("capability_hint", "")).strip()
        if (
            not case_id
            or not user_prompt
            or not target_tool
            or not isinstance(success_args, dict)
            or not capability_hint
        ):
            raise RuntimeError(f"invalid case item: {item}")
        if case_id in seen:
            raise RuntimeError(f"duplicate case id: {case_id}")
        if target_tool not in allowed_tools:
            raise RuntimeError(
                f"case={case_id} target_tool={target_tool} is not in selected full tool surface"
            )
        seen.add(case_id)
        cases.append(
            CaseV2(
                case_id=case_id,
                user_prompt=user_prompt,
                target_tool=target_tool,
                success_args=success_args,
                capability_hint=capability_hint,
            )
        )

    if not cases:
        raise RuntimeError(f"no valid cases in {path}")
    return cases


def _first_string_arg(args: dict[str, Any], keys: list[str]) -> str:
    for key in keys:
        value = args.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""


def auto_user_prompt_for_tool(tool: str, success_args: dict[str, Any], fallback_task: str) -> str:
    if tool == "workspace":
        path = _first_string_arg(success_args, ["path"]) or "README.md"
        return f"请在仓库里读取 `{path}` 开头，并告诉我是否包含 OmneAgent。"
    if tool == "process":
        op = _first_string_arg(success_args, ["op"]).lower() or "inspect"
        if op == "start":
            return "请实际执行一次 `echo hello` 并把 stdout 原文返回。"
        if op == "inspect":
            return "请检查进程 proc_001 当前状态，告诉我是否在运行。"
        if op == "tail":
            return "请读取进程 proc_001 最近 20 行输出。"
        if op == "follow":
            return "请跟踪进程 proc_001 输出，拿到 2 个输出片段。"
        if op == "kill":
            return "请终止进程 proc_001，并确认终止是否成功。"
        return "请帮我处理一个进程状态请求。"
    if tool == "thread":
        op = _first_string_arg(success_args, ["op"]).lower() or "state"
        if op == "state":
            return "请查询当前会话线程状态，并告诉我事件数量。"
        if op == "diff":
            return "请查询当前线程与起始状态的差异。"
        if op == "events":
            return "请读取当前线程最近事件列表。"
        if op == "usage":
            return "请查询当前线程 token 用量与工具调用统计。"
        if op == "hook_run":
            return "请执行一次线程 hook（setup）并确认执行结果。"
        if op == "spawn_agent":
            return "请在当前线程里创建一个子代理，目标是检查 TODO。"
        if op == "send_input":
            return "请向子代理 sa_001 发送输入 continue。"
        if op == "wait":
            return "请等待子代理 sa_001 返回最新输出。"
        if op == "close":
            return "请关闭子代理 sa_001。"
        return "请查询当前线程状态信息。"
    if tool == "artifact":
        op = _first_string_arg(success_args, ["op"]).lower() or "list"
        if op == "write":
            return "请写入一条 artifact，内容 hello，并返回 artifact_id。"
        if op == "list":
            return "请列出当前 artifact 列表，返回前几条 id。"
        if op == "read":
            return "请读取 artifact art_001 的内容。"
        if op == "delete":
            return "请删除 artifact art_001，并确认结果。"
        return "请处理一次 artifact 操作。"
    if tool == "integration":
        op = _first_string_arg(success_args, ["op"]).lower() or "web_search"
        if op == "web_search":
            return "请联网搜索 omne-agent architecture，并给我一条结果标题。"
        if op == "webfetch":
            return "请抓取 https://example.com 的网页标题。"
        if op == "view_image":
            return "请读取 /tmp/demo.png 并告诉我图片格式和大小。"
        return "请调用一次 integration 能力并返回真实结果。"
    if tool == "file_read":
        path = _first_string_arg(success_args, ["path"]) or "README.md"
        return f"请读取 `{path}` 的原文开头，并基于真实内容回答。"
    if tool == "file_glob":
        pattern = _first_string_arg(success_args, ["pattern"]) or "**/*.md"
        return f"请按模式 `{pattern}` 递归列出匹配文件，并返回数量与样例。"
    if tool == "file_grep":
        query = _first_string_arg(success_args, ["query"]) or "OmneAgent"
        return (
            f"请做一次精确字符串匹配（类似 grep），在代码文件里搜索 `{query}`，"
            "并返回命中的文件与行号。"
        )
    if tool == "repo_search":
        query = _first_string_arg(success_args, ["query"]) or "thread_state"
        return f"请做一次仓库级位置检索（可模糊匹配），查 `{query}` 相关位置并返回样例。"
    if tool == "repo_index":
        return "请统计当前仓库文件总数与主要后缀分布。"
    if tool == "repo_symbols":
        path = _first_string_arg(success_args, ["path"]) or "crates/app-server/src/main.rs"
        return f"请解析 `{path}` 里的符号定义，并列出前几个函数或类型名。"
    if tool == "repo_goto_definition":
        symbol = _first_string_arg(success_args, ["symbol"]) or "handle_repo_search"
        return f"请在仓库中定位符号 `{symbol}` 的定义位置，返回文件路径和行号。"
    if tool == "repo_find_references":
        symbol = _first_string_arg(success_args, ["symbol"]) or "handle_repo_search"
        return f"请在仓库中查找符号 `{symbol}` 的引用位置，并返回若干命中行。"
    if tool == "file_write":
        path = _first_string_arg(success_args, ["path"]) or "tmp/auto.txt"
        content = _first_string_arg(success_args, ["content"]) or "hello"
        return f"请在 `{path}` 写入 `{content}`，并告诉我写入字节数。"
    if tool == "file_edit":
        path = _first_string_arg(success_args, ["path"]) or "README.md"
        old_text = _first_string_arg(success_args, ["old_text"]) or "old"
        new_text = _first_string_arg(success_args, ["new_text"]) or "new"
        return (
            f"请在 `{path}` 中把 `{old_text}` 替换为 `{new_text}`，"
            "并告诉我实际替换了几处。"
        )
    if tool == "file_patch":
        path = _first_string_arg(success_args, ["path"]) or "README.md"
        patch_text = _first_string_arg(success_args, ["patch"]) or "@@\n-foo\n+bar"
        return (
            f"请对 `{path}` 应用下面这段补丁，并告诉我是替换还是追加：\n"
            f"{patch_text}"
        )
    if tool == "file_delete":
        path = _first_string_arg(success_args, ["path"]) or "tmp/old.log"
        return f"请删除 `{path}` 并确认删除成功。"
    if tool == "fs_mkdir":
        path = _first_string_arg(success_args, ["path"]) or "tmp/new_dir"
        return f"请在工作区创建目录 `{path}` 并返回最终路径。"
    if tool == "process_start":
        return "请实际执行命令 `echo hello`，并返回 stdout 原文。"
    if tool == "process_inspect":
        return "请检查进程 proc_001 的当前状态（是否运行、返回码）。"
    if tool == "process_tail":
        return "请读取进程 proc_001 最近 20 行输出并返回。"
    if tool == "process_follow":
        return "请跟踪进程 proc_001 输出，返回 2 个输出片段。"
    if tool == "process_kill":
        return "请对进程 proc_001 执行 kill 操作，并返回 killed 字段与 returncode。"
    if tool == "artifact_write":
        return "请写入一个 note 类型 artifact：summary=s, text=hello，并返回 artifact_id。"
    if tool == "artifact_list":
        return "请列出当前 artifact 列表（limit=10），返回具体 id。"
    if tool == "artifact_read":
        return "请读取 artifact art_001 的内容并返回预览。"
    if tool == "artifact_delete":
        return "请删除 artifact art_001，并返回删除结果。"
    if tool == "update_plan":
        return (
            "请调用计划更新能力，把计划更新为一个步骤：step=check, status=in_progress。"
            "并返回工具结果里的 plan_size 数字。"
        )
    if tool == "request_user_input":
        return (
            "请发起一个用户输入请求：问题 id=q1，问题为 continue?。"
            "并返回工具输出中的 requested_question_ids。"
        )
    if tool == "web_search":
        return "请联网搜索 omne agent，并返回至少一条搜索结果。"
    if tool == "webfetch":
        return "请抓取 https://example.com 并返回页面 title。"
    if tool == "view_image":
        return "请读取 /tmp/demo.png，返回图片格式、文件字节数和 sha256。"
    if tool == "mcp_list_servers":
        return "请列出当前 MCP server 列表。"
    if tool == "mcp_list_tools":
        return "请查询 default MCP server 当前可用工具列表，并返回第一个工具名。"
    if tool == "mcp_list_resources":
        return "请查询 default MCP server 当前资源列表。"
    if tool == "mcp_call":
        return "请通过 MCP 调用 echo 工具，参数 text=hello，并返回调用结果。"
    if tool == "thread_diff":
        return "请读取当前线程差异，并把 event_delta_since_start 这个数字返回给我。"
    if tool == "thread_state":
        return "请查询当前线程状态并返回 event_count。"
    if tool == "thread_usage":
        return "请查询当前线程使用量（tool_calls 和 token 统计）。"
    if tool == "thread_events":
        return "请查询当前线程最近事件（max_events=20）。"
    if tool == "thread_hook_run":
        return "请执行线程 hook `setup` 并反馈执行状态。"
    if tool == "agent_spawn":
        return "请创建一个子代理，目标是检查 TODOs，并返回 subagent_id。"
    if tool == "subagent_send_input":
        return "请给子代理 sa_001 发送输入 `continue`，并返回处理结果。"
    if tool == "subagent_wait":
        return "请等待子代理 sa_001，返回其最新状态和输出。"
    if tool == "subagent_close":
        return "请关闭子代理 sa_001 并确认关闭状态。"
    return f"请帮我完成这个请求：{fallback_task}"


def auto_capability_hint(tool: str, success_args: dict[str, Any]) -> str:
    if tool in {"file_read", "file_glob", "file_grep", "file_write", "file_patch", "file_edit", "file_delete"}:
        return "perform direct workspace file operations"
    if tool in {"repo_search", "repo_index", "repo_symbols", "repo_goto_definition", "repo_find_references"}:
        return "perform semantic or structural repository navigation"
    if tool in {"process", "process_start", "process_inspect", "process_tail", "process_follow", "process_kill"}:
        return "execute or inspect process runtime actions"
    if tool in {"thread", "thread_state", "thread_diff", "thread_events", "thread_usage", "thread_hook_run"}:
        return "query or operate thread runtime state"
    if tool in {"artifact", "artifact_write", "artifact_list", "artifact_read", "artifact_delete"}:
        return "read or write artifact records"
    if tool in {"mcp_list_servers", "mcp_list_tools", "mcp_list_resources", "mcp_call"}:
        return "use MCP operations with proper argument shape"
    return f"use tool {tool} with correct parameters"


def _default_readme_path() -> str:
    for candidate in ["README.md", "openspec/README.md", "docs/README.md"]:
        if (Path.cwd() / candidate).is_file():
            return candidate
    return "README.md"


def _massage_auto_success_args(tool: str, success_args: dict[str, Any]) -> dict[str, Any]:
    out = dict(success_args)
    readme = _default_readme_path()

    if tool in {"workspace", "file_read", "file_edit", "file_patch"}:
        if safe_text(out.get("path")) == "README.md":
            out["path"] = readme

    if tool == "file_edit":
        path = safe_text(out.get("path")) or readme
        old_text = safe_text(out.get("old_text"))
        new_text = safe_text(out.get("new_text"))
        candidate = Path.cwd() / path
        if candidate.is_file():
            content = candidate.read_text(encoding="utf-8", errors="replace")
            if old_text and old_text not in content:
                if "OmneAgent" in content:
                    out["old_text"] = "OmneAgent"
                    out["new_text"] = new_text or "Omne Agent"
                elif "Omne Agent" in content:
                    out["old_text"] = "Omne Agent"
                    out["new_text"] = new_text or "OmneAgent"
                else:
                    first = content.splitlines()[0].strip() if content.splitlines() else "OpenSpec"
                    out["old_text"] = first[:30]
                    out["new_text"] = f"{out['old_text']} (edited)"

    if tool == "file_patch":
        path = safe_text(out.get("path")) or readme
        candidate = Path.cwd() / path
        if candidate.is_file():
            content = candidate.read_text(encoding="utf-8", errors="replace")
            first_nonempty = ""
            for line in content.splitlines():
                if line.strip():
                    first_nonempty = line.strip()
                    break
            if not first_nonempty:
                first_nonempty = "OpenSpec"
            out["patch"] = f"@@\n-{first_nonempty}\n+{first_nonempty} [patched]"

    if tool == "repo_symbols":
        path = safe_text(out.get("path"))
        if not path or not (Path.cwd() / path).is_file():
            out["path"] = "scripts/tool_suite/run_tool_surface_benchmark_v2.py"
    if tool in {"repo_goto_definition", "repo_find_references"}:
        symbol = safe_text(out.get("symbol"))
        if not symbol:
            out["symbol"] = "handle_repo_search"
        if not safe_text(out.get("include_glob")):
            out["include_glob"] = "crates/**/*.rs"

    return out


def build_auto_cases_from_tools(tool_names: list[str]) -> list[CaseV2]:
    cases: list[CaseV2] = []
    for tool in tool_names:
        base = legacy.case_for_tool(tool)
        success_args = _massage_auto_success_args(tool, dict(base.single_args))
        if not success_args:
            # Ensure tool is still called for empty-arg tools.
            success_args = {}
        case_id = f"auto__{tool}"
        cases.append(
            CaseV2(
                case_id=case_id,
                user_prompt=auto_user_prompt_for_tool(tool, success_args, base.task),
                target_tool=tool,
                success_args=success_args,
                capability_hint=auto_capability_hint(tool, success_args),
            )
        )
    return cases


def safe_text(value: Any) -> str:
    if value is None:
        return ""
    return str(value)


def _truncate_text(value: str, max_chars: int) -> str:
    if len(value) <= max_chars:
        return value
    omitted = len(value) - max_chars
    return f"{value[:max_chars]}... [truncated {omitted} chars]"


def compact_tool_result_for_model(value: Any, depth: int = 0) -> Any:
    # Keep tool feedback compact for model-side stability on large payload tools.
    if depth >= MAX_TOOL_DEPTH:
        return "[truncated depth]"
    if isinstance(value, str):
        return _truncate_text(value, MAX_TOOL_STRING_CHARS)
    if isinstance(value, list):
        items = [compact_tool_result_for_model(v, depth + 1) for v in value[:MAX_TOOL_LIST_ITEMS]]
        if len(value) > MAX_TOOL_LIST_ITEMS:
            items.append({"_truncated_items": len(value) - MAX_TOOL_LIST_ITEMS})
        return items
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        items = list(value.items())
        for key, item in items[:MAX_TOOL_DICT_ITEMS]:
            out[str(key)] = compact_tool_result_for_model(item, depth + 1)
        if len(items) > MAX_TOOL_DICT_ITEMS:
            out["_truncated_keys"] = len(items) - MAX_TOOL_DICT_ITEMS
        return out
    return value


def extract_chat_text_and_reasoning(msg: dict[str, Any]) -> tuple[str, str]:
    text = safe_text(msg.get("content")).strip()
    reasoning = safe_text(msg.get("reasoning_content")).strip()
    return text, reasoning


class RuntimeBase:
    def __init__(self, case: CaseV2, mode: str) -> None:
        self.case = case
        self.mode = mode
        self.target_call_index = 0
        self.evidence_nonce = f"EVIDENCE-{case.case_id}-{secrets.token_hex(6)}"

    def _normalize_mcp_args(self, args: dict[str, Any]) -> dict[str, Any]:
        normalized = dict(args)
        arguments: dict[str, Any] = {}
        if isinstance(normalized.get("arguments"), dict):
            arguments.update(normalized["arguments"])
        elif isinstance(normalized.get("arguments"), str):
            try:
                parsed = json.loads(normalized["arguments"])
                if isinstance(parsed, dict):
                    arguments.update(parsed)
            except Exception:
                pass

        for wrapper in ["input", "params", "payload", "args"]:
            wrapped = normalized.get(wrapper)
            if isinstance(wrapped, dict):
                for inner_key, inner_value in wrapped.items():
                    arguments.setdefault(inner_key, inner_value)

        for key, value in normalized.items():
            if key in {"server", "tool", "arguments", "input", "params", "payload", "args"}:
                continue
            arguments.setdefault(key, value)

        if str(normalized.get("server", "")).strip().lower() == "mcp":
            normalized["server"] = "default"
        normalized["arguments"] = arguments
        return normalized

    def _normalize_args_for_match(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any]:
        if tool_name in {"workspace", "process", "thread", "artifact", "integration"}:
            return dict(args)
        if tool_name != "mcp_call":
            return dict(args)
        return self._normalize_mcp_args(args)

    def _match_target_or_alias(self, tool_name: str, args: dict[str, Any]) -> tuple[bool, str | None]:
        if tool_name == self.case.target_tool:
            return True, None

        target = self.case.target_tool
        target_op = str(self.case.success_args.get("op", "")).strip().lower()
        call_op = str(args.get("op", "")).strip().lower()

        workspace_op_to_atomic = {
            "read": {"file_read"},
            "glob": {"file_glob"},
            "grep": {"file_grep", "repo_search"},
            "repo_search": {"repo_search", "file_grep"},
            "repo_index": {"repo_index", "file_glob"},
            "repo_symbols": {"repo_symbols", "file_grep"},
            "repo_goto_definition": {"repo_goto_definition", "repo_search", "file_grep"},
            "repo_find_references": {"repo_find_references", "repo_search", "file_grep"},
            "write": {"file_write"},
            "patch": {"file_patch"},
            "edit": {"file_edit"},
            "delete": {"file_delete"},
            "mkdir": {"fs_mkdir"},
        }
        process_op_to_atomic = {
            "start": {"process_start"},
            "inspect": {"process_inspect"},
            "tail": {"process_tail"},
            "follow": {"process_follow"},
            "kill": {"process_kill"},
        }
        thread_op_to_atomic = {
            "state": {"thread_state"},
            "diff": {"thread_diff"},
            "events": {"thread_events"},
            "usage": {"thread_usage"},
            "hook_run": {"thread_hook_run"},
            "request_input": {"request_user_input", "ask_user"},
            "request_user_input": {"request_user_input", "ask_user"},
            "spawn_agent": {"agent_spawn"},
            "send_input": {"subagent_send_input"},
            "wait": {"subagent_wait"},
            "close": {"subagent_close"},
            "close_agent": {"subagent_close"},
        }
        artifact_op_to_atomic = {
            "write": {"artifact_write"},
            "update_plan": {"update_plan"},
            "list": {"artifact_list"},
            "read": {"artifact_read"},
            "delete": {"artifact_delete"},
        }
        integration_op_to_atomic = {
            "mcp_list_servers": {"mcp_list_servers"},
            "mcp_list_tools": {"mcp_list_tools"},
            "mcp_list_resources": {"mcp_list_resources"},
            "mcp_call": {"mcp_call"},
            "web_search": {"web_search"},
            "web_fetch": {"webfetch"},
            "webfetch": {"webfetch"},
            "view_image": {"view_image"},
        }

        if target == "workspace" and tool_name in workspace_op_to_atomic.get(target_op, set()):
            return True, f"workspace(op={target_op}) <-> {tool_name} alias"
        if target == "process" and tool_name in process_op_to_atomic.get(target_op, set()):
            return True, f"process(op={target_op}) <-> {tool_name} alias"
        if target == "thread" and tool_name in thread_op_to_atomic.get(target_op, set()):
            return True, f"thread(op={target_op}) <-> {tool_name} alias"
        if target == "artifact" and tool_name in artifact_op_to_atomic.get(target_op, set()):
            return True, f"artifact(op={target_op}) <-> {tool_name} alias"
        if target == "integration" and tool_name in integration_op_to_atomic.get(target_op, set()):
            return True, f"integration(op={target_op}) <-> {tool_name} alias"

        if tool_name == "workspace":
            for op, members in workspace_op_to_atomic.items():
                if target in members and call_op == op:
                    return True, f"{target} <-> workspace(op={op}) alias"
        if tool_name == "process":
            for op, members in process_op_to_atomic.items():
                if target in members and call_op == op:
                    return True, f"{target} <-> process(op={op}) alias"
        if tool_name == "thread":
            for op, members in thread_op_to_atomic.items():
                if target in members and call_op == op:
                    return True, f"{target} <-> thread(op={op}) alias"
        if tool_name == "artifact":
            for op, members in artifact_op_to_atomic.items():
                if target in members and call_op == op:
                    return True, f"{target} <-> artifact(op={op}) alias"
        if tool_name == "integration":
            for op, members in integration_op_to_atomic.items():
                if target in members and call_op == op:
                    return True, f"{target} <-> integration(op={op}) alias"

        return False, None

    def is_target_match(self, tool_name: str, args: dict[str, Any]) -> bool:
        matched, _ = self._match_target_or_alias(tool_name, args)
        return matched

    def close(self) -> None:
        return

    def execute(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any]:
        raise NotImplementedError


class MockRuntime(RuntimeBase):
    def _auxiliary_tools_for_case(self) -> set[str]:
        target = self.case.target_tool
        if target in {"file_edit", "file_patch", "file_write", "file_delete"}:
            return {"file_read", "file_glob", "file_grep", "repo_search", "repo_index"}
        if target == "mcp_call":
            return {"mcp_list_servers", "mcp_list_tools", "mcp_list_resources"}
        if target == "process_kill":
            return {"process_inspect", "process_tail", "process_follow"}
        return set()

    def execute(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any]:
        retry_instruction = (
            "You MUST call the tool again with the suggested_args to fix this. "
            "DO NOT finish the task yet."
        )
        matched_target, alias_reason = self._match_target_or_alias(tool_name, args)
        if not matched_target:
            if tool_name in self._auxiliary_tools_for_case():
                return {
                    "ok": True,
                    "result": "auxiliary_step_completed",
                    "summary": f"Accepted exploratory tool step {tool_name} before target action.",
                    "target_tool": self.case.target_tool,
                    "evidence_nonce": self.evidence_nonce,
                }
            return {
                "ok": False,
                "error_code": "WRONG_TOOL",
                "error": "Wrong tool for this task.",
                "message": "This tool call did not progress the user request.",
                "hint": f"Use a capability that can: {self.case.capability_hint}",
                "called_tool": tool_name,
                "expected_tool": self.case.target_tool,
                "suggested_args": self.case.success_args,
                "instruction": retry_instruction,
            }

        self.target_call_index += 1
        if self.mode == "recovery" and self.target_call_index == 1:
            return {
                "ok": False,
                "error_code": "INJECTED_TRANSIENT",
                "error": "Injected transient failure.",
                "message": "Temporary runtime failure. Retry the same operation.",
                "retryable": True,
                "suggested_args": self.case.success_args,
                "instruction": retry_instruction,
            }

        if alias_reason is not None:
            return {
                "ok": True,
                "result": "task_completed_via_alias",
                "target_tool": self.case.target_tool,
                "matched_tool": tool_name,
                "alias_reason": alias_reason,
                "evidence_nonce": self.evidence_nonce,
                "summary": f"Completed action for case {self.case.case_id} via alias.",
            }

        normalized_args = self._normalize_args_for_match(tool_name, args)
        if args_match_expected(self.case.success_args, normalized_args):
            return {
                "ok": True,
                "result": "task_completed",
                "target_tool": self.case.target_tool,
                "evidence_nonce": self.evidence_nonce,
                "summary": f"Completed action for case {self.case.case_id}.",
            }

        return {
            "ok": False,
            "error_code": "ARGS_MISMATCH",
            "error": "Tool arguments mismatch.",
            "message": "Tool arguments did not match the required action shape.",
            "suggested_args": self.case.success_args,
            "instruction": retry_instruction,
        }


class RealSandboxRuntime(RuntimeBase):
    def __init__(self, case: CaseV2, mode: str, config: RuntimeConfig) -> None:
        super().__init__(case, mode)
        self.config = config
        self.repo_root = Path.cwd().resolve()
        slug = re.sub(r"[^a-zA-Z0-9._-]+", "_", case.case_id).strip("_") or "case"
        self.runtime_root = (config.workspace_root / f"{slug}_{secrets.token_hex(3)}").resolve()
        self.runtime_root.mkdir(parents=True, exist_ok=True)
        self.artifact_root = self.runtime_root / ".artifacts"
        self.artifact_root.mkdir(parents=True, exist_ok=True)
        self.total_tool_calls = 0
        self.thread_events_log: list[dict[str, Any]] = []
        self.thread_baseline_event_count = 0
        self.thread_plan: list[dict[str, Any]] = []
        self.subagents: dict[str, dict[str, Any]] = {}
        self.artifacts: dict[str, dict[str, Any]] = {}
        self.processes: dict[str, dict[str, Any]] = {}
        # Files mutated in sandbox should shadow repository files during search/read style checks.
        self.overlay_paths: set[str] = set()
        self.mcp_servers: dict[str, dict[str, Any]] = {
            "default": {
                "tools": {
                    "echo": {
                        "description": "Echo back the input text.",
                        "input_schema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"],
                        },
                    }
                },
                "resources": [{"name": "bench://default", "description": "Default benchmark server"}],
            }
        }
        self._prepare_seed_state()

    def close(self) -> None:
        for record in self.processes.values():
            popen = record.get("popen")
            if isinstance(popen, subprocess.Popen):
                try:
                    if popen.poll() is None:
                        popen.terminate()
                        popen.wait(timeout=1.0)
                except Exception:
                    try:
                        popen.kill()
                    except Exception:
                        pass

    def _record_event(self, kind: str, detail: dict[str, Any]) -> None:
        self.thread_events_log.append(
            {
                "ts": dt.datetime.now().isoformat(timespec="seconds"),
                "kind": kind,
                "detail": detail,
            }
        )

    def _prepare_seed_state(self) -> None:
        demo_png = Path("/tmp/demo.png")
        if not demo_png.exists():
            demo_png.write_bytes(
                bytes.fromhex(
                    "89504E470D0A1A0A0000000D49484452000000010000000108060000001F15C489"
                    "0000000A49444154789C6360000000020001E221BC330000000049454E44AE426082"
                )
            )
        sandbox_demo = self._resolve_sandbox_path("tmp/demo.png")
        sandbox_demo.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(demo_png, sandbox_demo)

        old_log = self._resolve_sandbox_path("tmp/old.log")
        old_log.parent.mkdir(parents=True, exist_ok=True)
        if not old_log.exists():
            old_log.write_text("legacy log\n", encoding="utf-8")

        facade_patch = self._resolve_sandbox_path("tmp/facade_patch.txt")
        facade_patch.parent.mkdir(parents=True, exist_ok=True)
        if not facade_patch.exists():
            facade_patch.write_text("old\n", encoding="utf-8")

        facade_edit = self._resolve_sandbox_path("tmp/facade_edit.txt")
        facade_edit.parent.mkdir(parents=True, exist_ok=True)
        if not facade_edit.exists():
            facade_edit.write_text("old\n", encoding="utf-8")

        art_id = "art_001"
        art_path = self.artifact_root / f"{art_id}.txt"
        art_path.write_text("seed artifact\n", encoding="utf-8")
        self.artifacts[art_id] = {
            "artifact_id": art_id,
            "artifact_type": "note",
            "summary": "seed artifact",
            "text": "seed artifact",
            "path": str(art_path),
            "created_at": dt.datetime.now().isoformat(timespec="seconds"),
        }

        self.subagents["sa_001"] = {
            "subagent_id": "sa_001",
            "goal": "seed helper",
            "status": "ready",
            "history": [],
            "last_output": "",
        }
        self._record_event("runtime_init", {"workspace": str(self.runtime_root)})
        self.thread_baseline_event_count = len(self.thread_events_log)

    def _to_runtime_rel(self, path: Path) -> str:
        return str(path.relative_to(self.runtime_root)).replace("\\", "/")

    def _mark_overlay_path(self, path: str | Path) -> None:
        try:
            target = path if isinstance(path, Path) else self._resolve_sandbox_path(path)
            self.overlay_paths.add(self._to_runtime_rel(target))
        except Exception:
            return

    def _json_error(self, code: str, message: str, **extra: Any) -> dict[str, Any]:
        payload = {
            "ok": False,
            "error_code": code,
            "error": message,
            "message": message,
        }
        payload.update(extra)
        return payload

    def _safe_rel(self, path: str) -> Path:
        candidate = Path(path.strip()) if path.strip() else Path(".")
        if candidate.is_absolute():
            candidate = Path(str(candidate).lstrip("/"))
        return candidate

    def _resolve_sandbox_path(self, path: str) -> Path:
        rel = self._safe_rel(path)
        resolved = (self.runtime_root / rel).resolve()
        if self.runtime_root not in [resolved, *resolved.parents]:
            raise ValueError(f"path escapes sandbox: {path}")
        return resolved

    def _resolve_repo_path(self, path: str) -> Path:
        rel = self._safe_rel(path)
        resolved = (self.repo_root / rel).resolve()
        if self.repo_root not in [resolved, *resolved.parents]:
            raise ValueError(f"path escapes repo: {path}")
        return resolved

    def _find_readable_path(self, path: str) -> tuple[Path | None, str]:
        sandbox = self._resolve_sandbox_path(path)
        if sandbox.exists() and sandbox.is_file():
            return sandbox, "sandbox"
        repo = self._resolve_repo_path(path)
        if repo.exists() and repo.is_file():
            return repo, "repo"
        return None, "none"

    def _non_utf8_tool_hint(self, logical_path: str) -> dict[str, Any]:
        ext = Path(logical_path).suffix.lower()
        image_exts = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".tiff", ".ico"}
        if ext in image_exts:
            return {
                "tool": "view_image",
                "suggested_args": {"path": logical_path},
                "instruction": (
                    "This file looks like an image and is not UTF-8 text. "
                    "Use `view_image` with the same path."
                ),
                "valid_tools": ["view_image"],
            }

        inspect_cmd = [
            "python3",
            "-c",
            (
                "import hashlib,pathlib;"
                f"p=pathlib.Path({json.dumps(logical_path)});"
                "b=p.read_bytes();"
                "print('bytes',len(b));"
                "print('sha256',hashlib.sha256(b).hexdigest())"
            ),
        ]
        return {
            "tool": "process_start",
            "suggested_args": {"command": inspect_cmd},
            "instruction": (
                "This file is non-UTF8 binary content. "
                "Use `process_start` to inspect bytes/hash or a format-specific parser."
            ),
            "valid_tools": ["process_start"],
        }

    def _prepare_mutable_file(self, path: str) -> Path:
        sandbox = self._resolve_sandbox_path(path)
        self._mark_overlay_path(sandbox)
        if sandbox.exists():
            return sandbox
        repo = self._resolve_repo_path(path)
        sandbox.parent.mkdir(parents=True, exist_ok=True)
        if repo.exists() and repo.is_file():
            shutil.copy2(repo, sandbox)
        else:
            sandbox.write_text("", encoding="utf-8")
        return sandbox

    def _inject_recovery_error_if_needed(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any] | None:
        matched_target, _ = self._match_target_or_alias(tool_name, args)
        if not matched_target:
            return None
        self.target_call_index += 1
        if self.mode == "recovery" and self.target_call_index == 1:
            return {
                "ok": False,
                "error_code": "INJECTED_TRANSIENT",
                "error": "Injected transient failure.",
                "message": "Temporary runtime failure. Retry the same operation.",
                "retryable": True,
                "suggested_args": self.case.success_args,
                "instruction": (
                    "You MUST call the same target tool again with suggested_args. "
                    "DO NOT finish the task yet."
                ),
            }
        return None

    def _tool_file_read(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        real_path, scope = self._find_readable_path(path)
        if real_path is None:
            return self._json_error(
                "FILE_NOT_FOUND",
                f"File not found: {path}",
                suggested_args={"path": "README.md"},
            )
        raw = real_path.read_bytes()
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            hint = self._non_utf8_tool_hint(path)
            return self._json_error(
                "NON_UTF8_CONTENT",
                (
                    "file_read only supports UTF-8 text files. "
                    "This file cannot be decoded as UTF-8."
                ),
                path=path,
                scope=scope,
                suffix=real_path.suffix.lower(),
                suggested_tool=hint["tool"],
                suggested_args=hint["suggested_args"],
                valid_tools=hint["valid_tools"],
                instruction=hint["instruction"],
            )
        max_chars = MAX_FILE_READ_CHARS
        truncated = len(text) > max_chars
        preview = text[:max_chars]
        return {
            "ok": True,
            "status": "ok",
            "path": path,
            "scope": scope,
            "full_content_length": len(text),
            "truncated": truncated,
            "content": preview,
            "content_preview": preview[:400],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_file_glob(self, args: dict[str, Any]) -> dict[str, Any]:
        pattern = _first_string_arg(args, ["pattern"]) or "**/*"
        matches: list[str] = []
        seen: set[str] = set()
        for root in [self.repo_root, self.runtime_root]:
            for item in root.glob(pattern):
                if not item.is_file():
                    continue
                rel = item.relative_to(root)
                key = str(rel)
                if key in seen:
                    continue
                seen.add(key)
                matches.append(key)
                if len(matches) >= 200:
                    break
            if len(matches) >= 200:
                break
        matches.sort()
        return {
            "ok": True,
            "pattern": pattern,
            "count": len(matches),
            "matches": matches,
            "sample": matches[:20],
            "evidence_nonce": self.evidence_nonce,
        }

    def _grep_with_rg_in_root(
        self, root: Path, query: str, include_glob: str, max_rows: int
    ) -> list[dict[str, Any]]:
        cmd = ["rg", "-n", "--no-heading", "--color", "never"]
        if include_glob:
            cmd.extend(["--glob", include_glob])
        cmd.extend([query, "."])
        proc = subprocess.run(
            cmd,
            cwd=str(root),
            capture_output=True,
            text=True,
            timeout=self.config.process_timeout_sec,
            check=False,
        )
        if proc.returncode not in {0, 1}:
            raise RuntimeError(proc.stderr.strip() or "rg failed")
        rows: list[dict[str, Any]] = []
        for line in proc.stdout.splitlines():
            parts = line.split(":", 2)
            if len(parts) < 3:
                continue
            try:
                line_no = int(parts[1])
            except Exception:
                line_no = 0
            rel_path = str(Path(parts[0])).replace("\\", "/")
            if rel_path.startswith("./"):
                rel_path = rel_path[2:]
            rows.append(
                {
                    "path": rel_path,
                    "line": line_no,
                    "text": _truncate_text(parts[2], MAX_GREP_LINE_CHARS),
                }
            )
            if len(rows) >= max_rows:
                break
        return rows

    def _grep_with_rg(self, query: str, include_glob: str, max_rows: int) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        for source, root in (("sandbox", self.runtime_root), ("repo", self.repo_root)):
            part = self._grep_with_rg_in_root(root, query, include_glob, max_rows)
            for row in part:
                rel = safe_text(row.get("path"))
                # If a file was mutated in sandbox, repository view must be hidden for consistency.
                if source == "repo" and rel in self.overlay_paths:
                    continue
                rows.append(row)
                if len(rows) >= max_rows:
                    return rows
        return rows

    def _grep_fallback_in_root(
        self, root: Path, query: str, include_glob: str, max_rows: int
    ) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        skip_dirs = {".git", "target", "node_modules", ".venv", "venv", "__pycache__", ".artifacts"}
        for base, dirs, files in os.walk(root):
            dirs[:] = [d for d in dirs if d not in skip_dirs]
            for filename in files:
                fp = Path(base) / filename
                rel = str(fp.relative_to(root)).replace("\\", "/")
                if include_glob and not fnmatch.fnmatch(rel, include_glob):
                    continue
                try:
                    text = fp.read_text(encoding="utf-8", errors="ignore")
                except Exception:
                    continue
                if query not in text:
                    continue
                for idx, line in enumerate(text.splitlines(), start=1):
                    if query in line:
                        rows.append(
                            {
                                "path": rel,
                                "line": idx,
                                "text": _truncate_text(line, MAX_GREP_LINE_CHARS),
                            }
                        )
                        if len(rows) >= max_rows:
                            return rows
        return rows

    def _grep_fallback(self, query: str, include_glob: str, max_rows: int) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        for source, root in (("sandbox", self.runtime_root), ("repo", self.repo_root)):
            part = self._grep_fallback_in_root(root, query, include_glob, max_rows)
            for row in part:
                rel = safe_text(row.get("path"))
                if source == "repo" and rel in self.overlay_paths:
                    continue
                rows.append(row)
                if len(rows) >= max_rows:
                    return rows
        return rows

    def _tool_file_grep(self, args: dict[str, Any]) -> dict[str, Any]:
        query = _first_string_arg(args, ["query", "q", "pattern", "symbol"])
        if not query:
            return self._json_error("ARGS_MISMATCH", "`query` is required.")
        include_glob = _first_string_arg(args, ["include_glob", "glob", "path"])
        if include_glob.startswith("/"):
            include_glob = include_glob.lstrip("/")
        max_rows = MAX_GREP_MATCHES + 1
        try:
            rows = self._grep_with_rg(query, include_glob, max_rows)
        except Exception:
            rows = self._grep_fallback(query, include_glob, max_rows)
        truncated = len(rows) > MAX_GREP_MATCHES
        matches = rows[:MAX_GREP_MATCHES]
        return {
            "ok": True,
            "query": query,
            "include_glob": include_glob or None,
            "count": len(matches),
            "matches": matches,
            "truncated": truncated,
            "note": (
                "Search results are intentionally capped for model stability. "
                "Use include_glob to narrow scope for higher precision."
            ),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_repo_index(self, _: dict[str, Any]) -> dict[str, Any]:
        total = 0
        ext_counts: dict[str, int] = {}
        skip_dirs = {".git", "target", "node_modules", ".venv", "venv", "__pycache__"}
        for root, dirs, files in os.walk(self.repo_root):
            dirs[:] = [d for d in dirs if d not in skip_dirs]
            for filename in files:
                total += 1
                ext = Path(filename).suffix.lower() or "<none>"
                ext_counts[ext] = ext_counts.get(ext, 0) + 1
        top = sorted(ext_counts.items(), key=lambda x: (-x[1], x[0]))[:20]
        return {
            "ok": True,
            "file_count": total,
            "top_extensions": [{"ext": ext, "count": count} for ext, count in top],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_repo_symbols(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        real_path, scope = self._find_readable_path(path)
        if real_path is None:
            return self._json_error("FILE_NOT_FOUND", f"File not found: {path}")
        pattern = re.compile(
            r"^\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|trait|class|def)\s+([A-Za-z_][A-Za-z0-9_]*)"
        )
        symbols: list[dict[str, Any]] = []
        for idx, line in enumerate(real_path.read_text(encoding="utf-8", errors="ignore").splitlines(), start=1):
            match = pattern.search(line)
            if match:
                symbols.append({"line": idx, "name": match.group(1), "raw": _truncate_text(line.strip(), 160)})
            if len(symbols) >= MAX_REPO_SYMBOLS:
                break
        return {
            "ok": True,
            "path": path,
            "scope": scope,
            "symbol_count": len(symbols),
            "symbols": symbols,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_repo_goto_definition(self, args: dict[str, Any]) -> dict[str, Any]:
        symbol = _first_string_arg(args, ["symbol", "query", "name"])
        if not symbol:
            return self._json_error("ARGS_MISMATCH", "`symbol` is required.")
        symbol_tail = symbol.split("::")[-1].strip()
        if not symbol_tail:
            return self._json_error("ARGS_MISMATCH", "`symbol` is required.")
        include_glob = _first_string_arg(args, ["include_glob", "glob"]) or "crates/**/*.rs"
        path_hint = _first_string_arg(args, ["path"])
        max_results_raw = args.get("max_results")
        try:
            max_results = int(max_results_raw) if max_results_raw is not None else 20
        except Exception:
            max_results = 20
        max_results = max(1, min(max_results, 80))

        decl_pattern = re.compile(
            rf"^\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|trait|type|const|static)\s+{re.escape(symbol_tail)}\b"
        )
        definitions: list[dict[str, Any]] = []
        skip_dirs = {".git", "target", "node_modules", ".venv", "venv", "__pycache__", ".artifacts"}
        for source, root in (("sandbox", self.runtime_root), ("repo", self.repo_root)):
            for base, dirs, files in os.walk(root):
                dirs[:] = [d for d in dirs if d not in skip_dirs]
                for filename in files:
                    fp = Path(base) / filename
                    rel = str(fp.relative_to(root)).replace("\\", "/")
                    if include_glob and not fnmatch.fnmatch(rel, include_glob):
                        continue
                    if source == "repo" and rel in self.overlay_paths:
                        continue
                    try:
                        lines = fp.read_text(encoding="utf-8", errors="ignore").splitlines()
                    except Exception:
                        continue
                    for idx, line in enumerate(lines, start=1):
                        if not decl_pattern.search(line):
                            continue
                        score = 0
                        if line.strip().startswith("pub "):
                            score += 8
                        if f"fn {symbol_tail}" in line:
                            score += 6
                        if path_hint and (rel == path_hint or rel.endswith(path_hint) or path_hint in rel):
                            score += 30
                        definitions.append(
                            {
                                "path": rel,
                                "line": idx,
                                "text": _truncate_text(line.strip(), 180),
                                "_score": score,
                            }
                        )
                        if len(definitions) >= 400:
                            break
                    if len(definitions) >= 400:
                        break
                if len(definitions) >= 400:
                    break
            if len(definitions) >= 400:
                break

        definitions.sort(key=lambda item: (-int(item.get("_score", 0)), safe_text(item.get("path")), int(item.get("line", 0))))
        selected = []
        for item in definitions[:max_results]:
            selected.append(
                {
                    "path": item.get("path"),
                    "line": item.get("line"),
                    "text": item.get("text"),
                }
            )

        return {
            "ok": True,
            "symbol": symbol,
            "symbol_tail": symbol_tail,
            "include_glob": include_glob,
            "path_hint": path_hint or None,
            "definition_count": len(selected),
            "resolved": len(selected) > 0,
            "definitions": selected,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_repo_find_references(self, args: dict[str, Any]) -> dict[str, Any]:
        symbol = _first_string_arg(args, ["symbol", "query", "name"])
        if not symbol:
            return self._json_error("ARGS_MISMATCH", "`symbol` is required.")
        symbol_tail = symbol.split("::")[-1].strip()
        if not symbol_tail:
            return self._json_error("ARGS_MISMATCH", "`symbol` is required.")
        include_glob = _first_string_arg(args, ["include_glob", "glob"]) or "crates/**/*.rs"
        path_hint = _first_string_arg(args, ["path"])
        max_matches_raw = args.get("max_matches")
        try:
            max_matches = int(max_matches_raw) if max_matches_raw is not None else MAX_GREP_MATCHES
        except Exception:
            max_matches = MAX_GREP_MATCHES
        max_matches = max(1, min(max_matches, 120))

        try:
            rows = self._grep_with_rg(symbol_tail, include_glob, max_matches + 1)
        except Exception:
            rows = self._grep_fallback(symbol_tail, include_glob, max_matches + 1)

        if path_hint:
            preferred = [
                row
                for row in rows
                if (
                    safe_text(row.get("path")) == path_hint
                    or safe_text(row.get("path")).endswith(path_hint)
                    or path_hint in safe_text(row.get("path"))
                )
            ]
            if preferred:
                rows = preferred

        truncated = len(rows) > max_matches
        matches = rows[:max_matches]
        return {
            "ok": True,
            "symbol": symbol,
            "symbol_tail": symbol_tail,
            "include_glob": include_glob,
            "path_hint": path_hint or None,
            "count": len(matches),
            "references": matches,
            "truncated": truncated,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_file_write(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        content = safe_text(args.get("content"))
        if not content and "text" in args:
            content = safe_text(args.get("text"))
        target = self._resolve_sandbox_path(path)
        self._mark_overlay_path(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
        return {
            "ok": True,
            "path": str(target.relative_to(self.runtime_root)),
            "bytes_written": len(content.encode("utf-8")),
            "content_preview": content[:300],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_file_edit(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        old_text = safe_text(args.get("old_text"))
        new_text = safe_text(args.get("new_text"))
        if not old_text and isinstance(args.get("edits"), list) and args["edits"]:
            first = args["edits"][0]
            if isinstance(first, dict):
                old_text = safe_text(first.get("old") if first.get("old") is not None else first.get("old_text"))
                new_text = safe_text(first.get("new") if first.get("new") is not None else first.get("new_text"))
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        if not old_text:
            return self._json_error("ARGS_MISMATCH", "`old_text` is required.")
        target = self._prepare_mutable_file(path)
        before = target.read_text(encoding="utf-8", errors="replace")
        if old_text not in before:
            return self._json_error(
                "OLD_TEXT_NOT_FOUND",
                "old_text not found in file.",
                suggested_args={"path": path, "old_text": old_text, "new_text": new_text},
                preview=before[:500],
            )
        after = before.replace(old_text, new_text, 1)
        target.write_text(after, encoding="utf-8")
        return {
            "ok": True,
            "path": str(target.relative_to(self.runtime_root)),
            "replacements": 1,
            "before_hash": hashlib.sha256(before.encode("utf-8", errors="ignore")).hexdigest()[:16],
            "after_hash": hashlib.sha256(after.encode("utf-8", errors="ignore")).hexdigest()[:16],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_file_patch(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        patch = safe_text(args.get("patch"))
        if not path or not patch:
            return self._json_error("ARGS_MISMATCH", "`path` and `patch` are required.")
        target = self._prepare_mutable_file(path)
        text = target.read_text(encoding="utf-8", errors="replace")
        minus_lines: list[str] = []
        plus_lines: list[str] = []
        for line in patch.splitlines():
            if line.startswith(("---", "+++", "@@")):
                continue
            if line.startswith("-"):
                minus_lines.append(line[1:])
            elif line.startswith("+"):
                plus_lines.append(line[1:])
        old_block = "\n".join(minus_lines).strip("\n")
        new_block = "\n".join(plus_lines).strip("\n")
        if old_block and old_block in text:
            after = text.replace(old_block, new_block, 1)
            apply_mode = "replace"
        else:
            append_part = f"\n{new_block}\n" if new_block else "\n"
            after = text + append_part
            apply_mode = "append_fallback"
        target.write_text(after, encoding="utf-8")
        return {
            "ok": True,
            "path": str(target.relative_to(self.runtime_root)),
            "apply_mode": apply_mode,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_file_delete(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        target = self._resolve_sandbox_path(path)
        self._mark_overlay_path(target)
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("auto-generated for delete benchmark\n", encoding="utf-8")
        if target.is_dir():
            shutil.rmtree(target)
        else:
            target.unlink()
        return {
            "ok": True,
            "path": str(target.relative_to(self.runtime_root)),
            "deleted": True,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_fs_mkdir(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        target = self._resolve_sandbox_path(path)
        target.mkdir(parents=True, exist_ok=True)
        return {
            "ok": True,
            "path": str(target.relative_to(self.runtime_root)),
            "created": True,
            "evidence_nonce": self.evidence_nonce,
        }

    def _normalize_command(self, args: dict[str, Any]) -> list[str]:
        for key in ["argv", "command", "cmd", "cmdline"]:
            value = args.get(key)
            if isinstance(value, list):
                cmd = [safe_text(x) for x in value if safe_text(x)]
                if cmd:
                    return cmd
            elif isinstance(value, str):
                cmd = shlex.split(value)
                if cmd:
                    return cmd
        return []

    def _is_command_allowed(self, cmd: list[str]) -> bool:
        if not cmd:
            return False
        head = Path(cmd[0]).name.lower()
        return head in self.config.process_allowlist

    def _tool_process_start(self, args: dict[str, Any]) -> dict[str, Any]:
        command = self._normalize_command(args)
        if not command:
            return self._json_error("ARGS_MISMATCH", "`command` must be a non-empty list/string.")
        if not self._is_command_allowed(command):
            return self._json_error(
                "PERMISSION_DENIED",
                f"Command is not in allowlist: {command[0]}",
            )
        try:
            proc = subprocess.run(
                command,
                cwd=str(self.repo_root),
                capture_output=True,
                text=True,
                timeout=self.config.process_timeout_sec,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return self._json_error("PROCESS_TIMEOUT", "Process execution timed out.")

        process_id = f"proc_{secrets.token_hex(4)}"
        self.processes[process_id] = {
            "process_id": process_id,
            "command": command,
            "running": False,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "pid": None,
        }
        return {
            "ok": proc.returncode == 0,
            "process_id": process_id,
            "command": command,
            "returncode": proc.returncode,
            "stdout": (proc.stdout or "")[:2000],
            "stderr": (proc.stderr or "")[:1000],
            "evidence_nonce": self.evidence_nonce,
        }

    def _ensure_process_fixture(self, process_id: str) -> dict[str, Any]:
        record = self.processes.get(process_id)
        if record is not None:
            return record
        cmd = ["python3", "--version"]
        proc = subprocess.run(
            cmd,
            cwd=str(self.repo_root),
            capture_output=True,
            text=True,
            timeout=self.config.process_timeout_sec,
            check=False,
        )
        record = {
            "process_id": process_id,
            "command": cmd,
            "running": False,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "pid": None,
        }
        self.processes[process_id] = record
        return record

    def _tool_process_inspect(self, args: dict[str, Any]) -> dict[str, Any]:
        process_id = _first_string_arg(args, ["process_id"]) or "proc_001"
        record = self._ensure_process_fixture(process_id)
        popen = record.get("popen")
        running = bool(isinstance(popen, subprocess.Popen) and popen.poll() is None)
        return {
            "ok": True,
            "process_id": process_id,
            "running": running,
            "pid": popen.pid if isinstance(popen, subprocess.Popen) else record.get("pid"),
            "returncode": popen.poll() if isinstance(popen, subprocess.Popen) else record.get("returncode"),
            "command": record.get("command"),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_process_tail(self, args: dict[str, Any]) -> dict[str, Any]:
        process_id = _first_string_arg(args, ["process_id"]) or "proc_001"
        lines = args.get("lines")
        try:
            n = int(lines) if lines is not None else 20
        except Exception:
            n = 20
        n = max(1, min(n, 200))
        record = self._ensure_process_fixture(process_id)
        content = f"{record.get('stdout', '')}\n{record.get('stderr', '')}".strip()
        out_lines = content.splitlines()
        return {
            "ok": True,
            "process_id": process_id,
            "lines": out_lines[-n:],
            "line_count": len(out_lines[-n:]),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_process_follow(self, args: dict[str, Any]) -> dict[str, Any]:
        process_id = _first_string_arg(args, ["process_id"]) or "proc_001"
        max_chunks = args.get("max_chunks")
        try:
            chunks = int(max_chunks) if max_chunks is not None else 2
        except Exception:
            chunks = 2
        chunks = max(1, min(chunks, 20))
        record = self._ensure_process_fixture(process_id)
        content = f"{record.get('stdout', '')}\n{record.get('stderr', '')}".strip()
        lines = content.splitlines()[:chunks]
        return {
            "ok": True,
            "process_id": process_id,
            "chunks": lines,
            "chunk_count": len(lines),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_process_kill(self, args: dict[str, Any]) -> dict[str, Any]:
        process_id = _first_string_arg(args, ["process_id"]) or "proc_001"
        record = self.processes.get(process_id)
        popen = record.get("popen") if isinstance(record, dict) else None
        if not isinstance(popen, subprocess.Popen) or popen.poll() is not None:
            popen = subprocess.Popen(
                ["python3", "-c", "import time; print('killable'); time.sleep(30)"],
                cwd=str(self.repo_root),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            record = {
                "process_id": process_id,
                "command": ["python3", "-c", "import time; print('killable'); time.sleep(30)"],
                "running": True,
                "pid": popen.pid,
                "popen": popen,
                "stdout": "",
                "stderr": "",
                "returncode": None,
            }
            self.processes[process_id] = record

        popen.terminate()
        try:
            stdout, _ = popen.communicate(timeout=2.0)
        except Exception:
            popen.kill()
            stdout, _ = popen.communicate(timeout=2.0)
        record["running"] = False
        record["returncode"] = popen.returncode
        record["stdout"] = (record.get("stdout") or "") + (stdout or "")
        return {
            "ok": True,
            "process_id": process_id,
            "killed": True,
            "returncode": popen.returncode,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_artifact_write(self, args: dict[str, Any]) -> dict[str, Any]:
        artifact_id = _first_string_arg(args, ["artifact_id"]) or f"art_{secrets.token_hex(4)}"
        artifact_type = _first_string_arg(args, ["artifact_type"]) or "note"
        summary = _first_string_arg(args, ["summary"]) or "artifact"
        text = safe_text(args.get("text"))
        out_path = self.artifact_root / f"{artifact_id}.txt"
        out_path.write_text(text, encoding="utf-8")
        meta = {
            "artifact_id": artifact_id,
            "artifact_type": artifact_type,
            "summary": summary,
            "text": text,
            "path": str(out_path),
            "created_at": dt.datetime.now().isoformat(timespec="seconds"),
        }
        self.artifacts[artifact_id] = meta
        return {
            "ok": True,
            "artifact_id": artifact_id,
            "artifact_type": artifact_type,
            "summary": summary,
            "bytes": len(text.encode("utf-8")),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_artifact_list(self, args: dict[str, Any]) -> dict[str, Any]:
        limit_raw = args.get("limit")
        try:
            limit = int(limit_raw) if limit_raw is not None else 10
        except Exception:
            limit = 10
        limit = max(1, min(limit, 100))
        items = sorted(self.artifacts.values(), key=lambda x: x["artifact_id"])[:limit]
        return {
            "ok": True,
            "limit": limit,
            "count": len(items),
            "items": [
                {
                    "artifact_id": x["artifact_id"],
                    "artifact_type": x["artifact_type"],
                    "summary": x["summary"],
                }
                for x in items
            ],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_artifact_read(self, args: dict[str, Any]) -> dict[str, Any]:
        artifact_id = _first_string_arg(args, ["artifact_id"]) or "art_001"
        meta = self.artifacts.get(artifact_id)
        if meta is None:
            return self._json_error("NOT_FOUND", f"Artifact not found: {artifact_id}")
        text = safe_text(meta.get("text"))
        return {
            "ok": True,
            "artifact_id": artifact_id,
            "artifact_type": meta.get("artifact_type"),
            "summary": meta.get("summary"),
            "text": text,
            "text_preview": text[:300],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_artifact_delete(self, args: dict[str, Any]) -> dict[str, Any]:
        artifact_id = _first_string_arg(args, ["artifact_id"]) or "art_001"
        meta = self.artifacts.pop(artifact_id, None)
        if meta is None:
            return self._json_error("NOT_FOUND", f"Artifact not found: {artifact_id}")
        path = Path(str(meta.get("path")))
        if path.exists():
            path.unlink()
        return {"ok": True, "artifact_id": artifact_id, "deleted": True, "evidence_nonce": self.evidence_nonce}

    def _tool_thread_state(self, _: dict[str, Any]) -> dict[str, Any]:
        return {
            "ok": True,
            "thread_id": f"thread_{self.case.case_id}",
            "workspace_root": str(self.runtime_root),
            "event_count": len(self.thread_events_log),
            "tool_calls": self.total_tool_calls,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_thread_diff(self, _: dict[str, Any]) -> dict[str, Any]:
        delta = len(self.thread_events_log) - self.thread_baseline_event_count
        return {
            "ok": True,
            "event_delta_since_start": delta,
            "tool_call_delta": self.total_tool_calls,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_thread_events(self, args: dict[str, Any]) -> dict[str, Any]:
        max_events = args.get("max_events")
        try:
            n = int(max_events) if max_events is not None else 20
        except Exception:
            n = 20
        n = max(1, min(n, 200))
        return {
            "ok": True,
            "events": self.thread_events_log[-n:],
            "count": len(self.thread_events_log[-n:]),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_thread_usage(self, _: dict[str, Any]) -> dict[str, Any]:
        return {
            "ok": True,
            "tool_calls_total": self.total_tool_calls,
            "event_count_total": len(self.thread_events_log),
            "approx_input_tokens": 0,
            "approx_output_tokens": 0,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_thread_hook_run(self, args: dict[str, Any]) -> dict[str, Any]:
        hook = _first_string_arg(args, ["hook"]) or "default"
        self._record_event("thread_hook_run", {"hook": hook})
        return {"ok": True, "hook": hook, "status": "executed", "evidence_nonce": self.evidence_nonce}

    def _tool_update_plan(self, args: dict[str, Any]) -> dict[str, Any]:
        plan = args.get("plan")
        if isinstance(plan, dict):
            plan = [plan]
        if not isinstance(plan, list):
            return self._json_error("ARGS_MISMATCH", "`plan` must be a list.")
        cleaned: list[dict[str, Any]] = []
        for item in plan:
            if not isinstance(item, dict):
                continue
            step = safe_text(item.get("step")).strip()
            if not step:
                step = safe_text(item.get("description")).strip()
            status = safe_text(item.get("status")).strip()
            if step:
                cleaned.append({"step": step, "status": status or "pending"})
        self.thread_plan = cleaned
        self._record_event("plan_update", {"steps": len(cleaned)})
        return {"ok": True, "plan_size": len(cleaned), "plan": cleaned, "evidence_nonce": self.evidence_nonce}

    def _tool_request_user_input(self, args: dict[str, Any]) -> dict[str, Any]:
        questions = args.get("questions")
        if isinstance(questions, dict):
            questions = [questions]
        if isinstance(questions, str) and questions.strip():
            questions = [
                {
                    "header": "Input",
                    "id": "q1",
                    "question": questions.strip(),
                    "options": [
                        {"label": "Yes", "description": "Continue"},
                        {"label": "No", "description": "Stop"},
                    ],
                }
            ]
        if isinstance(questions, list):
            normalized_questions: list[dict[str, Any]] = []
            for idx, q in enumerate(questions):
                if isinstance(q, dict):
                    normalized_questions.append(dict(q))
                elif isinstance(q, str) and q.strip():
                    normalized_questions.append(
                        {
                            "header": "Input",
                            "id": f"q{idx+1}",
                            "question": q.strip(),
                            "options": [
                                {"label": "Yes", "description": "Continue"},
                                {"label": "No", "description": "Stop"},
                            ],
                        }
                    )
            questions = normalized_questions
        if not isinstance(questions, list):
            return self._json_error("ARGS_MISMATCH", "`questions` must be a list.")
        ids: list[str] = []
        for q in questions:
            if isinstance(q, dict):
                qid = safe_text(q.get("id")).strip()
                if qid:
                    ids.append(qid)
        self._record_event("request_user_input", {"count": len(ids)})
        return {
            "ok": True,
            "requested_question_ids": ids,
            "status": "input_requested",
            "note": "Benchmark runtime cannot interactively collect user input.",
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_ask_user(self, args: dict[str, Any]) -> dict[str, Any]:
        question = _first_string_arg(args, ["question", "prompt", "input"])
        if not question:
            return self._json_error("ARGS_MISMATCH", "`question` is required.")
        purpose = _first_string_arg(args, ["purpose", "objective", "reason"])
        self._record_event(
            "ask_user",
            {
                "question": question,
                "purpose": purpose or "confirm and continue task completion",
            },
        )
        return {
            "ok": True,
            "status": "user_query_sent",
            "question": question,
            "purpose": purpose or "confirm and continue task completion",
            "note": "Benchmark runtime will auto-inject a simulated user reply.",
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_agent_spawn(self, args: dict[str, Any]) -> dict[str, Any]:
        goal = _first_string_arg(args, ["goal", "input"])
        if not goal and isinstance(args.get("tasks"), list):
            for item in args["tasks"]:
                if isinstance(item, dict):
                    goal = _first_string_arg(item, ["input", "goal", "title"])
                    if goal:
                        break
        if not goal:
            goal = "general task"
        sid = f"sa_{secrets.token_hex(3)}"
        self.subagents[sid] = {
            "subagent_id": sid,
            "goal": goal,
            "status": "ready",
            "history": [],
            "last_output": "",
        }
        self._record_event("subagent_spawn", {"subagent_id": sid})
        return {"ok": True, "subagent_id": sid, "goal": goal, "evidence_nonce": self.evidence_nonce}

    def _ensure_subagent(self, subagent_id: str) -> dict[str, Any]:
        if subagent_id not in self.subagents:
            self.subagents[subagent_id] = {
                "subagent_id": subagent_id,
                "goal": "auto-created",
                "status": "ready",
                "history": [],
                "last_output": "",
            }
        return self.subagents[subagent_id]

    def _tool_subagent_send_input(self, args: dict[str, Any]) -> dict[str, Any]:
        subagent_id = _first_string_arg(args, ["subagent_id", "id"]) or "sa_001"
        text = safe_text(args.get("input"))
        if not text and "message" in args:
            text = safe_text(args.get("message"))
        agent = self._ensure_subagent(subagent_id)
        output = f"subagent({subagent_id}) processed: {text[:120]}"
        agent["history"].append({"input": text, "output": output})
        agent["last_output"] = output
        agent["status"] = "responded"
        self._record_event("subagent_send_input", {"subagent_id": subagent_id})
        return {
            "ok": True,
            "subagent_id": subagent_id,
            "status": agent["status"],
            "output": output,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_subagent_wait(self, args: dict[str, Any]) -> dict[str, Any]:
        subagent_id = _first_string_arg(args, ["subagent_id", "id"])
        if not subagent_id and isinstance(args.get("ids"), list):
            subagent_id = _first_string_arg({"id": args.get("ids")[0] if args.get("ids") else ""}, ["id"])
        if not subagent_id:
            subagent_id = "sa_001"
        agent = self._ensure_subagent(subagent_id)
        self._record_event("subagent_wait", {"subagent_id": subagent_id})
        return {
            "ok": True,
            "subagent_id": subagent_id,
            "status": agent["status"],
            "last_output": agent["last_output"],
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_subagent_close(self, args: dict[str, Any]) -> dict[str, Any]:
        subagent_id = _first_string_arg(args, ["subagent_id", "id"]) or "sa_001"
        agent = self._ensure_subagent(subagent_id)
        agent["status"] = "closed"
        self._record_event("subagent_close", {"subagent_id": subagent_id})
        return {"ok": True, "subagent_id": subagent_id, "status": "closed", "evidence_nonce": self.evidence_nonce}

    def _tool_mcp_list_servers(self, _: dict[str, Any]) -> dict[str, Any]:
        return {
            "ok": True,
            "servers": sorted(self.mcp_servers.keys()),
            "count": len(self.mcp_servers),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_mcp_list_tools(self, args: dict[str, Any]) -> dict[str, Any]:
        server = _first_string_arg(args, ["server"]) or "default"
        if server == "mcp":
            server = "default"
        data = self.mcp_servers.get(server)
        if data is None:
            return self._json_error("MCP_SERVER_NOT_FOUND", f"Unknown MCP server: {server}")
        tools = [{"name": name, **meta} for name, meta in data["tools"].items()]
        return {"ok": True, "server": server, "tools": tools, "count": len(tools), "evidence_nonce": self.evidence_nonce}

    def _tool_mcp_list_resources(self, args: dict[str, Any]) -> dict[str, Any]:
        server = _first_string_arg(args, ["server"]) or "default"
        if server == "mcp":
            server = "default"
        data = self.mcp_servers.get(server)
        if data is None:
            return self._json_error("MCP_SERVER_NOT_FOUND", f"Unknown MCP server: {server}")
        resources = list(data.get("resources", []))
        return {
            "ok": True,
            "server": server,
            "resources": resources,
            "count": len(resources),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_mcp_call(self, args: dict[str, Any]) -> dict[str, Any]:
        normalized = self._normalize_mcp_args(args)
        server = _first_string_arg(normalized, ["server"]) or "default"
        tool = _first_string_arg(normalized, ["tool"])
        arguments = normalized.get("arguments")
        if not tool:
            return self._json_error("ARGS_MISMATCH", "`tool` is required for mcp_call.")
        if not isinstance(arguments, dict):
            arguments = {}
        if server == "mcp":
            server = "default"
        data = self.mcp_servers.get(server)
        if data is None:
            return self._json_error("MCP_SERVER_NOT_FOUND", f"Unknown MCP server: {server}")
        if tool not in data["tools"]:
            return self._json_error("MCP_TOOL_NOT_FOUND", f"Unknown MCP tool: {tool}")
        if tool == "echo":
            text = safe_text(arguments.get("text"))
            return {
                "ok": True,
                "server": server,
                "tool": tool,
                "arguments": arguments,
                "result": {"text": text},
                "evidence_nonce": self.evidence_nonce,
            }
        return self._json_error("MCP_TOOL_UNSUPPORTED", f"MCP tool not implemented in benchmark runtime: {tool}")

    def _tool_web_search(self, args: dict[str, Any]) -> dict[str, Any]:
        query = _first_string_arg(args, ["q", "query"])
        if not query:
            return self._json_error("ARGS_MISMATCH", "`q` (or `query`) is required.")
        url = "https://api.duckduckgo.com/"
        payload: dict[str, Any] | None = None
        errors: list[str] = []
        try:
            resp = requests.get(
                url,
                params={"q": query, "format": "json", "no_html": 1, "skip_disambig": 1},
                timeout=self.config.web_timeout_sec,
            )
            resp.raise_for_status()
            payload = resp.json()
        except Exception as exc:
            errors.append(f"duckduckgo: {exc}")
            payload = None

        if payload is None:
            # Fallback: Wikipedia opensearch is often reachable when generic search APIs are blocked.
            try:
                resp = requests.get(
                    "https://en.wikipedia.org/w/api.php",
                    params={
                        "action": "opensearch",
                        "search": query,
                        "limit": 8,
                        "namespace": 0,
                        "format": "json",
                    },
                    timeout=self.config.web_timeout_sec,
                )
                resp.raise_for_status()
                data = resp.json()
                if isinstance(data, list) and len(data) >= 4:
                    titles = data[1] if isinstance(data[1], list) else []
                    snippets = data[2] if isinstance(data[2], list) else []
                    urls = data[3] if isinstance(data[3], list) else []
                    results = []
                    for title, snippet, url_item in zip(titles, snippets, urls):
                        results.append(
                            {
                                "title": _truncate_text(safe_text(title), 120),
                                "url": safe_text(url_item),
                                "snippet": _truncate_text(safe_text(snippet), MAX_WEB_SNIPPET_CHARS),
                            }
                        )
                    return {
                        "ok": True,
                        "query": query,
                        "source": "wikipedia_opensearch",
                        "results": results[:MAX_WEB_RESULTS],
                        "count": len(results[:MAX_WEB_RESULTS]),
                        "evidence_nonce": self.evidence_nonce,
                    }
            except Exception as exc:
                errors.append(f"wikipedia: {exc}")
                try:
                    resp = requests.get("https://example.com", timeout=self.config.web_timeout_sec)
                    resp.raise_for_status()
                    text = resp.text
                    title_match = re.search(
                        r"<title[^>]*>(.*?)</title>",
                        text,
                        flags=re.IGNORECASE | re.DOTALL,
                    )
                    title = title_match.group(1).strip() if title_match else "Example Domain"
                    return {
                        "ok": True,
                        "query": query,
                        "source": "example_fallback",
                        "results": [
                            {
                                "title": title,
                                "url": "https://example.com",
                                "snippet": _truncate_text(
                                    f"Fallback result because search endpoints timed out. query={query}",
                                    MAX_WEB_SNIPPET_CHARS,
                                ),
                            }
                        ],
                        "count": 1,
                        "evidence_nonce": self.evidence_nonce,
                    }
                except Exception as fallback_exc:
                    errors.append(f"example: {fallback_exc}")
                    return self._json_error("WEB_SEARCH_FAILED", "; ".join(errors))

        results: list[dict[str, Any]] = []
        abstract = safe_text(payload.get("AbstractText")).strip()
        abstract_url = safe_text(payload.get("AbstractURL")).strip()
        if abstract:
            results.append(
                {
                    "title": "Abstract",
                    "url": abstract_url,
                    "snippet": _truncate_text(abstract, MAX_WEB_SNIPPET_CHARS),
                }
            )
        related = payload.get("RelatedTopics")
        if isinstance(related, list):
            for item in related:
                if len(results) >= MAX_WEB_RESULTS:
                    break
                if isinstance(item, dict) and isinstance(item.get("Text"), str):
                    results.append(
                        {
                            "title": _truncate_text(safe_text(item.get("FirstURL")), 120),
                            "url": safe_text(item.get("FirstURL")),
                            "snippet": _truncate_text(safe_text(item["Text"]), MAX_WEB_SNIPPET_CHARS),
                        }
                    )
        return {
            "ok": True,
            "query": query,
            "results": results,
            "count": len(results),
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_webfetch(self, args: dict[str, Any]) -> dict[str, Any]:
        url = _first_string_arg(args, ["url"])
        if not url:
            return self._json_error("ARGS_MISMATCH", "`url` is required.")
        if not (url.startswith("http://") or url.startswith("https://")):
            return self._json_error("INVALID_URL", "Only http(s) URLs are allowed.")
        try:
            resp = requests.get(url, timeout=self.config.web_timeout_sec)
        except Exception as exc:
            return self._json_error("WEBFETCH_FAILED", f"webfetch request failed: {exc}")
        raw = resp.content[: self.config.web_max_bytes]
        text = raw.decode("utf-8", errors="replace")
        title_match = re.search(r"<title[^>]*>(.*?)</title>", text, flags=re.IGNORECASE | re.DOTALL)
        title = title_match.group(1).strip() if title_match else ""
        body = re.sub(r"(?is)<script[^>]*>.*?</script>", " ", text)
        body = re.sub(r"(?is)<style[^>]*>.*?</style>", " ", body)
        body = re.sub(r"(?s)<[^>]+>", " ", body)
        body = re.sub(r"\s+", " ", body).strip()
        preview = _truncate_text(body or title, 1200)
        return {
            "ok": True,
            "url": resp.url,
            "status_code": resp.status_code,
            "content_bytes": len(raw),
            "content_type": safe_text(resp.headers.get("content-type")),
            "title": title,
            "content_preview": preview,
            "evidence_nonce": self.evidence_nonce,
        }

    def _tool_view_image(self, args: dict[str, Any]) -> dict[str, Any]:
        path = _first_string_arg(args, ["path"])
        if not path:
            return self._json_error("ARGS_MISMATCH", "`path` is required.")
        candidate = Path(path)
        if not candidate.is_absolute():
            readable, _ = self._find_readable_path(path)
            if readable is None:
                candidate = self._resolve_sandbox_path(path)
            else:
                candidate = readable
        if not candidate.exists() or not candidate.is_file():
            return self._json_error("FILE_NOT_FOUND", f"Image not found: {path}")
        raw = candidate.read_bytes()
        fmt = "unknown"
        if raw.startswith(b"\x89PNG\r\n\x1a\n"):
            fmt = "png"
        elif raw.startswith(b"\xff\xd8\xff"):
            fmt = "jpeg"
        return {
            "ok": True,
            "path": str(candidate),
            "format": fmt,
            "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "evidence_nonce": self.evidence_nonce,
        }

    def _merge_facade_args(self, args: dict[str, Any]) -> dict[str, Any]:
        return dict(args)

    def _reject_nested_facade_args(self, facade_tool: str, payload: dict[str, Any]) -> dict[str, Any] | None:
        nested = payload.get("args")
        if isinstance(nested, dict):
            return self._json_error(
                "ARGS_MISMATCH",
                f"{facade_tool} parameters must be root-level fields. `args` wrapper is not allowed.",
                suggested_args=self.case.success_args,
                instruction="Call the same facade again with flat root-level parameters.",
            )
        return None

    def _facade_help(self, facade_tool: str, supported_ops: list[str]) -> dict[str, Any]:
        return self._json_error(
            "FACADE_HELP",
            f"{facade_tool} op=help returns guidance only; call a concrete op to execute.",
            facade_tool=facade_tool,
            supported_ops=supported_ops,
            suggested_args=self.case.success_args,
        )

    def _tool_workspace(self, args: dict[str, Any]) -> dict[str, Any]:
        payload = self._merge_facade_args(args)
        nested_err = self._reject_nested_facade_args("workspace", payload)
        if nested_err is not None:
            return nested_err
        op = _first_string_arg(payload, ["op"]).lower()
        if op == "help":
            return self._facade_help(
                "workspace",
                [
                    "read",
                    "glob",
                    "grep",
                    "repo_search",
                    "repo_index",
                    "repo_symbols",
                    "repo_goto_definition",
                    "repo_find_references",
                    "write",
                    "patch",
                    "edit",
                    "delete",
                    "mkdir",
                ],
            )
        if op == "read":
            return self._tool_file_read(payload)
        if op == "glob":
            return self._tool_file_glob(payload)
        if op == "grep":
            return self._tool_file_grep(payload)
        if op == "repo_search":
            return self._tool_file_grep(payload)
        if op == "repo_index":
            return self._tool_repo_index(payload)
        if op == "repo_symbols":
            return self._tool_repo_symbols(payload)
        if op == "repo_goto_definition":
            return self._tool_repo_goto_definition(payload)
        if op == "repo_find_references":
            return self._tool_repo_find_references(payload)
        if op == "write":
            return self._tool_file_write(payload)
        if op == "patch":
            return self._tool_file_patch(payload)
        if op == "edit":
            return self._tool_file_edit(payload)
        if op == "delete":
            return self._tool_file_delete(payload)
        if op == "mkdir":
            return self._tool_fs_mkdir(payload)
        return self._json_error("UNSUPPORTED_OP", f"workspace op not supported: {op}")

    def _tool_process(self, args: dict[str, Any]) -> dict[str, Any]:
        payload = self._merge_facade_args(args)
        nested_err = self._reject_nested_facade_args("process", payload)
        if nested_err is not None:
            return nested_err
        op = _first_string_arg(payload, ["op"]).lower()
        if op == "help":
            return self._facade_help(
                "process",
                ["start", "inspect", "tail", "follow", "kill"],
            )
        if op == "start":
            return self._tool_process_start(payload)
        if op == "inspect":
            return self._tool_process_inspect(payload)
        if op == "tail":
            return self._tool_process_tail(payload)
        if op == "follow":
            return self._tool_process_follow(payload)
        if op == "kill":
            return self._tool_process_kill(payload)
        return self._json_error("UNSUPPORTED_OP", f"process op not supported: {op}")

    def _tool_thread(self, args: dict[str, Any]) -> dict[str, Any]:
        payload = self._merge_facade_args(args)
        nested_err = self._reject_nested_facade_args("thread", payload)
        if nested_err is not None:
            return nested_err
        op = _first_string_arg(payload, ["op"]).lower()
        if op == "help":
            return self._facade_help(
                "thread",
                [
                    "diff",
                    "state",
                    "usage",
                    "events",
                    "hook_run",
                    "request_input",
                    "spawn_agent",
                    "send_input",
                    "wait",
                    "close",
                    "close_agent",
                ],
            )
        if op == "state":
            return self._tool_thread_state(payload)
        if op == "diff":
            return self._tool_thread_diff(payload)
        if op == "events":
            return self._tool_thread_events(payload)
        if op == "usage":
            return self._tool_thread_usage(payload)
        if op == "hook_run":
            return self._tool_thread_hook_run(payload)
        if op in {"request_input", "request_user_input"}:
            return self._tool_request_user_input(payload)
        if op in {"spawn_agent", "agent_spawn"}:
            return self._tool_agent_spawn(payload)
        if op == "send_input":
            return self._tool_subagent_send_input(
                {
                    "subagent_id": payload.get("subagent_id") if payload.get("subagent_id") is not None else payload.get("id"),
                    "input": payload.get("input") if payload.get("input") is not None else payload.get("message"),
                }
            )
        if op == "wait":
            wait_payload: dict[str, Any] = dict(payload)
            if wait_payload.get("subagent_id") is None and wait_payload.get("id") is not None:
                wait_payload["subagent_id"] = wait_payload.get("id")
            if wait_payload.get("subagent_id") is None and isinstance(wait_payload.get("ids"), list):
                ids = wait_payload.get("ids") or []
                wait_payload["subagent_id"] = ids[0] if ids else None
            return self._tool_subagent_wait(wait_payload)
        if op in {"close", "close_agent"}:
            close_payload: dict[str, Any] = dict(payload)
            if close_payload.get("subagent_id") is None and close_payload.get("id") is not None:
                close_payload["subagent_id"] = close_payload.get("id")
            if close_payload.get("subagent_id") is None and close_payload.get("agent_id") is not None:
                close_payload["subagent_id"] = close_payload.get("agent_id")
            return self._tool_subagent_close(close_payload)
        return self._json_error("UNSUPPORTED_OP", f"thread op not supported: {op}")

    def _tool_artifact(self, args: dict[str, Any]) -> dict[str, Any]:
        payload = self._merge_facade_args(args)
        nested_err = self._reject_nested_facade_args("artifact", payload)
        if nested_err is not None:
            return nested_err
        op = _first_string_arg(payload, ["op"]).lower()
        if op == "help":
            return self._facade_help(
                "artifact",
                ["write", "update_plan", "list", "read", "delete"],
            )
        if op == "write":
            return self._tool_artifact_write(payload)
        if op == "update_plan":
            return self._tool_update_plan(payload)
        if op == "list":
            return self._tool_artifact_list(payload)
        if op == "read":
            return self._tool_artifact_read(payload)
        if op == "delete":
            return self._tool_artifact_delete(payload)
        return self._json_error("UNSUPPORTED_OP", f"artifact op not supported: {op}")

    def _tool_integration(self, args: dict[str, Any]) -> dict[str, Any]:
        payload = self._merge_facade_args(args)
        nested_err = self._reject_nested_facade_args("integration", payload)
        if nested_err is not None:
            return nested_err
        op = _first_string_arg(payload, ["op"]).lower()
        if op == "help":
            return self._facade_help(
                "integration",
                [
                    "mcp_list_servers",
                    "mcp_list_tools",
                    "mcp_list_resources",
                    "mcp_call",
                    "web_search",
                    "web_fetch",
                    "view_image",
                ],
            )
        if op == "mcp_list_servers":
            return self._tool_mcp_list_servers(payload)
        if op == "mcp_list_tools":
            return self._tool_mcp_list_tools(payload)
        if op == "mcp_list_resources":
            return self._tool_mcp_list_resources(payload)
        if op == "mcp_call":
            return self._tool_mcp_call(payload)
        if op == "web_search":
            return self._tool_web_search({"q": payload.get("query") or payload.get("q")})
        if op in {"web_fetch", "webfetch"}:
            return self._tool_webfetch({"url": payload.get("url") or payload.get("link")})
        if op == "view_image":
            return self._tool_view_image({"path": payload.get("path") or payload.get("file")})
        return self._json_error("UNSUPPORTED_OP", f"integration op not supported: {op}")

    def execute(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any]:
        if not isinstance(args, dict):
            args = {"_raw": args}

        injected = self._inject_recovery_error_if_needed(tool_name, args)
        if injected is not None:
            return injected

        self.total_tool_calls += 1
        self._record_event("tool_call", {"tool": tool_name})
        try:
            if tool_name == "workspace":
                result = self._tool_workspace(args)
            elif tool_name == "process":
                result = self._tool_process(args)
            elif tool_name == "thread":
                result = self._tool_thread(args)
            elif tool_name == "artifact":
                result = self._tool_artifact(args)
            elif tool_name == "integration":
                result = self._tool_integration(args)
            elif tool_name == "file_read":
                result = self._tool_file_read(args)
            elif tool_name == "file_glob":
                result = self._tool_file_glob(args)
            elif tool_name == "file_grep":
                result = self._tool_file_grep(args)
            elif tool_name == "repo_search":
                result = self._tool_file_grep(args)
            elif tool_name == "repo_index":
                result = self._tool_repo_index(args)
            elif tool_name == "repo_symbols":
                result = self._tool_repo_symbols(args)
            elif tool_name == "repo_goto_definition":
                result = self._tool_repo_goto_definition(args)
            elif tool_name == "repo_find_references":
                result = self._tool_repo_find_references(args)
            elif tool_name == "file_write":
                result = self._tool_file_write(args)
            elif tool_name == "file_patch":
                result = self._tool_file_patch(args)
            elif tool_name == "file_edit":
                result = self._tool_file_edit(args)
            elif tool_name == "file_delete":
                result = self._tool_file_delete(args)
            elif tool_name == "fs_mkdir":
                result = self._tool_fs_mkdir(args)
            elif tool_name == "process_start":
                result = self._tool_process_start(args)
            elif tool_name == "process_inspect":
                result = self._tool_process_inspect(args)
            elif tool_name == "process_tail":
                result = self._tool_process_tail(args)
            elif tool_name == "process_follow":
                result = self._tool_process_follow(args)
            elif tool_name == "process_kill":
                result = self._tool_process_kill(args)
            elif tool_name == "artifact_write":
                result = self._tool_artifact_write(args)
            elif tool_name == "artifact_list":
                result = self._tool_artifact_list(args)
            elif tool_name == "artifact_read":
                result = self._tool_artifact_read(args)
            elif tool_name == "artifact_delete":
                result = self._tool_artifact_delete(args)
            elif tool_name == "update_plan":
                result = self._tool_update_plan(args)
            elif tool_name == "request_user_input":
                result = self._tool_request_user_input(args)
            elif tool_name == "ask_user":
                result = self._tool_ask_user(args)
            elif tool_name == "web_search":
                result = self._tool_web_search(args)
            elif tool_name == "webfetch":
                result = self._tool_webfetch(args)
            elif tool_name == "view_image":
                result = self._tool_view_image(args)
            elif tool_name == "mcp_list_servers":
                result = self._tool_mcp_list_servers(args)
            elif tool_name == "mcp_list_tools":
                result = self._tool_mcp_list_tools(args)
            elif tool_name == "mcp_list_resources":
                result = self._tool_mcp_list_resources(args)
            elif tool_name == "mcp_call":
                result = self._tool_mcp_call(args)
            elif tool_name == "thread_diff":
                result = self._tool_thread_diff(args)
            elif tool_name == "thread_state":
                result = self._tool_thread_state(args)
            elif tool_name == "thread_usage":
                result = self._tool_thread_usage(args)
            elif tool_name == "thread_events":
                result = self._tool_thread_events(args)
            elif tool_name == "thread_hook_run":
                result = self._tool_thread_hook_run(args)
            elif tool_name == "agent_spawn":
                result = self._tool_agent_spawn(args)
            elif tool_name == "subagent_send_input":
                result = self._tool_subagent_send_input(args)
            elif tool_name == "subagent_wait":
                result = self._tool_subagent_wait(args)
            elif tool_name == "subagent_close":
                result = self._tool_subagent_close(args)
            else:
                result = self._json_error("UNSUPPORTED_TOOL", f"Unsupported tool in runtime: {tool_name}")
        except Exception as exc:
            result = self._json_error("RUNTIME_EXCEPTION", f"Runtime exception: {exc}")
        return result


def claimed_success_without_execution(final_text: str, task_success: bool) -> bool:
    if task_success:
        return False
    pattern = re.compile(r"\"success\"\\s*:\\s*true|\\bsuccess\\b|完成|已完成|done", re.IGNORECASE)
    return bool(pattern.search(final_text))


def run_one_case(
    *,
    provider: legacy.ProviderCase,
    case: CaseV2,
    mode: str,
    full_surface: list[dict[str, Any]],
    timeout_sec: int,
    max_steps: int,
    no_tool_conclusion_guard: bool,
    runtime_config: RuntimeConfig,
) -> dict[str, Any]:
    system_prompt = (
        "You are a practical assistant. "
        "You MUST fully complete the user's request with available tools before concluding. "
        "Do not output conversational filler or future-tense promises. "
        "Do not claim actions you did not execute. "
        "If required information is missing or explicit confirmation is needed, "
        "you MUST call `ask_user` and continue after the user reply."
    )
    user_prompt = case.user_prompt

    if runtime_config.runtime_mode == "mock":
        runtime: RuntimeBase = MockRuntime(case, mode)
    else:
        runtime = RealSandboxRuntime(case, mode, runtime_config)
    tool_events: list[dict[str, Any]] = []
    step_traces: list[dict[str, Any]] = []
    final_text = ""
    reasoning_text = ""
    no_tool_guard_injections = 0
    recovery_retry_injections = 0
    max_no_tool_guard_injections = 1 if no_tool_conclusion_guard else 0
    max_recovery_retry_injections = 1
    started = time.perf_counter()

    def no_tool_correction_message() -> str:
        return (
            "System: You attempted to conclude the task without any tool call. "
            f"You MUST call `{case.target_tool}` and use real tool output as evidence. "
            "If you are blocked by missing user confirmation/input, call `ask_user`. "
            "Do not fabricate values. If a tool error provides suggested_args, call again "
            "with suggested_args. DO NOT finish the task yet."
        )

    def recovery_retry_correction_message() -> str:
        return (
            "System: Recovery is not complete yet. "
            f"You previously received INJECTED_TRANSIENT for `{case.target_tool}`. "
            "You MUST call the target capability again (use suggested_args when available), "
            "and finish only after a successful tool result."
        )

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
            tools = build_responses_tools(full_surface)

            with requests.Session() as session:
                for step in range(1, max_steps + 1):
                    body = {
                        "model": provider.model,
                        "input": history,
                        "tools": tools,
                        "tool_choice": "auto",
                        "temperature": 0,
                    }
                    t0 = time.perf_counter()
                    resp = session.post(url, headers=headers, json=body, timeout=timeout_sec)
                    latency_ms = int((time.perf_counter() - t0) * 1000)
                    data = resp.json()
                    if resp.status_code >= 400:
                        raise RuntimeError(
                            f"HTTP {resp.status_code}: {json.dumps(data, ensure_ascii=False)[:1200]}"
                        )

                    in_tok, out_tok, cached_tok = legacy.parse_usage(data.get("usage"))
                    step_trace = {
                        "step": step,
                        "request": {
                            "url": url,
                            "headers": legacy.sanitize_headers(headers),
                            "body": body,
                        },
                        "response": data,
                        "latency_ms": latency_ms,
                        "usage_parsed": {
                            "input_tokens": in_tok,
                            "output_tokens": out_tok,
                            "cached_tokens": cached_tok,
                        },
                        "tool_events": [],
                    }

                    output_items = data.get("output") if isinstance(data.get("output"), list) else []
                    function_calls = [x for x in output_items if x.get("type") == "function_call"]
                    if function_calls:
                        for fc in function_calls:
                            tool_name = safe_text(fc.get("name"))
                            args_raw = safe_text(fc.get("arguments") or "{}")
                            try:
                                parsed_args = json.loads(args_raw)
                                if not isinstance(parsed_args, dict):
                                    parsed_args = {"_raw": parsed_args}
                            except Exception:
                                parsed_args = {"_raw": args_raw, "_error": "invalid_json"}

                            exec_t0 = time.perf_counter()
                            is_target_match = runtime.is_target_match(tool_name, parsed_args)
                            result = runtime.execute(tool_name, parsed_args)
                            result_for_model = compact_tool_result_for_model(result)
                            exec_ms = int((time.perf_counter() - exec_t0) * 1000)
                            tool_output_text = json.dumps(
                                result_for_model, ensure_ascii=False, separators=(",", ":")
                            )

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
                                "is_target_match": is_target_match,
                                "tool_result": result,
                                "tool_result_for_model": result_for_model,
                                "tool_output_text": tool_output_text,
                                "execution_ms": exec_ms,
                            }
                            tool_events.append(event)
                            step_trace["tool_events"].append(event)

                            if is_user_query_tool_call(tool_name, parsed_args):
                                user_reply = simulated_user_reply_text(tool_name, parsed_args)
                                history.append(
                                    {
                                        "role": "user",
                                        "content": [{"type": "input_text", "text": user_reply}],
                                    }
                                )
                                step_trace.setdefault("auto_user_replies", []).append(
                                    {
                                        "trigger_tool": tool_name,
                                        "reply_text": user_reply,
                                    }
                                )

                        step_traces.append(step_trace)
                        continue

                    final_text, reasoning_text = legacy.extract_responses_text(output_items)
                    if (
                        mode == "recovery"
                        and recovery_retry_injections < max_recovery_retry_injections
                        and step < max_steps
                    ):
                        target_events_so_far = [
                            e
                            for e in tool_events
                            if e.get("tool") == case.target_tool or bool(e.get("is_target_match"))
                        ]
                        had_injected_so_far = any(
                            e.get("tool_result", {}).get("error_code") == "INJECTED_TRANSIENT"
                            for e in target_events_so_far
                        )
                        target_success_so_far = any(
                            e.get("tool_result", {}).get("ok") is True for e in target_events_so_far
                        )
                        if had_injected_so_far and not target_success_so_far:
                            correction = recovery_retry_correction_message()
                            history.append(
                                {
                                    "role": "system",
                                    "content": [{"type": "input_text", "text": correction}],
                                }
                            )
                            step_trace["guard_injected"] = {
                                "type": "recovery_retry_intercept",
                                "message": correction,
                            }
                            recovery_retry_injections += 1
                            step_traces.append(step_trace)
                            final_text = ""
                            reasoning_text = ""
                            continue
                    if (
                        len(tool_events) == 0
                        and no_tool_guard_injections < max_no_tool_guard_injections
                        and step < max_steps
                    ):
                        correction = no_tool_correction_message()
                        history.append(
                            {
                                "role": "system",
                                "content": [{"type": "input_text", "text": correction}],
                            }
                        )
                        step_trace["guard_injected"] = {
                            "type": "no_tool_conclusion_intercept",
                            "message": correction,
                        }
                        no_tool_guard_injections += 1
                        step_traces.append(step_trace)
                        final_text = ""
                        reasoning_text = ""
                        continue

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
            tools = build_chat_tools(full_surface)

            with requests.Session() as session:
                for step in range(1, max_steps + 1):
                    body = {
                        "model": provider.model,
                        "messages": messages,
                        "tools": tools,
                        "tool_choice": "auto",
                        "temperature": 0,
                        "stream": False,
                    }
                    t0 = time.perf_counter()
                    resp = session.post(url, headers=headers, json=body, timeout=timeout_sec)
                    latency_ms = int((time.perf_counter() - t0) * 1000)
                    data = resp.json()
                    if resp.status_code >= 400:
                        raise RuntimeError(
                            f"HTTP {resp.status_code}: {json.dumps(data, ensure_ascii=False)[:1200]}"
                        )

                    in_tok, out_tok, cached_tok = legacy.parse_usage(data.get("usage"))
                    step_trace = {
                        "step": step,
                        "request": {
                            "url": url,
                            "headers": legacy.sanitize_headers(headers),
                            "body": body,
                        },
                        "response": data,
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
                            tool_name = safe_text(fn.get("name"))
                            args_raw = safe_text(fn.get("arguments") or "{}")
                            try:
                                parsed_args = json.loads(args_raw)
                                if not isinstance(parsed_args, dict):
                                    parsed_args = {"_raw": parsed_args}
                            except Exception:
                                parsed_args = {"_raw": args_raw, "_error": "invalid_json"}

                            exec_t0 = time.perf_counter()
                            is_target_match = runtime.is_target_match(tool_name, parsed_args)
                            result = runtime.execute(tool_name, parsed_args)
                            result_for_model = compact_tool_result_for_model(result)
                            exec_ms = int((time.perf_counter() - exec_t0) * 1000)
                            tool_output_text = json.dumps(
                                result_for_model, ensure_ascii=False, separators=(",", ":")
                            )

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
                                "is_target_match": is_target_match,
                                "tool_result": result,
                                "tool_result_for_model": result_for_model,
                                "tool_output_text": tool_output_text,
                                "execution_ms": exec_ms,
                            }
                            tool_events.append(event)
                            step_trace["tool_events"].append(event)

                            if is_user_query_tool_call(tool_name, parsed_args):
                                user_reply = simulated_user_reply_text(tool_name, parsed_args)
                                messages.append({"role": "user", "content": user_reply})
                                step_trace.setdefault("auto_user_replies", []).append(
                                    {
                                        "trigger_tool": tool_name,
                                        "reply_text": user_reply,
                                    }
                                )

                        step_traces.append(step_trace)
                        continue

                    final_text, reasoning_text = extract_chat_text_and_reasoning(msg)
                    if (
                        mode == "recovery"
                        and recovery_retry_injections < max_recovery_retry_injections
                        and step < max_steps
                    ):
                        target_events_so_far = [
                            e
                            for e in tool_events
                            if e.get("tool") == case.target_tool or bool(e.get("is_target_match"))
                        ]
                        had_injected_so_far = any(
                            e.get("tool_result", {}).get("error_code") == "INJECTED_TRANSIENT"
                            for e in target_events_so_far
                        )
                        target_success_so_far = any(
                            e.get("tool_result", {}).get("ok") is True for e in target_events_so_far
                        )
                        if had_injected_so_far and not target_success_so_far:
                            correction = recovery_retry_correction_message()
                            messages.append({"role": "system", "content": correction})
                            step_trace["guard_injected"] = {
                                "type": "recovery_retry_intercept",
                                "message": correction,
                            }
                            recovery_retry_injections += 1
                            step_traces.append(step_trace)
                            final_text = ""
                            reasoning_text = ""
                            continue
                    if (
                        len(tool_events) == 0
                        and no_tool_guard_injections < max_no_tool_guard_injections
                        and step < max_steps
                    ):
                        correction = no_tool_correction_message()
                        messages.append({"role": "system", "content": correction})
                        step_trace["guard_injected"] = {
                            "type": "no_tool_conclusion_intercept",
                            "message": correction,
                        }
                        no_tool_guard_injections += 1
                        step_traces.append(step_trace)
                        final_text = ""
                        reasoning_text = ""
                        continue

                    step_traces.append(step_trace)
                    break

        target_events = [
            e
            for e in tool_events
            if e.get("tool") == case.target_tool or bool(e.get("is_target_match"))
        ]
        target_success = any(e.get("tool_result", {}).get("ok") is True for e in target_events)
        had_injected = any(
            e.get("tool_result", {}).get("error_code") == "INJECTED_TRANSIENT" for e in target_events
        )
        evidence_in_final = runtime.evidence_nonce in final_text
        called_any_tool = len(tool_events) > 0

        if mode == "recovery":
            case_pass = target_success and had_injected and len(target_events) >= 2
        else:
            case_pass = target_success

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
            "mode": mode,
            "case_id": case.case_id,
            "target_tool": case.target_tool,
            "capability_hint": case.capability_hint,
            "system_prompt": system_prompt,
            "user_prompt": user_prompt,
            "tool_events": tool_events,
            "step_traces": step_traces,
            "tool_call_count_all": len(tool_events),
            "tool_call_count_target": len(target_events),
            "called_target_tool": len(target_events) > 0,
            "called_any_tool": called_any_tool,
            "no_tool_guard_injections": no_tool_guard_injections,
            "recovery_retry_injections": recovery_retry_injections,
            "target_success": target_success,
            "had_injected_failure": had_injected,
            "evidence_nonce": runtime.evidence_nonce,
            "evidence_in_final": evidence_in_final,
            "case_pass": case_pass,
            "skip_tool": not called_any_tool,
            "final_text": final_text,
            "reasoning_text": reasoning_text,
            "claimed_success_without_execution": claimed_success_without_execution(
                final_text, target_success
            ),
            "usage_total": usage_total,
            "duration_ms": int((time.perf_counter() - started) * 1000),
            "error": None,
            "case": dataclasses.asdict(case),
        }
    except Exception as exc:
        return {
            "provider": provider.name,
            "endpoint": provider.endpoint,
            "base_url": provider.base_url,
            "model": provider.model,
            "mode": mode,
            "case_id": case.case_id,
            "target_tool": case.target_tool,
            "capability_hint": case.capability_hint,
            "system_prompt": system_prompt,
            "user_prompt": user_prompt,
            "tool_events": tool_events,
            "step_traces": step_traces,
            "tool_call_count_all": len(tool_events),
            "tool_call_count_target": len(
                [
                    e
                    for e in tool_events
                    if e.get("tool") == case.target_tool or bool(e.get("is_target_match"))
                ]
            ),
            "called_target_tool": len(
                [
                    e
                    for e in tool_events
                    if e.get("tool") == case.target_tool or bool(e.get("is_target_match"))
                ]
            )
            > 0,
            "called_any_tool": len(tool_events) > 0,
            "no_tool_guard_injections": no_tool_guard_injections,
            "recovery_retry_injections": recovery_retry_injections,
            "target_success": False,
            "had_injected_failure": False,
            "evidence_nonce": runtime.evidence_nonce,
            "evidence_in_final": False,
            "case_pass": False,
            "skip_tool": len(tool_events) == 0,
            "final_text": final_text,
            "reasoning_text": reasoning_text,
            "claimed_success_without_execution": claimed_success_without_execution(final_text, False),
            "usage_total": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0, "latency_ms": 0},
            "duration_ms": int((time.perf_counter() - started) * 1000),
            "error": str(exc),
            "case": dataclasses.asdict(case),
        }
    finally:
        runtime.close()


def safe_rate(numer: int, denom: int) -> float:
    if denom <= 0:
        return 0.0
    return numer / denom


def average(values: list[float]) -> float:
    if not values:
        return 0.0
    return float(statistics.fmean(values))


def render_report(
    results: list[dict[str, Any]],
    out_dir: Path,
    started_at: str,
    elapsed_ms: int,
    benchmark_version: str,
) -> None:
    providers = sorted({r["provider"] for r in results})
    lines: list[str] = []
    lines.append(f"# Tool Surface Realistic Benchmark ({benchmark_version})")
    lines.append("")
    lines.append(f"- Generated at: {started_at}")
    lines.append(f"- Total wall time: {elapsed_ms} ms")
    lines.append(f"- Result rows: {len(results)}")
    lines.append("")

    lines.append("## Provider Summary")
    lines.append("")
    lines.append(
        "| provider | modes | cases | pass_rate | skip_tool_rate | target_miss_rate | fake_success_rate | guard_trigger_rate | evidence_echo_rate | avg_latency_ms | avg_input_tokens | avg_cached_tokens |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")

    for provider in providers:
        rows = [r for r in results if r["provider"] == provider]
        passed = sum(1 for r in rows if r["case_pass"])
        skipped = sum(1 for r in rows if r["skip_tool"])
        target_miss = sum(1 for r in rows if not r["called_target_tool"])
        fake = sum(1 for r in rows if r["claimed_success_without_execution"])
        guarded = sum(1 for r in rows if int(r.get("no_tool_guard_injections", 0)) > 0)
        echoed = sum(1 for r in rows if r["evidence_in_final"])
        lat = average([float(r["usage_total"]["latency_ms"]) for r in rows])
        inp = average([float(r["usage_total"]["input_tokens"]) for r in rows])
        cac = average([float(r["usage_total"]["cached_tokens"]) for r in rows])
        lines.append(
            f"| {provider} | {len({r['mode'] for r in rows})} | {len(rows)} | "
            f"{safe_rate(passed, len(rows))*100:.2f}% | "
            f"{safe_rate(skipped, len(rows))*100:.2f}% | "
            f"{safe_rate(target_miss, len(rows))*100:.2f}% | "
            f"{safe_rate(fake, len(rows))*100:.2f}% | "
            f"{safe_rate(guarded, len(rows))*100:.2f}% | "
            f"{safe_rate(echoed, len(rows))*100:.2f}% | "
            f"{lat:.1f} | {inp:.1f} | {cac:.1f} |"
        )

    lines.append("")
    lines.append("## Per Case")
    lines.append("")
    lines.append(
        "| provider | case_id | mode | target_tool | pass | target_calls | all_calls | target_called | guard_injections | evidence_in_final | skip_tool | fake_success | error |"
    )
    lines.append("|---|---|---|---|---|---:|---:|---|---:|---|---|---|---|")
    for row in sorted(results, key=lambda x: (x["provider"], x["mode"], x["case_id"])):
        lines.append(
            f"| {row['provider']} | {row['case_id']} | {row['mode']} | {row['target_tool']} | "
            f"{'PASS' if row['case_pass'] else 'FAIL'} | {row['tool_call_count_target']} | {row['tool_call_count_all']} | "
            f"{str(row['called_target_tool']).lower()} | {int(row.get('no_tool_guard_injections', 0))} | "
            f"{str(row['evidence_in_final']).lower()} | {str(row['skip_tool']).lower()} | "
            f"{str(row['claimed_success_without_execution']).lower()} | {row.get('error') or ''} |"
        )

    lines.append("")
    lines.append("## Trace Files")
    lines.append("")
    lines.append("- Raw aggregate payload: `raw_results.json`.")
    lines.append("- Full per-case traces: `details/<provider>/<mode>/<case_id>.json`.")

    (out_dir / "report.md").write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    args = parse_args()
    parallel_cases = max(1, int(args.parallel_cases))
    process_allowlist = {x.strip().lower() for x in args.real_process_allowlist.split(",") if x.strip()}
    runtime_config = RuntimeConfig(
        runtime_mode=args.runtime_mode,
        workspace_root=args.runtime_workspace_root.resolve(),
        process_timeout_sec=max(1, args.real_process_timeout_sec),
        process_allowlist=process_allowlist,
        web_timeout_sec=max(1, args.real_web_timeout_sec),
        web_max_bytes=max(512, args.real_web_max_bytes),
    )
    all_tool_names = legacy.load_tool_names_from_spec(args.spec_rs)
    include_tools = {x.strip() for x in args.tools.split(",") if x.strip()}
    exclude_tools = {x.strip() for x in args.exclude_tools.split(",") if x.strip()}
    tool_names = [
        t
        for t in all_tool_names
        if (not include_tools or t in include_tools) and t not in exclude_tools
    ]
    if not tool_names:
        raise RuntimeError("no tools selected for full-tool injection")

    full_surface = build_full_tool_surface(tool_names)
    allowed_tools = {s["name"] for s in full_surface}
    if args.case_source == "auto":
        cases = build_auto_cases_from_tools(tool_names)
    else:
        cases = parse_cases(args.cases_file, allowed_tools)

    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    for m in modes:
        if m not in {"direct", "recovery"}:
            raise RuntimeError(f"unsupported mode: {m}")

    ts = dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    if args.list_cases_only:
        out_dir = args.out_dir or Path("docs/reports") / f"tool-surface-realistic-cases-{ts}"
        out_dir.mkdir(parents=True, exist_ok=True)
        payload = {
            "generated_at": dt.datetime.now().isoformat(timespec="seconds"),
            "tool_count_injected": len(full_surface),
            "case_count": len(cases),
            "case_source": args.case_source,
            "tools": tool_names,
            "cases": [dataclasses.asdict(c) for c in cases],
        }
        out = out_dir / "cases.json"
        out.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"[done] cases={out}")
        return 0

    env = legacy.load_env(args.env_file)
    selected_providers = {x.strip() for x in args.providers.split(",") if x.strip()} or None
    providers = legacy.load_provider_cases(args.providers_file, env, selected_providers)

    out_dir = args.out_dir or Path("docs/reports") / f"tool-surface-realistic-benchmark-{ts}"
    out_dir.mkdir(parents=True, exist_ok=True)

    started_at = dt.datetime.now().isoformat(timespec="seconds")
    run_start = time.perf_counter()
    results: list[dict[str, Any]] = []

    if args.shuffle_cases:
        random.shuffle(cases)

    def run_provider(provider: legacy.ProviderCase) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        task_list = [(mode, case) for mode in modes for case in cases]
        provider_root = runtime_config.workspace_root / provider.name
        if provider_root.exists():
            shutil.rmtree(provider_root, ignore_errors=True)

        def run_task(mode: str, case: CaseV2) -> dict[str, Any]:
            print(
                f"[run] provider={provider.name} mode={mode} case={case.case_id}",
                flush=True,
            )
            isolated_runtime = dataclasses.replace(
                runtime_config,
                workspace_root=runtime_config.workspace_root / provider.name / mode / case.case_id,
            )
            return run_one_case(
                provider=provider,
                case=case,
                mode=mode,
                full_surface=full_surface,
                timeout_sec=args.timeout_sec,
                max_steps=args.max_steps,
                no_tool_conclusion_guard=args.no_tool_conclusion_guard == "on",
                runtime_config=isolated_runtime,
            )

        if parallel_cases > 1:
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=min(parallel_cases, len(task_list))
            ) as ex:
                futures = [ex.submit(run_task, mode, case) for mode, case in task_list]
                for fut in concurrent.futures.as_completed(futures):
                    rows.append(fut.result())
            rows.sort(key=lambda r: (r["mode"], r["case_id"]))
        else:
            for mode, case in task_list:
                rows.append(run_task(mode, case))
        return rows

    if args.parallel_providers > 1 and len(providers) > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.parallel_providers) as ex:
            futures = [ex.submit(run_provider, provider) for provider in providers]
            for fut in concurrent.futures.as_completed(futures):
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
        "benchmark_version": args.benchmark_version,
        "providers": [dataclasses.asdict(p) | {"api_key": "***"} for p in providers],
        "modes": modes,
        "tool_count_injected": len(full_surface),
        "tool_names_injected": tool_names,
        "case_count": len(cases),
        "case_source": args.case_source,
        "cases_file": str(args.cases_file) if args.case_source == "file" else None,
        "args": {
            "timeout_sec": args.timeout_sec,
            "max_steps": args.max_steps,
            "parallel_providers": args.parallel_providers,
            "parallel_cases": parallel_cases,
            "providers_file": str(args.providers_file),
            "spec_rs": str(args.spec_rs),
            "shuffle_cases": args.shuffle_cases,
            "no_tool_conclusion_guard": args.no_tool_conclusion_guard,
            "runtime_mode": runtime_config.runtime_mode,
            "runtime_workspace_root": str(runtime_config.workspace_root),
            "real_process_timeout_sec": runtime_config.process_timeout_sec,
            "real_process_allowlist": sorted(runtime_config.process_allowlist),
            "real_web_timeout_sec": runtime_config.web_timeout_sec,
            "real_web_max_bytes": runtime_config.web_max_bytes,
        },
    }
    raw = {
        "meta": meta,
        "cases": [dataclasses.asdict(c) for c in cases],
        "results": results,
    }
    raw_path = out_dir / "raw_results.json"
    raw_path.write_text(json.dumps(raw, ensure_ascii=False, indent=2), encoding="utf-8")

    render_report(
        results,
        out_dir,
        started_at,
        meta["elapsed_ms"],
        args.benchmark_version,
    )
    print(f"[done] report={out_dir / 'report.md'}")
    print(f"[done] raw={raw_path}")
    print(f"[done] details={details_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
