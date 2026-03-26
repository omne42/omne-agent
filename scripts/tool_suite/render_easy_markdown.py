#!/usr/bin/env python3
"""Render an easy-to-scan markdown summary from tool-surface raw_results.json."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def pct(numer: int, denom: int) -> str:
    if denom <= 0:
        return "0.00%"
    return f"{(numer / denom) * 100:.2f}%"


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Render easy markdown summary for tool-surface benchmark.")
    p.add_argument("--raw", type=Path, required=True, help="Path to raw_results.json")
    p.add_argument("--out", type=Path, required=True, help="Output markdown path")
    return p.parse_args()


def main() -> int:
    args = parse_args()
    raw = json.loads(args.raw.read_text(encoding="utf-8"))
    meta: dict[str, Any] = raw.get("meta", {})
    results: list[dict[str, Any]] = raw.get("results", [])

    providers = sorted({r.get("provider", "") for r in results if r.get("provider")})
    lines: list[str] = []
    lines.append("# Tool 全量功能测试易读版")
    lines.append("")
    lines.append(f"- 生成时间: {meta.get('generated_at', '-')}")
    lines.append(f"- 总耗时: {meta.get('elapsed_ms', 0)} ms")
    lines.append(f"- Tool 数: {meta.get('tool_count', '-')}")
    lines.append(f"- Feature Case 数: {meta.get('case_count', '-')}")
    lines.append(f"- Difficulties: {', '.join(meta.get('difficulties', []) or ['-'])}")
    lines.append(f"- 结果行数: {len(results)}")
    lines.append("")

    lines.append("## Provider 总览")
    lines.append("")
    lines.append("| provider | endpoint | model | single_first | single_eventual | recovery |")
    lines.append("|---|---|---|---:|---:|---:|")

    for provider in providers:
        rows = [r for r in results if r.get("provider") == provider]
        single_rows = [r for r in rows if r.get("mode") == "single"]
        recovery_rows = [r for r in rows if r.get("mode") == "recovery"]

        single_first = sum(1 for r in single_rows if r.get("single_first_pass") is True)
        single_eventual = sum(1 for r in single_rows if r.get("single_eventual_pass") is True)
        recovery_pass = sum(1 for r in recovery_rows if r.get("recovery_pass") is True)

        sample = rows[0] if rows else {}
        endpoint = sample.get("endpoint", "-")
        model = sample.get("model", "-")

        lines.append(
            f"| {provider} | {endpoint} | {model} | "
            f"{single_first}/{len(single_rows)} ({pct(single_first, len(single_rows))}) | "
            f"{single_eventual}/{len(single_rows)} ({pct(single_eventual, len(single_rows))}) | "
            f"{recovery_pass}/{len(recovery_rows)} ({pct(recovery_pass, len(recovery_rows))}) |"
        )

    lines.append("")
    lines.append("## 关注清单（非一次通过/失败）")
    lines.append("")
    lines.append("| provider | case_id | mode | tool | feature | difficulty | status | tool_calls | final_output_ok | error | detail |")
    lines.append("|---|---|---|---|---|---|---|---:|---|---|---|")

    focus_rows: list[tuple[dict[str, Any], str]] = []
    for r in results:
        mode = r.get("mode")
        if mode == "single":
            if r.get("single_first_pass") is not True:
                status = (
                    "single_failed"
                    if r.get("single_eventual_pass") is not True
                    else "single_recovered"
                )
                focus_rows.append((r, status))
        elif mode == "recovery":
            if r.get("recovery_pass") is not True:
                focus_rows.append((r, "recovery_failed"))

    focus_rows.sort(
        key=lambda x: (
            x[0].get("provider", ""),
            x[0].get("case_id", ""),
            x[0].get("mode", ""),
        )
    )

    for r, status in focus_rows:
        provider = r.get("provider", "")
        mode = r.get("mode", "")
        case_id = r.get("case_id", "")
        tool = r.get("tool", "")
        feature = r.get("feature", "")
        difficulty = r.get("difficulty", "")
        tool_calls = r.get("tool_call_count", 0)
        final_output_ok = r.get("final_output_ok", False)
        error = r.get("error") or ""
        detail_rel = f"details/{provider}/{mode}/{case_id}.json"
        lines.append(
            f"| {provider} | {case_id} | {mode} | {tool} | {feature} | {difficulty} | {status} | "
            f"{tool_calls} | {str(final_output_ok).lower()} | {error} | `{detail_rel}` |"
        )

    lines.append("")
    lines.append("## 使用说明")
    lines.append("")
    lines.append("- 本文档默认不展示 `single` 模式的一次通过项。")
    lines.append("- `single_recovered`: single 模式首轮失败，但后续修正后跑通。")
    lines.append("- `single_failed`: single 模式最终未跑通。")
    lines.append("- `recovery_failed`: recovery 模式在注入错误后仍未跑通。")
    lines.append("")
    lines.append("## 原始文件")
    lines.append("")
    lines.append(f"- raw: `{args.raw}`")
    lines.append("")

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines), encoding="utf-8")
    print(str(args.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
