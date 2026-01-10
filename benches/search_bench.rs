//! Criterion benchmarks for fstsed search operations.
//!
//! Uses `fake-rs` to generate realistic plain text and NDJSON corpora.

use std::io::Cursor;

use camino::Utf8PathBuf;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use fake::faker::company::en::{Bs, BsAdj, BsNoun, BsVerb, Buzzword, CatchPhrase, CompanyName};
use fake::faker::internet::en::{DomainSuffix, IPv4, Username};
use fake::faker::lorem::en::{Paragraph, Sentence, Word, Words};
use fake::faker::name::en::{FirstName, LastName, Name};
use fake::rand::rngs::StdRng;
use fake::rand::SeedableRng;
use fake::Fake;
use termcolor::ColorChoice;

use fstsed::build::build_fstsed;
use fstsed::fstsed::{FstCache, FstSed};
use fstsed::jsonquotes::jsonquotes_range_iter;

/// Fixed seed for reproducible benchmarks.
const SEED: [u8; 32] = [
    42, 0, 0, 0, 23, 0, 0, 0, 200, 1, 0, 0, 210, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0,
];

/// Generates a list of fake "entity" keys for the FST dictionary.
fn generate_dictionary_keys(rng: &mut StdRng, count: usize) -> Vec<String> {
    let mut keys = Vec::with_capacity(count);

    for i in 0..count {
        // Mix different types of keys for variety
        let key: String = match i % 6 {
            0 => Name().fake_with_rng(rng),
            1 => CompanyName().fake_with_rng(rng),
            2 => Username().fake_with_rng(rng),
            3 => Buzzword().fake_with_rng(rng),
            4 => format!(
                "{} {}",
                FirstName().fake_with_rng::<String, _>(rng),
                LastName().fake_with_rng::<String, _>(rng)
            ),
            _ => CatchPhrase().fake_with_rng(rng),
        };
        keys.push(key);
    }

    // Deduplicate and sort for FST building
    keys.sort();
    keys.dedup();
    keys
}

