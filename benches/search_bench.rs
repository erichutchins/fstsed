//! Criterion benchmarks for fstsed search operations.
//!
//! Mirrors the structure of `benchmarks/performance.py`:
//! - Fixed corpus size (~1MB for fast iteration)
//! - Varying FST dictionary sizes: 10, 100, 1K, 10K, 100K keys
//! - Reports throughput in MB/s
//!
//! Uses `fake-rs` to generate realistic hostnames, IPs, and log-like data
//! similar to what `flog` produces.

use std::io::Cursor;

use camino::Utf8PathBuf;
use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use fake::faker::internet::en::{DomainSuffix, IPv4, Username};
use fake::faker::lorem::en::Sentence;
use fake::rand::rngs::StdRng;
use fake::rand::{Rng, SeedableRng};
use fake::Fake;
use fstsed::builder::build_fstsed;
use fstsed::json::jsonquotes_range_iter;
use fstsed::matches::FstMatch;
use fstsed::search::{FstCache, FstSed};
use fstsed::template::Template;
use std::borrow::Cow;
use std::hint::black_box;
use termcolor::ColorChoice;

/// Fixed seed for reproducible benchmarks.
const SEED: [u8; 32] = [
    42, 0, 0, 0, 23, 0, 0, 0, 200, 1, 0, 0, 210, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0,
];

/// Key counts to benchmark, matching performance.py
const KEY_COUNTS: &[usize] = &[10, 100, 1_000, 10_000, 100_000];

/// Target corpus size in bytes (~1MB for quick benchmarks)
const CORPUS_SIZE_BYTES: usize = 1024 * 1024;

/// Generates realistic hostname-like keys similar to flog output.
/// These look like: "user123.example.com", "192.168.1.1", etc.
fn generate_flog_style_keys(rng: &mut StdRng, count: usize) -> Vec<String> {
    let mut keys = Vec::with_capacity(count * 2); // Extra capacity for dedup

    while keys.len() < count * 2 {
        // Mix of different key styles like flog generates
        let key: String = match keys.len() % 4 {
            0 => {
                // hostname style: user.domain.suffix
                let user: String = Username().fake_with_rng(rng);
                let domain: String = DomainSuffix().fake_with_rng(rng);
                format!("{}.example.{}", user.to_lowercase(), domain)
            }
            1 => {
                // IP address
                IPv4().fake_with_rng(rng)
            }
            2 => {
                // subdomain style
                let user: String = Username().fake_with_rng(rng);
                format!("{}.internal.local", user.to_lowercase())
            }
            _ => {
                // server name style
                let num: u32 = rng.random_range(0..1000);
                format!("server{:03}.datacenter.net", num)
            }
        };
        keys.push(key);
    }

    // Deduplicate and sort for FST building
    keys.sort();
    keys.dedup();
    keys.truncate(count);
    keys
}

