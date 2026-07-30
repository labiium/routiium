#!/usr/bin/env python3
"""Small Routiium load harness with latency and TTFB gates.

This intentionally uses only the Python standard library so it can run in CI,
release smoke tests, and production jump boxes without extra package setup.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import statistics
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Iterable


@dataclass(frozen=True)
class Sample:
    ok: bool
    status: int
    latency_ms: float
    ttfb_ms: float
    bytes_read: int
    error: str | None = None


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    ordered = sorted(values)
    rank = (len(ordered) - 1) * pct
    lower = int(rank)
    upper = min(lower + 1, len(ordered) - 1)
    weight = rank - lower
    return ordered[lower] * (1 - weight) + ordered[upper] * weight


def request_once(args: argparse.Namespace) -> Sample:
    payload = {
        "model": args.model,
        "stream": args.stream,
        "messages": [{"role": "user", "content": args.prompt}],
    }
    body = json.dumps(payload).encode("utf-8")
    headers = {
        "content-type": "application/json",
        "user-agent": "routiium-load-harness/1",
    }
    if args.api_key:
        headers["authorization"] = f"Bearer {args.api_key}"

    request = urllib.request.Request(
        f"{args.base_url.rstrip('/')}/v1/chat/completions",
        data=body,
        headers=headers,
        method="POST",
    )

    started = time.perf_counter()
    first_byte_at: float | None = None
    bytes_read = 0
    status = 0
    try:
        with urllib.request.urlopen(request, timeout=args.timeout_seconds) as response:
            status = response.status
            while True:
                chunk = response.read(args.read_size)
                if first_byte_at is None:
                    first_byte_at = time.perf_counter()
                if not chunk:
                    break
                bytes_read += len(chunk)
                if not args.stream:
                    bytes_read += len(response.read())
                    break
        ended = time.perf_counter()
        ttfb_at = first_byte_at or ended
        return Sample(
            ok=200 <= status < 300,
            status=status,
            latency_ms=(ended - started) * 1000,
            ttfb_ms=(ttfb_at - started) * 1000,
            bytes_read=bytes_read,
        )
    except urllib.error.HTTPError as exc:
        ended = time.perf_counter()
        return Sample(
            ok=False,
            status=exc.code,
            latency_ms=(ended - started) * 1000,
            ttfb_ms=(ended - started) * 1000,
            bytes_read=0,
            error=str(exc),
        )
    except Exception as exc:  # noqa: BLE001 - CLI reports transport failures.
        ended = time.perf_counter()
        return Sample(
            ok=False,
            status=0,
            latency_ms=(ended - started) * 1000,
            ttfb_ms=(ended - started) * 1000,
            bytes_read=0,
            error=str(exc),
        )


def run_load(args: argparse.Namespace) -> list[Sample]:
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as executor:
        futures: Iterable[concurrent.futures.Future[Sample]] = (
            executor.submit(request_once, args) for _ in range(args.requests)
        )
        return [future.result() for future in concurrent.futures.as_completed(futures)]


def summarize(samples: list[Sample]) -> dict[str, object]:
    latencies = [sample.latency_ms for sample in samples]
    ttfbs = [sample.ttfb_ms for sample in samples]
    failures = [sample for sample in samples if not sample.ok]
    return {
        "requests": len(samples),
        "successes": len(samples) - len(failures),
        "failures": len(failures),
        "latency_ms": {
            "p50": percentile(latencies, 0.50),
            "p95": percentile(latencies, 0.95),
            "p99": percentile(latencies, 0.99),
            "avg": statistics.fmean(latencies) if latencies else 0.0,
        },
        "ttfb_ms": {
            "p50": percentile(ttfbs, 0.50),
            "p95": percentile(ttfbs, 0.95),
            "p99": percentile(ttfbs, 0.99),
            "avg": statistics.fmean(ttfbs) if ttfbs else 0.0,
        },
        "bytes_read": sum(sample.bytes_read for sample in samples),
        "failure_examples": [
            {"status": sample.status, "error": sample.error}
            for sample in failures[:5]
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:8088")
    parser.add_argument("--api-key", default="")
    parser.add_argument("--model", default="auto")
    parser.add_argument("--prompt", default="Reply with one short sentence.")
    parser.add_argument("--requests", type=int, default=50)
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--stream", action="store_true")
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    parser.add_argument("--read-size", type=int, default=4096)
    parser.add_argument("--max-p95-ms", type=float, default=0.0)
    parser.add_argument("--max-p99-ms", type=float, default=0.0)
    parser.add_argument("--max-ttfb-p99-ms", type=float, default=0.0)
    args = parser.parse_args()

    if args.requests < 1 or args.concurrency < 1:
        parser.error("--requests and --concurrency must be positive")

    samples = run_load(args)
    summary = summarize(samples)
    print(json.dumps(summary, indent=2, sort_keys=True))

    failed = summary["failures"] != 0
    latency = summary["latency_ms"]
    ttfb = summary["ttfb_ms"]
    if args.max_p95_ms and latency["p95"] > args.max_p95_ms:
        failed = True
    if args.max_p99_ms and latency["p99"] > args.max_p99_ms:
        failed = True
    if args.max_ttfb_p99_ms and ttfb["p99"] > args.max_ttfb_p99_ms:
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
