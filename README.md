# fstsed

Search **and enrich** at near-ripgrep speed

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org)

**fstsed** is a high-performance search-and-enrichment tool for cases where every match needs its own context. It's not a faster ripgrep -- instead, fstsed brings ripgrep-class performance to a different problem:

> Searching for many strings at once, where each string carries its own structured metadata.

fstsed is built on [BurntSushi's `fst` crate](https://github.com/BurntSushi/fst) and is inspired by the blog post _[Index 1,600,000,000 Keys with Automata and Rust](https://burntsushi.net/transducers/)_ and of course [ripgrep](https://github.com/BurntSushi/ripgrep) itself.

My other tool, [**geoipsed**](https://github.com/erichutchins/geoipsed), uses regexes to match only IP addresses and enrich with only the metadata from vendors MaxMind and Spur. In contrast, **fstsed** is completely flexible, allowing you to match any string and bundle any metadata you want.

---

## Table of Contents

- [Why Inline Enrichment Matters](#why-inline-enrichment-matters)
- [Use Cases](#use-cases)
- [Key Features](#key-features)
- [Quick Start](#quick-start)
- [Performance](#performance)
- [Examples](#examples)
- [When to Use fstsed](#when-to-use-fstsed)
- [Installation](#installation)
- [See Also](#see-also)
- [Acknowledgements](#acknowledgements)
- [License](#license)

---

## Why Inline Enrichment Matters

When investigating logs, reports, or documents, raw matches are never enough. You need context.

**Before**

```text
2024-01-10 14:23:45 Connection established to 192.168.1.105
2024-01-10 14:23:51 DNS query: flowersbyirene.com
2024-01-10 14:24:02 HTTP request to suspicious-domain.net
```

**After enrichment with fstsed**

```text
2024-01-10 14:23:45 Connection established to 192.168.1.105 (High-priority APT indicator, last seen: Jan 8)
2024-01-10 14:23:51 DNS query: flowersbyirene.com (Attributed to APT99, confidence: high)
2024-01-10 14:24:02 HTTP request to suspicious-domain.net (Phishing infrastructure)
```

Inline enrichment eliminates context switching. You read enriched output directly, even when working with **tens or hundreds of thousands of indicators**.

---

## Use Cases

| Tool | Patterns | Enrichment | Scaling |
| :--- | :--- | :--- | :--- |
| `grep -f searchterms.txt` | Regex and fixed strings | No capability | Linear with keys |
| `rg -f searchterms.txt --replace "replacement"` | Regex and fixed strings | One replacement string for all searchterms (although that replacement string can include [named capture groups](https://github.com/BurntSushi/ripgrep/blob/master/GUIDE.md#replacements) from the regex search) | Search optimized |
| `fstsed -f searchterms.fst` | Fixed strings | Replacement text **per search term** | Constant time complexity |

fstsed trades a small amount of raw speed (vs pure ripgrep) to gain **rich, structured, per-indicator context**.

---

## Key Features

- **Per-indicator metadata**  
  Each search term has a full JSON object of metadata

- **Template-driven output**  
  Use `--template` to control which fields from the JSON metadata object appear in output

- **Predictable performance at scale**  
  Search time remains stable from 10 to 100,000+ indicators

- **JSON-aware search mode**  
  Optionally search only inside JSON string values, with proper decoding and re-encoding

- **Word-boundary-aware matching**  
  Prevents partial matches (`apple` won't match `pineapples`)

- **Nested metadata access**  
  Reference deeply nested fields using JSON Pointer (RFC 6901)

- **Compact storage**  
  FST databases are Zstd-compressed for minimal disk usage

---

## Quick Start

Build once, reuse everywhere.

```bash
# Build an FST database from JSON threat intelligence.
# All the values from .indicator_key can be enriched with any part of the json record
# {"indicator_key": "indicator_value", "threat_actor": "APT99", "severity": "high", "alias": "Operation Red"}
fstsed --build -f intel.fst -k indicator_key threat_intel.json

# Enrich logs with selected fields
cat logs.txt | fstsed -f intel.fst \
  --template "{key} | {threat_actor} | severity: {severity}"

# Same database, different analysis context
fstsed -f intel.fst logs.txt \
  --template "{key} ({campaign})"

# JSON-only search mode (search within JSON string values)
fstsed -f intel.fst --json events.json \
  --template "{key} | remediate: {remediation_steps}"
```

Templates are fstsed's core abstraction to adapt output without rebuilding your database.

---

<details>
<summary><strong>Usage Reference</strong></summary>

```text
Find and replace/decorate text at scale using finite state transducers (fst)

Usage: fstsed [OPTIONS] -f <FST> [PATH]...

Arguments:
  [PATH]...
          Input file(s) or directory(s) to process. Leave empty or use "-" to read from stdin. (In build mode, only the first path is used)

Options:
  -o, --only-matching
          Show only nonempty parts of lines that match

      --color [<WHEN>]
          This flag controls when to use colors to highlight matched (non-empty) strings and the rendered template.

          fstsed will suppress color output by default in some other
          circumstances as well. These include, but are not limited to:

          • When the NO_COLOR environment variable is set (regardless of value).

          Possible values:
          - always: Always use color highlighting
          - never:  Never use color highlighting
          - auto:   Use color highlighting only when writing to a terminal (default)

          [default: auto]

  -f <FST>
          Specify fst db to use in search or to create in build mode

      --build
          Build mode. Build a fst from json data instead of querying one. Specify output path with the -f --fst
          parameter. Only first file input parameter or stdin is used to make the fst

  -k, --key <KEY>
          When building a fst, extract the given json field to use as the key in the fst database. Key may also be
          provided as a jsonpointer, e.g. /obj/array/1/item

          [default: key]

      --sorted
          When building a fst, set this if the keys of input json are already lexicographically sorted. This will
          make build construction much faster. If this is set but the keys are not sorted, the fst creation will
          error

  -t, --template <TEMPLATE>
          Specify the format of the fstsed match decoration. Field names are enclosed in {}, for example "{field1}
          any fixed string {field2} & {field3}". Fields may be json keys or jsonpointers {/obj/array/1/item}

  -j, --json
          Json search mode. Fstsed will treat input as json, searching only inside quoted strings. All strings are
          deserialized/decoded before json before searching, and all template decorations are properly json-encoded
          in the output for subsequent processing

  -w, --threads <NUM>
          The number of threads to use for searching

  -u, --no-ignore
          Do not respect ignore files (.gitignore, .ignore, etc.)

      --hidden
          For recursive directory scanning, search hidden files and directories

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Examples:
  Build an FST database from JSON:
    fstsed --build -f data.fst -k key data.json

  Basic find and replace:
    echo "match" | fstsed -f data.fst

  Use a template for decoration:
    echo "match" | fstsed -f data.fst --template "{key} ({info})"

  JSON search mode (only search inside quoted JSON strings):
    fstsed -f data.fst --json input.json

```

</details>

<details>
<summary><strong>Template Syntax</strong></summary>

Templates control how matches are rendered in output.

### Variables

| Syntax             | Description                              |
| ------------------ | ---------------------------------------- |
| `{key}`            | The matched search term                  |
| `{value}`          | Full JSON payload (entire record)        |
| `{fieldname}`      | Top-level JSON field value               |
| `{/path/to/field}` | Nested field via JSON Pointer (RFC 6901) |

### Examples

```bash
# Simple replacement
--template "{key}"

# Key with single field
--template "{key} ({threat_actor})"

# Multiple fields with formatting
--template "[{severity}] {key} - {description}"

# Nested JSON access
--template "{key}: {/metadata/attribution/actor}"

# Include literal braces by doubling them
--template "{{literal braces}} {key}"
```

### Default Template

When no template is specified, fstsed uses:

```text
<{key}|{value}>
```

This outputs the matched key and its full JSON payload, useful for debugging or when you need all metadata.

</details>

---

## Performance

fstsed's defining characteristic is **predictable, ripgrep-scale performance as key counts grow**.

Benchmarks below were run against a 100MB realistic log corpus (~1% positive match rate) on a modern multicore system.

| Tool       |    Keys | Time (s) | Throughput (MB/s) |
| ---------- | ------: | -------: | ----------------: |
| fstsed     |      10 |     0.41 |               245 |
| fstsed     | 100,000 |     0.92 |               110 |
| rg -F -w   |      10 |     0.08 |              1314 |
| rg -F -w   | 100,000 |     0.98 |               103 |
| grep -F -w |     100 |      102 |                ~1 |

### Takeaways

- **ripgrep** remains the fastest general-purpose search tool, especially for small pattern sets
- **fstsed** converges with ripgrep throughput at large key counts
- **grep** becomes impractical beyond ~100 patterns
- fstsed shines when you need **custom per-match transformations** that ripgrep can't provide

---

## Examples

### IOC Enrichment with Real Threat Intel (Volexity)

This example demonstrates building a real-world IOC database and enriching text with source-aware metadata.

**1. Get the data**

```bash
git clone https://github.com/volexity/threat-intel.git
```

```bash
uvx --with pandas ipython
```

**2. Convert CSV files to JSON**

```python
import pandas as pd
import glob

csvs = glob.glob("threat-intel/**/*.csv", recursive=True)

def conv(csv):
    df = pd.read_csv(csv)
    df["path"] = csv
    return df.to_json(orient="records", lines=True)

iocjson = "\n".join(map(conv, csvs))
with open("volexity.json", "w") as f:
    f.write(iocjson)
```

**3. Build FST database**

```bash
fstsed --build -f volexity.fst -k value volexity.json

# or pipe from stdin
cat volexity.json | fstsed --build -f volexity.fst -k value
```

**4. Enrich and analyze**

Basic search (default template shows full JSON record):

```bash
$ echo "test of avsvmcloud.com metadata" | fstsed -f volexity.fst

test of <avsvmcloud.com|{"value":"avsvmcloud.com","type":"hostname","notes":null,"path":"2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv"}> metadata
```

Custom template for cleaner output:

```bash
$ echo "test of avsvmcloud.com metadata" | fstsed -f volexity.fst \
    --template "{key} (a {type} from {path} report)"

test of avsvmcloud.com (a hostname from 2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv report) metadata
```

<details>
<summary><strong>More Examples</strong></summary>

### MITRE ATT&CK Technique Tagging

Enrich code or logs with ATT&CK technique context:

```bash
# Build from ATT&CK data
cat attack_patterns.json | fstsed --build -f attack.fst -k pattern

# Tag findings with technique IDs and tactics
fstsed -f attack.fst suspicious_script.ps1 \
  --template "{key} [ATT&CK: {technique_id} - {tactic}]"
```

### Translation and Highlighting

fstsed is not limited to infosec use cases. Here's a translation highlighting example:

```bash
# Build translation database
echo '{"key":"ဗိုလ်ချုပ်မှူးကြီး","translated":"Senior General of Myanmar Army"}' \
  | fstsed -f myanmar.fst --build -k key

# Highlight and translate in foreign text
fstsed -f myanmar.fst article.txt --template "<{key}> ({translated})"
```

Then, taking the lede from a [BBC article](https://www.bbc.com/burmese/burma-57432310) as a test case, the output shows matched terms with inline translations:

```bash
fstsed -f myanmar.fst bbc.txt --template "<{key}> ({translated})"
```

> လွန်ခဲ့တဲ့ ၅ နှစ်က တပ်မတော်ကာကွယ်ရေးဦးစီးချုပ် ရဲ့သက်တမ်းဟာ အကန့်အသတ်မရှိတဲ့ သဘောဖြစ်နေလို့ ၆၅ နှစ်ကန့်သတ်ပြီးပြင်ခဲ့တယ်လို့ **<ဗိုလ်ချုပ်မှူးကြီး> (Senior General of Myanmar Army)** မင်းအောင်လှိုင်က ပြောခဲ့ပြီး သူ့အသက် ၆၅ နှစ်ပြည့်ဖို့ လပိုင်းအလိုမှာ အာဏာသိမ်းကာ အဲ့ဒီ့ကန့်သတ်ချက်ကို ပယ်ဖျက်လိုက်တဲ့ အတွက် တပ်မတော်ကာကွယ်ရေးဦးစီးချုပ်သက်တမ်းဟာ အကန့်အသတ်မဲ့ ပြန်ဖြစ်သွားပါတယ်။

Even if I can't read any of the Burmese, I still know which key phrase matched, what that phrase means in my native tongue, and where generally the match occurred in the document.

</details>

---

## When to Use fstsed

**Use fstsed when:**

- You have **many search terms** (100s to 100,000s)
- Each term has **associated metadata** (threat intel, translations, annotations)
- Inline context matters more than raw matching speed
- You want to **reuse the same database** with different output templates

**Use ripgrep when:**

- You need the fastest possible literal search
- You have a handful of patterns
- You don't need per-match metadata
- You need regex or capture group substitution

---

<details>
<summary><strong>Limitations</strong></summary>

### Current Constraints

> [!NOTE]
> fstsed is optimized for literal string matching with rich metadata. The following constraints are by design.

- **Word boundaries only**  
  Matches must start and end at word boundaries. `apple` won't match inside `pineapple` or even in `apples`.

- **Literal strings only**  
  Patterns are exact strings, not regular expressions. Use ripgrep for regex needs.

- **Immutable databases**  
  FST databases cannot be updated incrementally. Any changes require a full rebuild.

</details>

<details>
<summary><strong>Matching Semantics & Build Notes</strong></summary>

### Boundary Matching

> [!IMPORTANT]
> Keys beginning or ending with word characters (`[a-zA-Z0-9_]`) must be bounded by non-word characters in input text.

| Key           | Input            | Matches? |
| ------------- | ---------------- | -------- |
| `apple`       | `an apple`       | ✅ Yes   |
| `apple`       | `an apple.`      | ✅ Yes   |
| `apple`       | `an apple,`      | ✅ Yes   |
| `apple`       | `pineapple`      | ❌ No    |
| `apple`       | `apples`         | ❌ No    |
| `192.168.1.1` | `ip:192.168.1.1` | ✅ Yes   |
| `192.168.1.1` | `192.168.1.15`   | ❌ No    |
| `10.10.1.1`   | `110.10.1.1`     | ❌ No    |

### Shadowed Keys (Prefix Handling)

When one key is a prefix of another (e.g., `abc` vs `abcde`), the **longer key takes precedence** when both could match.

| Input            | Match   |
| ---------------- | ------- |
| `hello abc test` | `abc`   |
| `foo abcde test` | `abcde` |

### Build Process

1. Input JSON records are parsed
2. The specified key field (`-k`) becomes the search term
3. The entire JSON record becomes the metadata payload
4. Records are sorted and compiled into an FST
5. The FST is Zstd-compressed and written to disk

### Database Inspection

```bash
# Optional: install fst-bin for database inspection
cargo install fst-bin

# Dump all keys in an FST
fst range your.fst

# Check if a specific key exists
fst grep your.fst "exact-key"
```

</details>

---

## Installation

<!-- ### Using Cargo

```bash
cargo install fstsed
``` -->

### From Source

```bash
git clone https://github.com/erichutchins/fstsed.git
cd fstsed
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

---

## See Also

- [ripgrep](https://github.com/BurntSushi/ripgrep) — Fast line-oriented search tool
- [fst](https://github.com/BurntSushi/fst) — Finite state transducers in Rust
- [aho-corasick](https://github.com/BurntSushi/aho-corasick) — Multi-pattern string matching
- [geoipsed](https://github.com/erichutchins/geoipsed) — IP geolocation enrichment tool

---

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. For major changes, please open an issue first to discuss what you would like to change.

---

## Acknowledgements

fstsed is built on BurntSushi's excellent `fst` crate and is inspired by the performance principles behind ripgrep. This project would not exist without that foundational work.

---

## License

MIT OR Apache-2.0
