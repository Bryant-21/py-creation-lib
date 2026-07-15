from __future__ import annotations

import argparse
import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path


TIMESTAMP_RE = re.compile(r"^\[\d{2}:\d{2}\]\s+")
DETAIL_RE = re.compile(r"^\[\d{2}:\d{2}\]\s{4,}(.*\S)\s*$")
TYPED_FORM_ID_RE = re.compile(r"\[([A-Z0-9_]{4}):[0-9A-F]{8}\]")
PLAIN_FORM_ID_RE = re.compile(r"\[[0-9A-F]{8}\]")
DECIMAL_BRACKET_RE = re.compile(r"\[(\d+)\]")


@dataclass
class ErrorGroup:
    count: int = 0
    samples: list[str] = field(default_factory=list)

    def add(self, sample: str) -> None:
        self.count += 1
        if len(self.samples) < 3 and sample not in self.samples:
            self.samples.append(sample)


def normalize_line(line: str) -> str:
    return re.sub(r"\s+", " ", line.strip())


def strip_timestamp(line: str) -> str:
    return TIMESTAMP_RE.sub("", line).rstrip()


def normalize_error_type(detail: str) -> str:
    detail = normalize_line(detail)
    detail = TYPED_FORM_ID_RE.sub(r"[\1:FORM_ID]", detail)
    detail = PLAIN_FORM_ID_RE.sub("[FORM_ID]", detail)
    detail = DECIMAL_BRACKET_RE.sub("[N]", detail)
    return detail


def read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8", errors="replace").splitlines()


def extract_error_blocks(path: Path) -> list[tuple[str, str]]:
    blocks: list[tuple[str, str]] = []
    current_record: str | None = None

    for raw_line in read_lines(path):
        detail_match = DETAIL_RE.match(raw_line)
        if detail_match:
            if current_record is not None:
                blocks.append((current_record, raw_line.rstrip()))
            continue

        content = strip_timestamp(raw_line).strip()
        if not content:
            continue
        if content.startswith("Start:") or content.startswith("Checking for Errors in "):
            current_record = None
            continue

        current_record = raw_line.rstrip()

    return blocks


def collect_error_groups(path: Path) -> dict[str, ErrorGroup]:
    groups: dict[str, ErrorGroup] = defaultdict(ErrorGroup)
    for record_line, detail_line in extract_error_blocks(path):
        detail = normalize_line(DETAIL_RE.match(detail_line).group(1))
        error_type = normalize_error_type(detail)
        sample = f"{strip_timestamp(record_line)} | {detail}"
        groups[error_type].add(sample)
    return dict(groups)


def count_total_lines(path: Path) -> int:
    return len(read_lines(path))


def count_error_detail_lines(path: Path) -> int:
    return sum(1 for line in read_lines(path) if DETAIL_RE.match(line))


def format_block(block: tuple[str, str]) -> str:
    return f"{block[0]}\n{block[1]}"


def build_report(roundtrip_path: Path, standard_path: Path) -> str:
    roundtrip_lines = read_lines(roundtrip_path)
    standard_lines = read_lines(standard_path)
    roundtrip_blocks = extract_error_blocks(roundtrip_path)
    standard_blocks = extract_error_blocks(standard_path)
    roundtrip_groups = collect_error_groups(roundtrip_path)
    standard_groups = collect_error_groups(standard_path)

    roundtrip_line_set = set(roundtrip_lines)
    standard_line_set = set(standard_lines)
    roundtrip_block_counter = Counter(roundtrip_blocks)
    standard_block_counter = Counter(standard_blocks)

    raw_lines_only_in_roundtrip = sorted(roundtrip_line_set - standard_line_set)
    raw_lines_only_in_standard = sorted(standard_line_set - roundtrip_line_set)
    raw_blocks_only_in_roundtrip = sorted(
        set(roundtrip_block_counter) - set(standard_block_counter),
        key=lambda item: (item[0], item[1]),
    )
    raw_blocks_only_in_standard = sorted(
        set(standard_block_counter) - set(roundtrip_block_counter),
        key=lambda item: (item[0], item[1]),
    )

    unique_signatures = sorted(set(roundtrip_groups) - set(standard_groups))

    lines = [
        f"Roundtrip file: {roundtrip_path}",
        f"Standard file: {standard_path}",
        "",
        "Direct file comparison:",
        f"- Roundtrip total lines: {count_total_lines(roundtrip_path)}",
        f"- Standard total lines: {count_total_lines(standard_path)}",
        f"- Raw lines only in roundtrip: {len(raw_lines_only_in_roundtrip)}",
        f"- Raw lines only in standard: {len(raw_lines_only_in_standard)}",
        "",
        "Direct error-block comparison:",
        f"- Roundtrip error blocks: {len(roundtrip_blocks)}",
        f"- Standard error blocks: {len(standard_blocks)}",
        f"- Raw error blocks only in roundtrip: {len(raw_blocks_only_in_roundtrip)}",
        f"- Raw error blocks only in standard: {len(raw_blocks_only_in_standard)}",
        "",
        "Consolidated type comparison:",
        f"- Roundtrip error-detail lines: {count_error_detail_lines(roundtrip_path)}",
        f"- Standard error-detail lines: {count_error_detail_lines(standard_path)}",
        f"- Unique consolidated error types in roundtrip: {len(unique_signatures)}",
        "",
    ]

    if raw_blocks_only_in_roundtrip:
        lines.append("Sample raw error blocks only in roundtrip:")
        for block in raw_blocks_only_in_roundtrip[:20]:
            lines.append(format_block(block))
            lines.append("")

    if raw_blocks_only_in_standard:
        lines.append("Sample raw error blocks only in standard:")
        for block in raw_blocks_only_in_standard[:20]:
            lines.append(format_block(block))
            lines.append("")

    if unique_signatures:
        lines.append("Consolidated error types only in roundtrip:")
        for index, signature in enumerate(unique_signatures, start=1):
            group = roundtrip_groups[signature]
            lines.append(f"{index}. {signature}")
            lines.append(f"   Occurrences in roundtrip: {group.count}")
            for sample in group.samples:
                lines.append(f"   Example: {sample}")
            lines.append("")
    else:
        lines.append("No consolidated error types appear only in roundtrip.")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Compare two xEdit-style error logs directly and also by consolidated error type."
        )
    )
    parser.add_argument(
        "--roundtrip",
        type=Path,
        required=True,
        help="Path to the roundtrip error log.",
    )
    parser.add_argument(
        "--standard",
        type=Path,
        required=True,
        help="Path to the standard error log.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="Where to write the report.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    report = build_report(args.roundtrip, args.standard)
    args.output.write_text(report, encoding="utf-8")
    print(f"Wrote report to: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
