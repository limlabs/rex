"""
Agent loop: sends a task prompt to the Anthropic API with condition-specific
tools, dispatches tool calls, and collects metrics.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import anthropic

client = anthropic.Anthropic()


@dataclass
class AgentMetrics:
    input_tokens: int = 0
    output_tokens: int = 0
    tool_calls: int = 0
    errors: int = 0
    wall_clock_ms: float = 0
    turns: int = 0

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens


ToolExecutor = Callable[[str, dict, Path], tuple[str, bool]]


def run_agent(
    prompt: str,
    tools: list[dict],
    workdir: Path,
    tool_executor: ToolExecutor,
    *,
    max_turns: int = 50,
    model: str = "claude-sonnet-4-6-20250514",
) -> AgentMetrics:
    """
    Agentic tool-use loop. Sends the prompt, executes tool calls returned by the
    model, feeds results back, and repeats until the model stops calling tools
    or max_turns is reached.

    Returns collected metrics (tokens, tool calls, timing).
    """
    metrics = AgentMetrics()
    t0 = time.monotonic()

    system = (
        f"You are building a web application in a Rex project. "
        f"Rex is a Rust-native React framework with file-based routing (pages/ directory), "
        f"getServerSideProps for server-side data fetching, and API routes in pages/api/. "
        f"Your working directory is: {workdir}\n\n"
        f"Complete the task described in the user message. Use the tools provided. "
        f"When you are finished, respond with text only (no tool calls) summarizing what you did."
    )

    messages: list[dict] = [{"role": "user", "content": prompt}]

    for _ in range(max_turns):
        metrics.turns += 1

        response = client.messages.create(
            model=model,
            max_tokens=4096,
            system=system,
            tools=tools,
            messages=messages,
        )

        metrics.input_tokens += response.usage.input_tokens
        metrics.output_tokens += response.usage.output_tokens

        tool_blocks = [b for b in response.content if b.type == "tool_use"]

        # No tool calls — agent is done
        if not tool_blocks:
            break

        # Execute tool calls and collect results
        messages.append({"role": "assistant", "content": response.content})

        tool_results = []
        for block in tool_blocks:
            metrics.tool_calls += 1
            result_text, is_error = tool_executor(block.name, block.input, workdir)
            if is_error:
                metrics.errors += 1
            tool_results.append(
                {
                    "type": "tool_result",
                    "tool_use_id": block.id,
                    "content": str(result_text)[:4000],
                    "is_error": is_error,
                }
            )

        messages.append({"role": "user", "content": tool_results})

    metrics.wall_clock_ms = (time.monotonic() - t0) * 1000
    return metrics
