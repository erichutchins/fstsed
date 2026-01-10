# /// script
# dependencies = [
#   "psutil",
# ]
# ///

import json
import os
import random
import subprocess
import time

import psutil

FLOG_PATH = os.path.expanduser("~/go/bin/flog")


def get_flog_keys(count):
    print(f"Generating {count} realistic keys using flog...")
    # Increase count slightly to account for potential duplicates
    target_count = count if count < 10000 else count + (count // 10)
    cmd = [FLOG_PATH, "-n", str(target_count), "-f", "json"]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    keys = []
    for line in proc.stdout.splitlines():
        if not line:
            continue
        try:
            data = json.loads(line)
            keys.append(data["host"])
        except:
            continue
    unique_keys = []
    seen = set()
    for k in keys:
        if k not in seen:
            unique_keys.append(k)
            seen.add(k)
        if len(unique_keys) >= count:
            break
    return unique_keys


def create_corpus(path, size_mb, all_keys, json_mode=False):
    print(f"Generating {size_mb}MB {'JSON ' if json_mode else ''}corpus with flog...")
    fmt = "json" if json_mode else "apache_combined"
    cmd = [FLOG_PATH, "-b", str(size_mb * 1024 * 1024), "-f", fmt]

    with open(path, "w") as f:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True)
        line_count = 0
        for line in proc.stdout:
            f.write(line)
            line_count += 1
            # Inject a positive match every 100 lines to ensure realistic hit rate (~1%)
            if line_count % 100 == 0:
                key = random.choice(all_keys)
                if json_mode:
                    inj = {
                        "host": key,
                        "user-identifier": "-",
                        "datetime": "09/Jan/2026:18:38:39 -0500",
                        "method": "GET",
                        "request": "/injected",
                        "protocol": "HTTP/1.1",
                        "status": 200,
                        "bytes": 123,
                    }
                    f.write(json.dumps(inj) + "\n")
                else:
                    f.write(
                        f'{key} - - [09/Jan/2026:18:41:47 -0500] "GET /injected HTTP/1.1" 200 123 "-" "Antigravity/1.0"\n'
                    )
        proc.wait()


def create_fst_json(path, keys):
    print(f"Generating FST JSON for {len(keys)} keys...")
    with open(path, "w") as f:
        for i, key in enumerate(keys):
            f.write(json.dumps({"key": key, "value": f"VAL_{i}"}) + "\n")


def create_patterns_file(path, keys):
    with open(path, "w") as f:
        for key in keys:
            f.write(key + "\n")