/// Generates NDJSON input for FST building from dictionary keys.
fn generate_ndjson_dictionary(keys: &[String]) -> String {
    keys.iter()
        .map(|key| {
            format!(
                r#"{{"key": "{}", "category": "entity", "score": {}}}"#,
                key.replace('"', "\\\""),
                keys.len() % 100
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generates plain text corpus with some dictionary keys embedded.
fn generate_plain_text_corpus(rng: &mut StdRng, keys: &[String], paragraphs: usize) -> String {
    let mut text = String::new();

    for i in 0..paragraphs {
        // Add a paragraph of lorem ipsum
        let para: String = Paragraph(3..6).fake_with_rng(rng);
        text.push_str(&para);
        text.push(' ');

        // Sprinkle in some dictionary keys every few paragraphs
        if i % 3 == 0 && !keys.is_empty() {
            let key_idx = i % keys.len();
            text.push_str(&keys[key_idx]);
            text.push_str(". ");
        }

        // Add some variety with sentences
        let sentence: String = Sentence(5..12).fake_with_rng(rng);
        text.push_str(&sentence);
        text.push('\n');
    }

    text
}

/// Generates NDJSON corpus with dictionary keys embedded in string values.
fn generate_ndjson_corpus(rng: &mut StdRng, keys: &[String], lines: usize) -> String {
    let mut ndjson = String::new();

    for i in 0..lines {
        let words: Vec<String> = Words(3..8).fake_with_rng(rng);
        let sentence: String = Sentence(4..10).fake_with_rng(rng);

        // Embed a dictionary key in some lines
        let description = if i % 5 == 0 && !keys.is_empty() {
            let key_idx = i % keys.len();
            format!("{} {} {}", sentence, keys[key_idx], words.join(" "))
        } else {
            format!("{} {}", sentence, words.join(" "))
        };

        let line = format!(
            r#"{{"id": {}, "name": "{}", "description": "{}", "tags": ["{}", "{}", "{}"], "active": {}}}"#,
            i,
            Username().fake_with_rng::<String, _>(rng).replace('"', ""),
            description.replace('"', "\\\""),
            Word().fake_with_rng::<String, _>(rng),
            Word().fake_with_rng::<String, _>(rng),
            Word().fake_with_rng::<String, _>(rng),
            i % 2 == 0
        );
        ndjson.push_str(&line);
        ndjson.push('\n');
    }

    ndjson
}

/// Generates complex NDJSON with nested structures for JSON parsing benchmarks.
fn generate_complex_ndjson(rng: &mut StdRng, lines: usize) -> String {
    let mut ndjson = String::new();

    for i in 0..lines {
        let name: String = Name().fake_with_rng(rng);
        let company: String = CompanyName().fake_with_rng(rng);
        let bs: String = Bs().fake_with_rng(rng);
        let verb: String = BsVerb().fake_with_rng(rng);
        let adj: String = BsAdj().fake_with_rng(rng);
        let noun: String = BsNoun().fake_with_rng(rng);
        let domain: String = DomainSuffix().fake_with_rng(rng);
        let ip: String = IPv4().fake_with_rng(rng);

        let line = format!(
            r#"{{"id": {}, "user": {{"name": "{}", "email": "{}@{}", "company": "{}"}}, "metadata": {{"ip": "{}", "action": "{}", "details": "{} {} {}"}}, "nested": {{"level1": {{"level2": {{"value": "deep string here"}}}}}}}}"#,
            i,
            name.replace('"', ""),
            Username().fake_with_rng::<String, _>(rng),
            domain,
            company.replace('"', ""),
            ip,
            verb,
            adj,
            noun,
            bs.replace('"', "")
        );
        ndjson.push_str(&line);
        ndjson.push('\n');
    }

    ndjson
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

fn bench_longest_match_at(c: &mut Criterion) {
    let mut group = c.benchmark_group("longest_match_at");

    // Test with different dictionary sizes
    for key_count in [100, 1000, 10_000] {
        let mut rng = StdRng::from_seed(SEED);
        let keys = generate_dictionary_keys(&mut rng, key_count);
        let ndjson = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        // Generate test text with embedded keys
        let mut rng = StdRng::from_seed(SEED);
        let text = generate_plain_text_corpus(&mut rng, &keys, 50);
        let text_bytes = text.as_bytes();

        group.throughput(Throughput::Bytes(text_bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("keys", key_count),
            &(fstsed, text_bytes),
            |b, (fstsed, text)| {
                let mut cache = FstCache::new();
                b.iter(|| {
                    // Test longest_match_at at various positions
                    for pos in (0..text.len()).step_by(100) {
                        black_box(fstsed.longest_match_at(text, pos, &mut cache));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_find_iter(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_iter");

    // Test with different corpus sizes
    for (key_count, para_count) in [(500, 100), (1000, 500), (5000, 1000)] {
        let mut rng = StdRng::from_seed(SEED);
        let keys = generate_dictionary_keys(&mut rng, key_count);
        let ndjson = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        // Generate larger corpus for throughput testing
        let mut rng = StdRng::from_seed(SEED);
        let text = generate_plain_text_corpus(&mut rng, &keys, para_count);
        let text_bytes = text.as_bytes();

        group.throughput(Throughput::Bytes(text_bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("corpus_size", text_bytes.len()),
            &(fstsed, text_bytes),
            |b, (fstsed, text)| {
                let mut cache = FstCache::new();
                b.iter(|| {
                    let count = fstsed.find_iter(text, &mut cache).count();
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

fn bench_find_iter_ndjson_corpus(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_iter_ndjson");

    for line_count in [100, 500, 1000] {
        let mut rng = StdRng::from_seed(SEED);
        let keys = generate_dictionary_keys(&mut rng, 1000);
        let ndjson_dict = generate_ndjson_dictionary(&keys);
        let (_temp_dir, fst_path) = create_temp_fst(&ndjson_dict);

        let fstsed = FstSed::new(&fst_path, None, ColorChoice::Never).expect("Failed to load FST");

        // Generate NDJSON corpus to search
        let mut rng = StdRng::from_seed(SEED);
        let corpus = generate_ndjson_corpus(&mut rng, &keys, line_count);
        let corpus_bytes = corpus.as_bytes();

        group.throughput(Throughput::Bytes(corpus_bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("lines", line_count),
            &(fstsed, corpus_bytes),
            |b, (fstsed, text)| {
                let mut cache = FstCache::new();
                b.iter(|| {
                    let count = fstsed.find_iter(text, &mut cache).count();
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

fn bench_jsonquotes_range_iter(c: &mut Criterion) {
    let mut group = c.benchmark_group("jsonquotes_range_iter");

    // Test with different NDJSON complexities and sizes
    for line_count in [100, 500, 1000, 5000] {
        let mut rng = StdRng::from_seed(SEED);
        let ndjson = generate_complex_ndjson(&mut rng, line_count);
        let ndjson_bytes = ndjson.as_bytes();

        group.throughput(Throughput::Bytes(ndjson_bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("lines", line_count),
            ndjson_bytes,
            |b, text| {
                b.iter(|| {
                    let count = jsonquotes_range_iter(text).count();
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

fn bench_jsonquotes_with_escapes(c: &mut Criterion) {
    let mut group = c.benchmark_group("jsonquotes_escapes");

    // Generate JSON with varying amounts of escaped characters
    let mut rng = StdRng::from_seed(SEED);

    // Low escape density
    let low_escape = generate_complex_ndjson(&mut rng, 500);

    // High escape density - manually add more escapes
    let high_escape: String = (0..500)
        .map(|i| {
            format!(
                r#"{{"id": {}, "path": "C:\\Users\\test\\file{}.txt", "quote": "He said \"hello\" and \"goodbye\"", "mixed": "tab:\there\nnewline\nbackslash:\\"}}"#,
                i, i
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let low_bytes = low_escape.as_bytes();
    let high_bytes = high_escape.as_bytes();

    group.throughput(Throughput::Bytes(low_bytes.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("escape_density", "low"),
        low_bytes,
        |b, text| {
            b.iter(|| {
                let count = jsonquotes_range_iter(text).count();
                black_box(count)
            });
        },
    );

    group.throughput(Throughput::Bytes(high_bytes.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("escape_density", "high"),
        high_bytes,
        |b, text| {
            b.iter(|| {
                let count = jsonquotes_range_iter(text).count();
                black_box(count)
            });
        },
    );

    group.finish();
}

fn bench_cache_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_reuse");

    let mut rng = StdRng::from_seed(SEED);
    let keys = generate_dictionary_keys(&mut rng, 1000);
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

    // Benchmark with cache reuse (recommended pattern)
    group.bench_function("with_reuse", |b| {
        let mut cache = FstCache::new();
        b.iter(|| {
            for text in &texts {
                let count = fstsed.find_iter(text.as_bytes(), &mut cache).count();
                black_box(count);
            }
        });
    });

    // Benchmark without cache reuse (creating new cache each time)
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

criterion_group!(
    benches,
    bench_longest_match_at,
    bench_find_iter,
    bench_find_iter_ndjson_corpus,
    bench_jsonquotes_range_iter,
    bench_jsonquotes_with_escapes,
    bench_cache_reuse,
);

criterion_main!(benches);
