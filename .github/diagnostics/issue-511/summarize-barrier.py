import json, sys
for path in sys.argv[1:]:
    rounds = [json.loads(line) for line in open(path)]
    waits = sorted(r["marker_wait_ms"] for r in rounds)
    print(f"{path}: {len(rounds)} rounds")
    print(f"  late setup events reaching the real watcher: {sum(r['late_config_events'] > 0 for r in rounds)} rounds")
    print(f"  real-watcher events numbered at or below the marker: {sum(r['real_events_at_or_below_marker'] for r in rounds)}")
    print(f"  rounds where the marker's own watcher got late setup events: {sum(r['setup_seen_by_marker_stream'] > 0 for r in rounds)}")
    print(f"  ...of which numbered after the marker: {sum(r['setup_numbered_after_marker'] for r in rounds)}")
    print(f"  marker wait p50={waits[len(waits)//2]:.1f} p99={waits[int(len(waits)*.99)]:.1f} max={waits[-1]:.1f} ms")