def run_bench(bin_path, fst_path, corpus_path):
    corpus_size = os.path.getsize(corpus_path)
    start = time.time()
    process = subprocess.Popen(
        [bin_path, "-f", fst_path, corpus_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )

    p = psutil.Process(process.pid)
    peak_rss = 0
    try:
        while process.poll() is None:
            try:
                rss = p.memory_info().rss
                if rss > peak_rss:
                    peak_rss = rss
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                break
            time.sleep(0.01)
    except Exception:
        pass

    process.wait()
    end = time.time()
    duration = end - start

    if process.returncode != 0:
        _, stderr = process.communicate()
        print(f"Error running bench: {stderr.decode()}")
        return None
    return duration, peak_rss, (corpus_size / 1024 / 1024) / duration


def run_other_tool(cmd, corpus_path):
    corpus_size = os.path.getsize(corpus_path)
    start = time.time()
    process = subprocess.Popen(
        cmd + [corpus_path], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )

    p = psutil.Process(process.pid)
    peak_rss = 0
    try:
        while process.poll() is None:
            try:
                rss = p.memory_info().rss
                if rss > peak_rss:
                    peak_rss = rss
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                break
            time.sleep(0.01)
    except Exception:
        pass

    process.wait()
    end = time.time()
    duration = end - start
    return duration, peak_rss, (corpus_size / 1024 / 1024) / duration


def main():
    bin_path = "./target/release/fstsed"
    if not os.path.exists(bin_path):
        print("Release binary not found. Please run 'cargo build --release'")
        return

    # Generate keys pool
    all_keys = get_flog_keys(100000)

    corpus_path = "bench_corpus.txt"
    create_corpus(corpus_path, 100, all_keys)  # 100MB corpus

    json_corpus_path = "bench_corpus.json"
    create_corpus(json_corpus_path, 100, all_keys, json_mode=True)  # 100MB JSON corpus

    key_counts = [10, 100, 1000, 10000, 100000]
    results = []

    # Scaling Benchmark
    for count in key_counts:
        json_path = f"bench_{count}.json"
        fst_path = f"bench_{count}.fst"
        patterns_path = f"bench_{count}.txt"

        current_keys = all_keys[:count]
        create_fst_json(json_path, current_keys)
        create_patterns_file(patterns_path, current_keys)

        # Build FST
        subprocess.run(
            [bin_path, "--build", "-f", fst_path, json_path], stdout=subprocess.DEVNULL
        )

        # Run Benchmarks
        print(f"\n--- Results for {count} keys ---")

        # fstsed
        res = run_bench(bin_path, fst_path, corpus_path)
        if res:
            duration, mem, throughput = res
            print(
                f"fstsed         | Time: {duration:.4f}s | Throughput: {throughput:.2f} MB/s | Mem: {mem / 1024 / 1024:.2f} MB"
            )
            results.append(("fstsed", count, duration, throughput, mem))

        # fstsed --json
        res = run_other_tool([bin_path, "-f", fst_path, "--json"], json_corpus_path)
        if res:
            duration, mem, throughput = res
            print(
                f"fstsed --json  | Time: {duration:.4f}s | Throughput: {throughput:.2f} MB/s | Mem: {mem / 1024 / 1024:.2f} MB"
            )
            results.append(("fstsed --json", count, duration, throughput, mem))

        # ripgrep (fixed-string, word-boundary, passthru)
        if count <= 100000:
            res = run_other_tool(
                ["rg", "-F", "-w", "-f", patterns_path, "--passthru"], corpus_path
            )
            if res:
                duration, mem, throughput = res
                print(
                    f"rg -F -w       | Time: {duration:.4f}s | Throughput: {throughput:.2f} MB/s | Mem: {mem / 1024 / 1024:.2f} MB"
                )
                results.append(("rg -F -w", count, duration, throughput, mem))
        else:
            print("rg -F -w       | Skipped (> 100k keys)")

        # ripgrep (fixed-string, passthru)
        if count <= 100000:
            res = run_other_tool(
                ["rg", "-F", "-f", patterns_path, "--passthru"], corpus_path
            )
            if res:
                duration, mem, throughput = res
                print(
                    f"rg -F          | Time: {duration:.4f}s | Throughput: {throughput:.2f} MB/s | Mem: {mem / 1024 / 1024:.2f} MB"
                )
                results.append(("rg -F", count, duration, throughput, mem))
        else:
            print("rg -F          | Skipped (> 100k keys)")

        # grep (fixed-string, word-boundary)
        if count <= 100:
            res = run_other_tool(["grep", "-F", "-w", "-f", patterns_path], corpus_path)
            if res:
                duration, mem, throughput = res
                print(
                    f"grep -F -w     | Time: {duration:.4f}s | Throughput: {throughput:.2f} MB/s | Mem: {mem / 1024 / 1024:.2f} MB"
                )
                results.append(("grep -F -w", count, duration, throughput, mem))
        else:
            print("grep -F -w     | Skipped (> 1k keys)")

        # Cleanup
        os.remove(json_path)
        os.remove(fst_path)
        os.remove(patterns_path)

    os.remove(corpus_path)
    os.remove(json_corpus_path)

    print("\nSummary Table:")
    print(
        f"{'Tool':15} | {'Keys':>7} | {'Time(s)':>8} | {'MB/s':>8} | {'RSS(MB)':>8} | {'vs Prev':>8}"
    )
    print("-" * 67)

    prev_durations = {}

    for tool, count, duration, throughput, mem in results:
        diff_str = "-"
        if tool in prev_durations:
            ratio = duration / prev_durations[tool]
            diff_str = f"{ratio:.2f}x"
        prev_durations[tool] = duration

        print(
            f"{tool:15} | {count:7,d} | {duration:8.4f} | {throughput:8.2f} | {mem / 1024 / 1024:8.2f} | {diff_str:>8}"
        )


if __name__ == "__main__":
    main()
