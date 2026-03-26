#!/usr/bin/env python3
"""Render easy markdown for realistic benchmark v2."""

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
    p = argparse.ArgumentParser(description="Render easy markdown for realistic benchmark v2.")
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
    lines.append("# Tool Realistic Benchmark v2 易读版")
    lines.append("")
    lines.append(f"- 生成时间: {meta.get('generated_at', '-')}")
    lines.append(f"- 总耗时: {meta.get('elapsed_ms', 0)} ms")
    lines.append(f"- 注入工具数: {meta.get('tool_count_injected', '-')}")
    lines.append(f"- 用例数: {meta.get('case_count', '-')}")
    lines.append(f"- 结果行数: {len(results)}")
    lines.append("")

    lines.append("## Provider 总览")
    lines.append("")
    lines.append("| provider | model | pass | skip_tool | fake_success | evidence_echo |")
    lines.append("|---|---|---:|---:|---:|---:|")
    for provider in providers:
        rows = [r for r in results if r.get("provider") == provider]
        passed = sum(1 for r in rows if r.get("case_pass") is True)
        skipped = sum(1 for r in rows if r.get("skip_tool") is True)
        fake = sum(1 for r in rows if r.get("claimed_success_without_execution") is True)
        echoed = sum(1 for r in rows if r.get("evidence_in_final") is True)
        model = rows[0].get("model", "-") if rows else "-"
        lines.append(
            f"| {provider} | {model} | {passed}/{len(rows)} ({pct(passed, len(rows))}) | "
            f"{skipped}/{len(rows)} ({pct(skipped, len(rows))}) | "
            f"{fake}/{len(rows)} ({pct(fake, len(rows))}) | "
            f"{echoed}/{len(rows)} ({pct(echoed, len(rows))}) |"
        )

    lines.append("")
    lines.append("## 重点清单（失败/跳过/口头成功）")
    lines.append("")
    lines.append(
        "| provider | case_id | mode | target_tool | pass | target_calls | all_calls | skip_tool | fake_success | evidence_in_final | detail |"
    )
    lines.append("|---|---|---|---|---|---:|---:|---|---|---|---|")

    focus = []
    for r in results:
        if (
            r.get("case_pass") is not True
            or r.get("skip_tool") is True
            or r.get("claimed_success_without_execution") is True
        ):
            focus.append(r)
    focus.sort(key=lambda x: (x.get("provider", ""), x.get("case_id", ""), x.get("mode", "")))

    for r in focus:
        provider = r.get("provider", "")
        case_id = r.get("case_id", "")
        mode = r.get("mode", "")
        tool = r.get("target_tool", "")
        detail_rel = f"details/{provider}/{mode}/{case_id}.json"
        lines.append(
            f"| {provider} | {case_id} | {mode} | {tool} | "
            f"{'PASS' if r.get('case_pass') else 'FAIL'} | {r.get('tool_call_count_target', 0)} | "
            f"{r.get('tool_call_count_all', 0)} | {str(r.get('skip_tool', False)).lower()} | "
            f"{str(r.get('claimed_success_without_execution', False)).lower()} | "
            f"{str(r.get('evidence_in_final', False)).lower()} | `{detail_rel}` |"
        )

    lines.append("")
    lines.append("## 说明")
    lines.append("")
    lines.append("- `pass`: 框架判定工具执行成功（而不是模型自报成功）。")
    lines.append("- `skip_tool`: 没有任何工具调用。")
    lines.append("- `fake_success`: 未执行成功但文本出现成功声称。")
    lines.append("- `evidence_in_final`: 最终文本是否主动回显证据 nonce。")

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines), encoding="utf-8")
    print(str(args.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
