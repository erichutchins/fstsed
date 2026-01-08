# /// script
# dependencies = [
#   "psutil",
# ]
# ///

import subprocess
import time
import os
import json
import random
import string
import psutil

def generate_random_word(length=8):
    return ''.join(random.choices(string.ascii_lowercase, k=length))

def create_corpus(path, size_mb, json_mode=False):
    print(f"Generating {size_mb}MB {'JSON ' if json_mode else ''}corpus...")
    with open(path, 'w') as f:
        words = [generate_random_word() for _ in range(1000)]
        total_bytes = 0
        target_bytes = size_mb * 1024 * 1024
        while total_bytes < target_bytes:
            content = ' '.join(random.choices(words, k=20))
            if json_mode:
                line = json.dumps({"text": content}) + '\n'
            else:
                line = content + '\n'
            f.write(line)
            total_bytes += len(line)

def create_fst_json(path, key_count):
    print(f"Generating JSON for {key_count} keys...")
    keys = []
    with open(path, 'w') as f:
        for i in range(key_count):
            key = f"word_{i}_{generate_random_word()}"
            keys.append(key)
            f.write(json.dumps({"key": key, "value": f"VAL_{i}"}) + '\n')
    return keys

def create_patterns_file(path, keys):
    with open(path, 'w') as f:
        for key in keys:
            f.write(key + '\n')

def run_bench(bin_path, fst_path, corpus_path):
    start = time.time()
    process = subprocess.Popen(
        [bin_path, "-f", fst_path, corpus_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE
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
    
    if process.returncode != 0:
        _, stderr = process.communicate()
        print(f"Error running bench: {stderr.decode()}")
        return None
    return end - start, peak_rss

def run_other_tool(cmd, corpus_path):
    start = time.time()
    process = subprocess.Popen(
        cmd + [corpus_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE
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
    return end - start, peak_rss

def main():
    bin_path = "./target/release/fstsed"
    if not os.path.exists(bin_path):
        print("Release binary not found. Please run 'cargo build --release'")
        return

    corpus_path = "bench_corpus.txt"
    create_corpus(corpus_path, 10) # 10MB corpus

    json_corpus_path = "bench_corpus.json"
    create_corpus(json_corpus_path, 10, json_mode=True) # 10MB JSON corpus

    key_counts = [10, 100, 1000, 10000, 100000]
    results = []

    # Scaling Benchmark
    for count in key_counts:
        json_path = f"bench_{count}.json"
        fst_path = f"bench_{count}.fst"
        patterns_path = f"bench_{count}.txt"
        
        keys = create_fst_json(json_path, count)
        create_patterns_file(patterns_path, keys)
        
        # Build FST
        subprocess.run([bin_path, "--build", "-f", fst_path, json_path], stdout=subprocess.DEVNULL)
        
        # Run Benchmarks
        print(f"\n--- Results for {count} keys ---")
        
        # fstsed
        res = run_bench(bin_path, fst_path, corpus_path)
        if res:
            duration, mem = res
            print(f"fstsed         | Time: {duration:.4f}s | Throughput: {10/duration:.2f} MB/s | Mem: {mem/1024/1024:.2f} MB")
            results.append(("fstsed", count, duration, mem))

        # fstsed --json
        # Treat corpus as JSON strings line-by-line
        res = run_other_tool([bin_path, "-f", fst_path, "--json"], json_corpus_path)
        if res:
            duration, mem = res
            print(f"fstsed --json  | Time: {duration:.4f}s | Throughput: {10/duration:.2f} MB/s | Mem: {mem/1024/1024:.2f} MB")
            results.append(("fstsed --json", count, duration, mem))

        # ripgrep (fixed-string, word-boundary, passthru)
        if count <= 10000:
            res = run_other_tool(["rg", "-F", "-w", "-f", patterns_path, "--passthru"], corpus_path)
            if res:
                duration, mem = res
                print(f"rg -F -w       | Time: {duration:.4f}s | Throughput: {10/duration:.2f} MB/s | Mem: {mem/1024/1024:.2f} MB")
                results.append(("rg -F -w", count, duration, mem))
        else:
            print("rg -F -w       | Skipped (> 10k keys)")

        # ripgrep (fixed-string, passthru)
        if count <= 10000:
            res = run_other_tool(["rg", "-F", "-f", patterns_path, "--passthru"], corpus_path)
            if res:
                duration, mem = res
                print(f"rg -F          | Time: {duration:.4f}s | Throughput: {10/duration:.2f} MB/s | Mem: {mem/1024/1024:.2f} MB")
                results.append(("rg -F", count, duration, mem))
        else:
            print("rg -F          | Skipped (> 10k keys)")

        # grep (fixed-string, word-boundary)
        if count <= 1000:
            res = run_other_tool(["grep", "-F", "-w", "-f", patterns_path], corpus_path)
            if res:
                duration, mem = res
                print(f"grep -F -w     | Time: {duration:.4f}s | Throughput: {10/duration:.2f} MB/s | Mem: {mem/1024/1024:.2f} MB")
                results.append(("grep -F -w", count, duration, mem))
        else:
            print("grep -F -w     | Skipped (> 1k keys)")
        
        # Cleanup
        os.remove(json_path)
        os.remove(fst_path)
        os.remove(patterns_path)

    os.remove(corpus_path)
    os.remove(json_corpus_path)
    
    print("\nSummary (10MB Corpus):")
    print("Tool\tKeys\tTime(s)\tThroughput(MB/s)\tPeak RSS(MB)")
    for tool, count, duration, mem in results:
        print(f"{tool:12}\t{count}\t{duration:.4f}\t{10/duration:.2f}\t{mem/1024/1024:.2f}")

if __name__ == "__main__":
    main()
