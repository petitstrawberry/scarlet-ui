#!/usr/bin/env python3
"""Validate matched GUI runs and summarize repetition-level CPU frame intervals."""
import argparse
from collections import defaultdict
import csv
import json
from pathlib import Path
from statistics import median

FRAMEWORKS = ("zgui", "gpui", "scarletui")


def percentile(values, fraction):
    values = sorted(values)
    return values[int((len(values) - 1) * fraction)] if values else None


def summarize(document, repetitions=3):
    groups = defaultdict(dict)
    seen = set()
    for condition in document["conditions"]:
        if condition["mode"] != "headline":
            continue
        framework = condition["framework"]
        if framework not in FRAMEWORKS:
            continue
        spec = condition["scenario"]
        identity = (spec["id"], framework, condition["repetition"])
        assert identity not in seen, f"duplicate condition: {identity}"
        seen.add(identity)
        entry = groups[spec["id"]].setdefault(framework, {"scenario": spec, "runs": [], "scales_milli": set()})
        assert entry["scenario"] == spec, identity
        frames = condition["frames"]
        if condition["status"] != "complete":
            entry["runs"].append({"repetition": condition["repetition"], "status": condition["status"], "samples": len(frames)})
            continue
        total = 1 + spec["warmup_frames"] + spec["sample_frames"]
        assert len(frames) == total, f"missing frames: {identity}"
        assert [f["sequence"] for f in frames] == list(range(total)), identity
        assert sum(f["phase"] == "warmup" for f in frames) == spec["warmup_frames"], identity
        measurement = [f for f in frames if f["phase"] == "measurement"]
        assert len(measurement) == spec["sample_frames"], identity
        assert all(f["valid"] and f["frame_time_ns"] > 0 for f in measurement), f"invalid measurement frames: {identity}"
        assert all(b["timestamp_ns"] > a["timestamp_ns"] for a, b in zip(frames, frames[1:])), identity
        sizes = {(f["observed_width"], f["observed_height"]) for f in measurement}
        scales = {f.get("output_scale_milli") for f in measurement}
        assert len(scales) == 1 and None not in scales, f"missing/changing output scale: {identity}"
        resize = spec["workload"] in ("resize_step", "resize_storm")
        size_matches = len(sizes) > 1 if resize else sizes == {(spec["width"], spec["height"])}
        assert size_matches, f"unmatched observed size: {identity}: {sizes}"
        entry["scales_milli"].update(scales)
        intervals = [f["frame_time_ns"] / 1e6 for f in measurement]
        latency = [f["stimulus_latency_ns"] / 1e6 for f in measurement if f["stimulus_latency_ns"] is not None]
        entry["runs"].append({
            "repetition": condition["repetition"], "status": "complete", "samples": len(intervals),
            "frame_p50_ms": percentile(intervals, .5), "frame_p95_ms": percentile(intervals, .95),
            "latency_p95_ms": percentile(latency, .95),
        })
    rows = []
    assert groups, "no headline comparison groups"
    for scenario, frameworks in groups.items():
        assert set(frameworks) == set(FRAMEWORKS), f"missing framework: {scenario}"
        all_scales = set().union(*(entry["scales_milli"] for entry in frameworks.values()))
        matched_scale = len(all_scales) <= 1
        assert len({json.dumps(entry["scenario"], sort_keys=True) for entry in frameworks.values()}) == 1, scenario
        for framework in FRAMEWORKS:
            entry = frameworks[framework]
            runs = sorted(entry["runs"], key=lambda run: run["repetition"])
            assert [run["repetition"] for run in runs] == list(range(repetitions)), f"missing repetitions: {scenario}/{framework}"
            spec = entry["scenario"]
            complete = matched_scale and all(run["status"] == "complete" for run in runs)
            p50 = [run["frame_p50_ms"] for run in runs] if complete else []
            latency = [run["latency_p95_ms"] for run in runs if run["latency_p95_ms"] is not None] if complete else []
            rows.append({
                "scenario": scenario, "framework": framework, "nodes": spec["nodes"],
                "workload": spec["workload"], "style": spec["style"], "scale_milli": next(iter(entry["scales_milli"])) if len(entry["scales_milli"]) == 1 else None, "observed_scales_milli": sorted(entry["scales_milli"]),
                "repetitions": len(runs), "complete_repetitions": sum(run["status"] == "complete" for run in runs),
                "status": "incomparable_scale" if not matched_scale else ("complete" if complete else "incomplete"), "samples_per_repetition": spec["sample_frames"],
                "frame_p50_ms": median(p50) if p50 else None, "frame_p50_min_ms": min(p50) if p50 else None, "frame_p50_max_ms": max(p50) if p50 else None,
                "frame_p95_ms": median(run["frame_p95_ms"] for run in runs) if complete else None,
                "latency_p95_ms": median(latency) if latency else None, "runs": runs,
                "comparison_scope": "native-style characterization" if spec["style"] == "decorated" else "matched minimal workload",
            })
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = summarize(json.loads(args.result.read_text()), args.repetitions)
    args.output.write_text(json.dumps({
        "method": "Median of per-repetition percentiles; percentile index=floor((n-1)*q). Bounds are the min/max of repetition medians, not confidence intervals. Failed repetitions suppress that framework/scenario's headline percentiles; differing output scales suppress all frameworks' headline comparison for the scenario. Original statuses remain in runs.",
        "units": "milliseconds, CPU-side observed frame/submission cadence; not GPU execution or scanout",
        "rows": rows,
    }, indent=2) + "\n")
    columns = [key for key in rows[0] if key != "runs"]
    with args.output.with_suffix(".csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    incomplete = sum(row["status"] == "incomplete" for row in rows)
    incomparable = sum(row["status"] == "incomparable_scale" for row in rows)
    print(f"Checked {len(rows) // len(FRAMEWORKS)} scenarios across {len(FRAMEWORKS)} frameworks, {args.repetitions} repetitions each; headline percentiles withheld for {incomplete} incomplete and {incomparable} scale-incomparable framework/scenario cells")
