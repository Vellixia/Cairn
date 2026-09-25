#!/usr/bin/env python3
"""Count Rust source lines excluding complete `#[cfg(test)]` items.

This is a small lexer, not a brace-counting text filter: comments, rustdoc,
normal strings, chars, and raw strings are masked before attributes/items are
located.  It deliberately counts physical lines, matching `wc -l`.
"""
from __future__ import annotations

import pathlib
import re
import signal
import sys


def mask_rust(text: str) -> str:
    """Keep code/newlines; replace comments and literals with spaces."""
    out = list(text)
    i, state, raw_hashes = 0, "code", 0
    while i < len(text):
        if state == "code":
            if text.startswith("//", i):
                state = "line_comment"; out[i:i + 2] = "  "; i += 2; continue
            if text.startswith("/*", i):
                state = "block_comment"; out[i:i + 2] = "  "; i += 2; continue
            raw = re.match(r'(?:br|rb|r)(#{0,})"', text[i:])
            if raw:
                length = len(raw.group(0)); raw_hashes = len(raw.group(1))
                out[i:i + length] = " " * length; i += length; state = "raw"; continue
            if text[i] == '"':
                out[i] = " "; i += 1; state = "string"; continue
            # A lifetime (`'a`) is code, not a character literal. Rust chars
            # are one scalar or escape sequence, so require a nearby close.
            if text[i] == "'" and re.match(r"'(?:\\\\.|[^'\\\\\n]){1,4}'", text[i:]):
                out[i] = " "; i += 1; state = "char"; continue
            i += 1; continue
        if state == "line_comment":
            if text[i] == "\n": state = "code"
            else: out[i] = " "
            i += 1; continue
        if state == "block_comment":
            if text.startswith("*/", i): out[i:i + 2] = "  "; i += 2; state = "code"
            else:
                if text[i] != "\n": out[i] = " "
                i += 1
            continue
        if state in ("string", "char"):
            if text[i] == "\\":
                out[i] = " "
                if i + 1 < len(text) and text[i + 1] != "\n": out[i + 1] = " "
                i += 2; continue
            closing = '"' if state == "string" else "'"
            if text[i] == closing:
                out[i] = " "; i += 1; state = "code"
            else:
                if text[i] != "\n": out[i] = " "
                i += 1
            continue
        # raw
        end = '"' + ('#' * raw_hashes)
        if text.startswith(end, i):
            out[i:i + len(end)] = " " * len(end); i += len(end); state = "code"
        else:
            if text[i] != "\n": out[i] = " "
            i += 1
    return "".join(out)


def excluded_lines(text: str) -> set[int]:
    masked = mask_rust(text)
    excluded: set[int] = set()
    for match in re.finditer(r'(?m)^[ \t]*#\[cfg\(test\)\][ \t]*$', masked):
        start, pos = match.start(), match.end()
        while pos < len(masked) and masked[pos].isspace(): pos += 1
        item_start = pos
        brace = masked.find("{", pos)
        semi = masked.find(";", pos)
        if semi != -1 and (brace == -1 or semi < brace):
            end = semi + 1
        elif brace != -1:
            depth, cursor = 0, brace
            while cursor < len(masked):
                if masked[cursor] == "{": depth += 1
                elif masked[cursor] == "}":
                    depth -= 1
                    if depth == 0:
                        cursor += 1
                        while cursor < len(masked) and masked[cursor] in " \t": cursor += 1
                        end = cursor + 1 if cursor < len(masked) and masked[cursor] == ";" else cursor
                        break
                cursor += 1
            else: raise ValueError("unclosed #[cfg(test)] item")
        else:
            raise ValueError("cannot find #[cfg(test)] item")
        first = text.count("\n", 0, start) + 1
        last = text.count("\n", 0, max(item_start, end - 1)) + 1
        excluded.update(range(first, last + 1))
    return excluded


def count(paths: list[pathlib.Path]) -> int:
    total = 0
    for path in paths:
        text = path.read_text()
        total += text.count("\n") - len(excluded_lines(text))
    return total


def self_check() -> None:
    fixture = pathlib.Path(__file__).with_name("fixtures") / "v1-rust-loc.rs"
    text = fixture.read_text()
    # Five source lines remain: rustdoc, two literals and comments each contain
    # attribute/braces text but must stay production.
    assert count([fixture]) == 5, count([fixture])
    assert len(excluded_lines(text)) == 10, excluded_lines(text)


def table_names(paths: list[pathlib.Path]) -> None:
    """Print table names from SQL migrations plus non-test Rust creator code."""
    expanded = [child for path in paths for child in (path.rglob("*") if path.is_dir() else [path]) if child.is_file()]
    for path in expanded:
        text = path.read_text()
        if path.suffix == ".rs":
            excluded = excluded_lines(text)
            text = "".join(
                line if number not in excluded else "\n"
                for number, line in enumerate(text.splitlines(keepends=True), 1)
            )
        for name in re.findall(
            r"\bCREATE\s+(?:VIRTUAL\s+)?TABLE(?:\s+IF\s+NOT\s+EXISTS)?\s+([A-Za-z_][A-Za-z0-9_]*)",
            text,
            re.I,
        ):
            print(name)


if __name__ == "__main__":
    # A malformed lexer must fail, never leave an unbounded baseline process.
    if hasattr(signal, "SIGALRM"):
        signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("Rust lexer timed out")))
        signal.alarm(30)
    try:
        if sys.argv[1:] == ["--self-check"]:
            self_check()
        elif sys.argv[1] == "--table-names":
            table_names([pathlib.Path(path) for path in sys.argv[2:]])
        else:
            print(count([pathlib.Path(path) for path in sys.argv[1:]]))
    finally:
        if hasattr(signal, "SIGALRM"):
            signal.alarm(0)
