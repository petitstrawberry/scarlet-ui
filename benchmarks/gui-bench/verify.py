#!/usr/bin/env python3
"""Check protocol/frame coverage in a ScarletUI gui-bench smoke result."""
import argparse
from collections import Counter
import json
from pathlib import Path


def verify(document):
    assert document["schema_version"] == 1, "unexpected result schema"
    assert document["sources"]["scarlet-ui"]["revision"], "missing ScarletUI revision"
    conditions = [c for c in document["conditions"] if c["framework"] == "scarletui"]
    assert conditions, "no ScarletUI conditions"
    for condition in conditions:
        spec = condition["scenario"]
        label = f'{spec["id"]}/{condition["mode"]}/{condition["repetition"]}'
        assert condition["status"] == "complete", f"{label}: incomplete"
        frames = condition["frames"]
        total = 1 + spec["warmup_frames"] + spec["sample_frames"]
        assert len(frames) == total, f"{label}: incorrect frame count"
        phases = Counter(f["phase"] for f in frames)
        assert phases == Counter(startup=1, warmup=spec["warmup_frames"], measurement=spec["sample_frames"]), label
        assert [f["sequence"] for f in frames] == list(range(total)), label
        assert frames[0]["frame_time_ns"] is None, label
        for previous, frame in zip(frames, frames[1:]):
            assert frame["timestamp_ns"] > previous["timestamp_ns"], label
            assert frame["frame_time_ns"] > 0, label
        for frame in frames:
            assert frame["valid"], f'{label}: invalid frame: {frame["note"]}'
            assert frame["observed_width"] > 0 and frame["observed_height"] > 0, label
            assert frame["framework_memory"] is None, label
            assert all(value is None for key, value in frame["pipeline"].items() if key != "device_gpu_timestamps"), label
            assert frame["pipeline"]["device_gpu_timestamps"] is False, label
            if spec["workload"] not in ("cold_start", "idle"):
                assert frame["stimulus_latency_ns"] > 0, label
        if spec["workload"] in ("resize_step", "resize_storm"):
            assert len({(f["observed_width"], f["observed_height"]) for f in frames}) > 1, f"{label}: no observed resize"
    return len(conditions), sum(len(c["frames"]) for c in conditions)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    args = parser.parse_args()
    count, frames = verify(json.loads(args.result.read_text()))
    print(f"Verified {count} ScarletUI conditions and {frames} valid frame records")
