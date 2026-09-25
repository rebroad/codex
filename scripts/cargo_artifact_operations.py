#!/usr/bin/env python3
"""List recorded Cargo artifact operations from the build-tree ledger."""

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--fingerprint")
    parser.add_argument("--target-dir")
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args()

    if not args.ledger.exists():
        return 0

    records = []
    for line in args.ledger.read_text(encoding="utf-8").splitlines():
        record = json.loads(line)
        if args.fingerprint and record["fingerprint"] != args.fingerprint:
            continue
        if args.target_dir and record["target_dir"] != args.target_dir:
            continue
        records.append(record)

    if args.as_json:
        print(json.dumps(records, indent=2, sort_keys=True))
        return 0

    for record in records:
        print(
            f"{record['timestamp']} {record['fingerprint']} "
            f"{record['operation']} {record['target_dir']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
