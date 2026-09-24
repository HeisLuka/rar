#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os

from query import PubHarnessQuery
from store import PubHarnessStore


def dump(value):
    print(json.dumps(value, ensure_ascii=False, indent=2, default=str))


def main() -> int:
    parser = argparse.ArgumentParser(description="Local PUB harness query CLI")
    parser.add_argument("--db", default=os.environ.get("PUB_HARNESS_DB", "pub-harness.db"))
    sub = parser.add_subparsers(dest="command", required=True)

    init = sub.add_parser("init")
    get = sub.add_parser("task-get")
    get.add_argument("task_id")

    nxt = sub.add_parser("task-next")
    nxt.add_argument("--lane", required=True, choices=["local_research", "implementation"])
    nxt.add_argument("--owner")
    nxt.add_argument("--limit", type=int, default=5)

    blockers = sub.add_parser("blockers")
    blockers.add_argument("--priority")
    blockers.add_argument("--limit", type=int, default=100)

    search = sub.add_parser("search")
    search.add_argument("query")
    search.add_argument("--type", dest="entity_type")
    search.add_argument("--limit", type=int, default=25)

    args = parser.parse_args()
    store = PubHarnessStore(args.db)
    store.init_schema()
    query = PubHarnessQuery(store)
    try:
        if args.command == "init":
            dump({"db": str(store.db_path), "initialized": True})
        elif args.command == "task-get":
            dump(query.task_get(args.task_id))
        elif args.command == "task-next":
            dump(query.task_next(lane=args.lane, owner=args.owner, limit=args.limit))
        elif args.command == "blockers":
            dump(query.blockers_list(priority=args.priority, limit=args.limit))
        elif args.command == "search":
            dump(query.search(args.query, entity_type=args.entity_type, limit=args.limit))
    finally:
        store.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