/// Generates NDJSON input for FST building from dictionary keys.
fn generate_ndjson_dictionary(keys: &[String]) -> String {
    keys.iter()
        .map(|key| {
            format!(
                r#"{{"key": "{}", "value": "VAL_{:05}"}}"#,
                key.replace('"', "\\\""),
                keys.len() % 100000
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generates Apache combined log format corpus with injected keys.
/// Similar to flog's apache_combined output.
fn generate_apache_log_corpus(rng: &mut StdRng, keys: &[String], target_size: usize) -> String {
    let mut corpus = String::with_capacity(target_size + 1024);
    let mut line_count = 0;

    while corpus.len() < target_size {
        // Generate a realistic Apache log line
        let ip: String = IPv4().fake_with_rng(rng);
        let user: String = Username().fake_with_rng(rng);
        let status = match line_count % 10 {
            0 => 404,
            1 => 500,
            _ => 200,
        };
        let bytes = 100 + (line_count % 10000);

        let line = format!(
            r#"{} - {} [09/Jan/2026:18:38:39 -0500] "GET /path/to/resource/{} HTTP/1.1" {} {} "-" "Mozilla/5.0""#,
            ip, user, line_count, status, bytes
        );
        corpus.push_str(&line);
        corpus.push('\n');

        // Inject a positive match every ~100 lines for ~1% hit rate
        if line_count % 100 == 0 && !keys.is_empty() {
            let key_idx = line_count % keys.len();
            let injected = format!(
                r#"{} - - [09/Jan/2026:18:41:47 -0500] "GET /injected HTTP/1.1" 200 123 "-" "Benchmark/1.0""#,
                keys[key_idx]
            );
            corpus.push_str(&injected);
            corpus.push('\n');
        }

        line_count += 1;
    }

    corpus
}

/// Generates JSON log corpus with injected keys.
/// Similar to flog's json output.
fn generate_json_log_corpus(rng: &mut StdRng, keys: &[String], target_size: usize) -> String {
    let mut corpus = String::with_capacity(target_size + 1024);
    let mut line_count = 0;

    while corpus.len() < target_size {
        let ip: String = IPv4().fake_with_rng(rng);
        let user: String = Username().fake_with_rng(rng);
        let status = match line_count % 10 {
            0 => 404,
            1 => 500,
            _ => 200,
        };
        let bytes = 100 + (line_count % 10000);

        let line = format!(
            r#"{{"host": "{}", "user-identifier": "{}", "datetime": "09/Jan/2026:18:38:39 -0500", "method": "GET", "request": "/path/{}", "protocol": "HTTP/1.1", "status": {}, "bytes": {}}}"#,
            ip, user, line_count, status, bytes
        );
        corpus.push_str(&line);
        corpus.push('\n');

        // Inject a positive match every ~100 lines
        if line_count % 100 == 0 && !keys.is_empty() {
            let key_idx = line_count % keys.len();
            let injected = format!(
                r#"{{"host": "{}", "user-identifier": "-", "datetime": "09/Jan/2026:18:41:47 -0500", "method": "GET", "request": "/injected", "protocol": "HTTP/1.1", "status": 200, "bytes": 123}}"#,
                keys[key_idx]
            );
            corpus.push_str(&injected);
            corpus.push('\n');
        }

        line_count += 1;
    }

    corpus
}

/// Creates a temporary FST file from the given NDJSON dictionary.
fn create_temp_fst(ndjson: &str) -> (tempfile::TempDir, Utf8PathBuf) {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let fst_path =
        Utf8PathBuf::from_path_buf(temp_dir.path().join("bench.fst")).expect("Invalid UTF-8 path");

    let cursor = Cursor::new(ndjson);
    build_fstsed(cursor, "key", &fst_path, true).expect("Failed to build FST");

    (temp_dir, fst_path)
}

/// Benchmark: fstsed throughput with varying key counts (plain text mode).
///
/// This mirrors the main benchmark in performance.py:
/// - Fixed ~1MB corpus
/// - Key counts: 10, 100, 1K, 10K, 100K
fn bench_fstsed_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("fstsed");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(50);

    // Generate the full key pool once (100K keys)
    let mut rng = StdRng::from_seed(SEED);
    let all_keys = generate_flog_style_keys(&mut rng, 100_000);

    // Generate fixed-size corpus with all keys available for injection
    let mut rng = StdRng::from_seed(SEED);
    let corpus = generate_apache_log_corpus(&mut rng, &all_keys, CORPUS_SIZE_BYTES);
    let corpus_bytes = corpus.as_bytes();

    group.throughput(Throughput::Bytes(corpus_bytes.len() as u64));

    for &key_count in KEY_COUNTS {
        // Take subset of keys for this benchmark
        let keys: Vec<String> = all_keys.iter().take(key_count).cloned().collect();
        let ndjson = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        group.bench_with_input(BenchmarkId::new("keys", key_count), &fstsed, |b, fstsed| {
            let mut cache = FstCache::new();
            b.iter(|| {
                let count = fstsed.find_iter(corpus_bytes, &mut cache).count();
                black_box(count)
            });
        });
    }

    group.finish();
}

/// Benchmark: fstsed --json mode throughput with varying key counts.
///
/// Tests searching within JSON string values only.
fn bench_fstsed_json_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("fstsed_json");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(50);

    // Generate the full key pool once (100K keys)
    let mut rng = StdRng::from_seed(SEED);
    let all_keys = generate_flog_style_keys(&mut rng, 100_000);

    // Generate fixed-size JSON corpus
    let mut rng = StdRng::from_seed(SEED);
    let corpus = generate_json_log_corpus(&mut rng, &all_keys, CORPUS_SIZE_BYTES);
    let corpus_bytes = corpus.as_bytes();

    group.throughput(Throughput::Bytes(corpus_bytes.len() as u64));

    for &key_count in KEY_COUNTS {
        let keys: Vec<String> = all_keys.iter().take(key_count).cloned().collect();
        let ndjson = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        group.bench_with_input(BenchmarkId::new("keys", key_count), &fstsed, |b, fstsed| {
            let mut cache = FstCache::new();
            b.iter(|| {
                // Simulate --json mode: only search within JSON string ranges
                let mut match_count = 0;
                for (start, end, _has_esc) in jsonquotes_range_iter(corpus_bytes) {
                    let slice = &corpus_bytes[start..end];
                    match_count += fstsed.find_iter(slice, &mut cache).count();
                }
                black_box(match_count)
            });
        });
    }

    group.finish();
}

/// Benchmark: jsonquotes_range_iter parsing speed.
///
/// Tests the JSON string boundary detection algorithm.
fn bench_jsonquotes_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("jsonquotes");
    group.sampling_mode(SamplingMode::Flat);

    // Generate JSON corpus
    let mut rng = StdRng::from_seed(SEED);
    let keys = generate_flog_style_keys(&mut rng, 1000);
    let corpus = generate_json_log_corpus(&mut rng, &keys, CORPUS_SIZE_BYTES);
    let corpus_bytes = corpus.as_bytes();

    group.throughput(Throughput::Bytes(corpus_bytes.len() as u64));

    group.bench_function("parse_1mb", |b| {
        b.iter(|| {
            let count = jsonquotes_range_iter(corpus_bytes).count();
            black_box(count)
        });
    });

    // Also test with high escape density
    let high_escape_corpus: String = (0..5000)
        .map(|i| {
            format!(
                r#"{{"id": {}, "path": "C:\\Users\\test\\file{}.txt", "quote": "He said \"hello\" and \"goodbye\"", "data": "tab:\there\nnewline\nbackslash:\\"}}"#,
                i, i
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let high_escape_bytes = high_escape_corpus.as_bytes();

    group.throughput(Throughput::Bytes(high_escape_bytes.len() as u64));
    group.bench_function("parse_high_escapes", |b| {
        b.iter(|| {
            let count = jsonquotes_range_iter(high_escape_bytes).count();
            black_box(count)
        });
    });

    group.finish();
}

/// Benchmark: longest_match_at with varying FST sizes.
///
/// Tests the core FST lookup performance.
fn bench_longest_match_at(c: &mut Criterion) {
    let mut group = c.benchmark_group("longest_match_at");

    let mut rng = StdRng::from_seed(SEED);
    let all_keys = generate_flog_style_keys(&mut rng, 100_000);

    // Create a test string that contains some keys
    let test_text: String = all_keys
        .iter()
        .take(100)
        .map(|k| format!("prefix {} suffix", k))
        .collect::<Vec<_>>()
        .join(" ");
    let test_bytes = test_text.as_bytes();

    for &key_count in &[100, 1_000, 10_000, 100_000] {
        let keys: Vec<String> = all_keys.iter().take(key_count).cloned().collect();
        let ndjson = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        group.bench_with_input(
            BenchmarkId::new("keys", key_count),
            &(fstsed, test_bytes),
            |b, (fstsed, text)| {
                let mut cache = FstCache::new();
                b.iter(|| {
                    // Test at word boundaries where matches might occur
                    for pos in (0..text.len()).step_by(20) {
                        black_box(fstsed.longest_match_at(text, pos, &mut cache));
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: FstCache reuse vs recreation.
///
/// Demonstrates the benefit of reusing the cache across searches.
fn bench_cache_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_reuse");

    let mut rng = StdRng::from_seed(SEED);
    let keys = generate_flog_style_keys(&mut rng, 1000);
    let ndjson = generate_ndjson_dictionary(&keys);
    let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

    let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

    // Generate multiple small texts to simulate line-by-line processing
    let mut rng = StdRng::from_seed(SEED);
    let texts: Vec<String> = (0..100)
        .map(|_| {
            let sentence: String = Sentence(10..20).fake_with_rng(&mut rng);
            sentence
        })
        .collect();

    let total_bytes: usize = texts.iter().map(|t| t.len()).sum();
    group.throughput(Throughput::Bytes(total_bytes as u64));

    group.bench_function("with_reuse", |b| {
        let mut cache = FstCache::new();
        b.iter(|| {
            for text in &texts {
                let count = fstsed.find_iter(text.as_bytes(), &mut cache).count();
                black_box(count);
            }
        });
    });

    group.bench_function("without_reuse", |b| {
        b.iter(|| {
            for text in &texts {
                let mut cache = FstCache::new();
                let count = fstsed.find_iter(text.as_bytes(), &mut cache).count();
                black_box(count);
            }
        });
    });

    group.finish();
}

/// Benchmark: Template rendering performance.
///
/// Compares specialized templates (Standard, KeyOnly) vs generic Custom templates.
fn bench_template_rendering(c: &mut Criterion) {
    let mut group = c.benchmark_group("rendering");

    let standard_template = Template::parse("<{key}|{value}>");
    let key_only_template = Template::parse("{key}");
    let custom_template = Template::parse("prefix {key} middle {value} suffix");

    let match_standard = FstMatch::new(
        Cow::Borrowed("my-key"),
        Cow::Borrowed("my-value"),
        &standard_template,
        None,
    );
    let match_key_only = FstMatch::new(
        Cow::Borrowed("my-key"),
        Cow::Borrowed("my-value"),
        &key_only_template,
        None,
    );
    let match_custom = FstMatch::new(
        Cow::Borrowed("my-key"),
        Cow::Borrowed("my-value"),
        &custom_template,
        None,
    );

    group.bench_function("standard", |b| {
        b.iter(|| {
            let res = match_standard.display().to_string();
            black_box(res);
        });
    });

    group.bench_function("key_only", |b| {
        b.iter(|| {
            let res = match_key_only.display().to_string();
            black_box(res);
        });
    });

    group.bench_function("custom", |b| {
        b.iter(|| {
            let res = match_custom.display().to_string();
            black_box(res);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_fstsed_scaling,
    bench_fstsed_json_scaling,
    bench_jsonquotes_parsing,
    bench_longest_match_at,
    bench_cache_reuse,
    bench_template_rendering,
);

criterion_main!(benches);
